//! Tauri command surface for the session IPC bridge.
//!
//! Every command is a thin wrapper over [`crate::bridge::SessionBridge`],
//! plus an [`EventSink`] implementation that forwards payloads to the
//! owning session's webview via `AppHandle::emit_to(EventTarget::webview_window(
//! "session-{session_id}"), "session:event", …)`.
//!
//! **Authorization (F-051 / H10):** each session command requires the
//! calling webview's label to equal `format!("session-{session_id}")`.
//! Window labels are set by `window_manager` at window creation and cannot
//! be forged from webview JS, so they serve as the per-window authenticator
//! binding a session's control channel to its review channel. Mismatches
//! return a label-mismatch error and never reach the daemon.
//!
//! **Webview isolation (F-062 / M10 / T5):** the event sink targets a single
//! webview (`session-{session_id}`) instead of broadcasting app-wide. Prior
//! to this fix, every session window (and the dashboard) received every
//! session's events; the trust boundary was enforced client-side in the
//! Solid store. The per-sink `session_id` is bound at construction in
//! `session_subscribe` (already label-authenticated), not re-read from the
//! event payload.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use forge_core::approvals::{
    load_user_config_in, load_workspace_config, save_user_config_in, save_workspace_config,
    ApprovalConfig, ApprovalEntry,
};
use forge_core::{ApprovalLevel, ApprovalScope, RerunVariant, TerminalId};
use forge_ipc::HelloAck;
use forge_lsp::{
    Bootstrap as LspBootstrap, MessageTransport, Server as LspServer, ServerEvent as LspServerEvent,
};
use forge_term::{ShellSpec, TerminalEvent, TerminalSession, TerminalSize};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime, State, Webview};
use ts_rs::TS;

use crate::bridge::{EventSink, SessionBridge, SessionConnections, SessionEventPayload};

/// F-051 / H10: structured error returned when the calling webview's label
/// does not match the expected owner for a command's scope. Kept as a plain
/// `String` so it matches every `#[tauri::command]`'s existing `Err(String)`
/// wire shape — never a panic.
pub(crate) const LABEL_MISMATCH_ERROR: &str = "forbidden: window label mismatch";

/// F-068 / L4 (T7): per-field byte caps on untyped-string inputs to session
/// commands. `forge_ipc::write_frame` rejects frames above 4 MiB, but a
/// compromised webview can still loop 4 MiB sends — each causes transient
/// Rust-side allocation and, for `text`, billable model calls. These caps
/// stop the allocation before serialization.
///
/// All caps are byte counts (`.len()` on `String`), not char counts — the
/// resource being bounded is memory/wire cost.
///
/// F-069 / L5 (T7) superseded the byte cap on `session_approve_tool`'s `scope`
/// with a typed-enum (`forge_core::ApprovalScope`) whose variants are all
/// short; any oversize or non-variant input is rejected by serde at the Tauri
/// arg-deserialization layer — earlier than this check. No `MAX_SCOPE_BYTES`
/// constant is defined for that reason.
pub(crate) const MAX_MESSAGE_TEXT_BYTES: usize = 128 * 1024;
pub(crate) const MAX_TOOL_CALL_ID_BYTES: usize = 64;
pub(crate) const MAX_REJECT_REASON_BYTES: usize = 1024;
/// F-143: cap on the `msg_id` string accepted by `rerun_message`. `MessageId`
/// hex is 16 chars; 64 bytes leaves room for the wrapper/URL-safe variants
/// without permitting unbounded growth if a compromised webview lies.
pub(crate) const MAX_MESSAGE_ID_BYTES: usize = 64;

/// F-036 / F-068 (L4 / T7): caps on untyped-string inputs to the persistent
/// approval commands. `workspace_root` is an absolute filesystem path — 4096
/// bytes covers PATH_MAX on every target platform (Linux 4096, macOS 1024,
/// Windows 32 767 WTF-16 → bounded under 64 KiB). `scope_key` mirrors the
/// deterministic keys produced by the frontend (`file:<tool>:<path>`,
/// `pattern:<tool>:<glob>`, `tool:<name>`) so 4 KiB is well above any realistic
/// value. `tool_name` and `label` stay small; we cap them together with a
/// per-entry cap of 8 KiB to block oversized pseudo-entries before the bridge
/// allocates TOML output.
pub(crate) const MAX_WORKSPACE_ROOT_BYTES: usize = 4096;
pub(crate) const MAX_SCOPE_KEY_BYTES: usize = 4096;
pub(crate) const MAX_APPROVAL_ENTRY_BYTES: usize = 8 * 1024;

/// F-125: caps on untyped-string / byte-vec inputs to the terminal commands.
/// `cwd` is an absolute fs path — same 4 KiB PATH_MAX envelope as the approval
/// commands' `workspace_root`. `shell` (optional program override) is bounded
/// generously since some exotic shells live under deep paths. `data` is the
/// per-call input write cap — most terminal input frames are well under this,
/// but pastes can be large; 64 KiB matches common terminal emulator paste
/// chunks without letting a compromised webview loop PTY writes at full wire
/// speed.
pub(crate) const MAX_TERMINAL_CWD_BYTES: usize = 4096;
pub(crate) const MAX_TERMINAL_SHELL_BYTES: usize = 4096;
pub(crate) const MAX_TERMINAL_WRITE_BYTES: usize = 64 * 1024;

/// F-068 / L4 (T7): error returned when a session command's untyped-string
/// input exceeds its byte cap. Tests assert against the literal fragments
/// `"payload too large"` + the field name — keep both when evolving the
/// message so existing tests and any UI handling stay stable.
fn payload_too_large(field: &str, limit_bytes: usize) -> String {
    format!("payload too large: {field} exceeds {limit_bytes}-byte limit")
}

/// Assert the calling webview's label equals `expected`. Used at the top of
/// every session/dashboard `#[tauri::command]` to reject cross-window invokes
/// before the bridge sees the frame.
pub(crate) fn require_window_label<R: Runtime>(
    webview: &Webview<R>,
    expected: &str,
) -> Result<(), String> {
    if webview.label() == expected {
        Ok(())
    } else {
        Err(LABEL_MISMATCH_ERROR.to_string())
    }
}

/// F-068 / L4 (T7): reject payloads whose byte length exceeds `limit_bytes`.
/// Runs after `require_window_label` (so unauthorized windows don't learn
/// about cap values) and before any bridge call (so the allocation/wire cost
/// never materializes). Returns `Err` with a stable marker that tests and
/// any UI-side handling can pattern-match on.
pub(crate) fn require_size(field: &str, value: &str, limit_bytes: usize) -> Result<(), String> {
    if value.len() <= limit_bytes {
        Ok(())
    } else {
        Err(payload_too_large(field, limit_bytes))
    }
}

/// Tauri-managed bridge state. One per App; commands resolve it via
/// `State<BridgeState>`.
///
/// **F-052 (H11 / T7):** the production `session_hello` command never
/// accepts a webview-supplied socket path; the path is always derived via
/// [`crate::bridge::default_socket_path`]. Integration tests (which run
/// against ephemeral tempdir sockets) wire an override through the
/// `webview-test`-gated `test_socket_override` field. The field is
/// absent from production builds entirely.
pub struct BridgeState {
    pub bridge: SessionBridge,
    #[cfg(feature = "webview-test")]
    pub test_socket_override: Option<std::path::PathBuf>,
    /// F-036 test seam: redirect the user-scope approvals file to this
    /// directory instead of the platform config dir. Mirrors
    /// `test_socket_override`'s pattern — absent from production builds.
    #[cfg(feature = "webview-test")]
    pub test_user_config_dir_override: Option<std::path::PathBuf>,
}

impl BridgeState {
    pub fn new(connections: SessionConnections) -> Self {
        Self {
            bridge: SessionBridge::new(connections),
            #[cfg(feature = "webview-test")]
            test_socket_override: None,
            #[cfg(feature = "webview-test")]
            test_user_config_dir_override: None,
        }
    }

    /// Test-only constructor: wires a fixed socket path that `session_hello`
    /// will use instead of [`crate::bridge::default_socket_path`]. Gated
    /// behind the `webview-test` feature so production builds cannot
    /// construct a `BridgeState` that bypasses the default path.
    #[cfg(feature = "webview-test")]
    pub fn with_test_socket_override(
        connections: SessionConnections,
        socket_path: std::path::PathBuf,
    ) -> Self {
        Self {
            bridge: SessionBridge::new(connections),
            test_socket_override: Some(socket_path),
            test_user_config_dir_override: None,
        }
    }

    /// F-036 test-only constructor: override the user-scope config dir so tests
    /// can point at a tempdir instead of the real `{config_dir}/forge/`.
    #[cfg(feature = "webview-test")]
    pub fn with_test_user_config_dir(
        connections: SessionConnections,
        user_config_dir: std::path::PathBuf,
    ) -> Self {
        Self {
            bridge: SessionBridge::new(connections),
            test_socket_override: None,
            test_user_config_dir_override: Some(user_config_dir),
        }
    }
}

/// Event sink that forwards session events to the owning session's webview
/// under the `session:event` event name.
///
/// **F-062 (M10 / T5):** `session_id` is bound at construction from the
/// authenticated `session_subscribe` argument (already gated by
/// [`require_window_label`]). It is *not* re-read from the payload, so a
/// forged payload field cannot redirect delivery to another window.
pub(crate) struct AppHandleSink<R: Runtime> {
    pub(crate) app: AppHandle<R>,
    pub(crate) session_id: String,
}

impl<R: Runtime> EventSink for AppHandleSink<R> {
    fn emit(&self, payload: SessionEventPayload) {
        // F-062 (M10 / T5): target the session's own webview window instead
        // of broadcasting app-wide. Prior to this, every `session-*` window
        // (and the dashboard) received every session's events; filtering
        // happened client-side in the Solid store — exactly the wrong place
        // for a trust boundary. Target label uses `self.session_id` (bound
        // at construction from the authenticated `session_subscribe`
        // argument), not a payload field, so a forged payload cannot
        // redirect delivery.
        let target = EventTarget::webview_window(format!("session-{}", self.session_id));
        if let Err(e) = self.app.emit_to(target, "session:event", payload) {
            eprintln!("session:event emit failed: {e}");
        }
    }
}

/// Test-only constructor for the per-session app-handle event sink. Gated
/// behind the `webview-test` feature so production builds cannot reach into
/// the sink.
#[cfg(feature = "webview-test")]
pub fn make_app_handle_sink<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
) -> std::sync::Arc<dyn EventSink> {
    std::sync::Arc::new(AppHandleSink { app, session_id })
}

#[tauri::command]
pub async fn session_hello<R: Runtime>(
    session_id: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<HelloAck, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // F-052 (H11 / T7): the socket path is never taken from the invoke
    // payload — a webview cannot redirect this connection to an arbitrary
    // UDS. Production always resolves through `default_socket_path`; tests
    // inject a tempdir path via the `webview-test` override field.
    #[cfg(feature = "webview-test")]
    let override_path = state.test_socket_override.as_deref();
    #[cfg(not(feature = "webview-test"))]
    let override_path: Option<&std::path::Path> = None;
    state
        .bridge
        .hello(&session_id, override_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_subscribe<R: Runtime>(
    session_id: String,
    since: Option<u64>,
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    let sink: Arc<dyn EventSink> = Arc::new(AppHandleSink {
        app,
        session_id: session_id.clone(),
    });
    state
        .bridge
        .subscribe(&session_id, since.unwrap_or(0), sink)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_send_message<R: Runtime>(
    session_id: String,
    text: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // F-068 / L4 (T7): bound `text` before the bridge allocates a frame or
    // the provider is billed. Runs after authz so unauthorized windows
    // don't learn the cap value.
    require_size("text", &text, MAX_MESSAGE_TEXT_BYTES)?;
    state
        .bridge
        .send_message(&session_id, text)
        .await
        .map_err(|e| e.to_string())
}

/// F-391: cancel the in-flight turn for this session.
///
/// Fired by the Composer's Stop button and Esc handler. The authz check is
/// the primary contract today: only the session's own window may cancel it.
/// A server-side transport for the actual cancel is a separate follow-up
/// (no `IpcMessage::CancelTurn` exists yet); the frontend optimistically
/// clears its streaming lock so the composer becomes interactive regardless.
#[tauri::command]
pub async fn session_cancel<R: Runtime>(
    session_id: String,
    webview: Webview<R>,
    _state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    Ok(())
}

#[tauri::command]
pub async fn session_approve_tool<R: Runtime>(
    session_id: String,
    tool_call_id: String,
    scope: ApprovalScope,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // F-068 / L4 (T7): tool_call_id is a short opaque handle; bound it here.
    // F-069 / L5 (T7): `scope` is typed as `forge_core::ApprovalScope` — serde
    // rejects any non-variant string at Tauri arg-deserialization (before this
    // body runs), so no explicit scope validation is needed and no byte cap
    // is useful (the longest variant is 11 bytes).
    require_size("tool_call_id", &tool_call_id, MAX_TOOL_CALL_ID_BYTES)?;
    state
        .bridge
        .approve_tool(&session_id, tool_call_id, scope)
        .await
        .map_err(|e| e.to_string())
}

/// F-143 / F-144: re-run an assistant message. All three `RerunVariant`s
/// (`Replace`, `Branch`, `Fresh`) are dispatched through the bridge. The
/// `variant` parameter is typed so serde rejects any non-variant at the
/// Tauri arg-deserialization layer — no byte cap is useful here.
#[tauri::command]
pub async fn rerun_message<R: Runtime>(
    session_id: String,
    msg_id: String,
    variant: RerunVariant,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // F-068 / L4: bound `msg_id` before the bridge allocates a frame.
    require_size("msg_id", &msg_id, MAX_MESSAGE_ID_BYTES)?;
    state
        .bridge
        .rerun_message(&session_id, msg_id, variant)
        .await
        .map_err(|e| e.to_string())
}

/// F-144: activate a branch variant for replay / UI display. Resolves
/// `variant_index` against the session log and (on success) emits
/// `Event::BranchSelected { parent, selected }`. The emission arrives
/// through the session event stream; this command returns `Ok(())` once
/// the frame is written to the daemon.
///
/// Authz + size caps mirror `rerun_message` — only the owning session's
/// webview may drive its branch selection; `parent_id` is bounded by
/// `MAX_MESSAGE_ID_BYTES`.
#[tauri::command]
pub async fn select_branch<R: Runtime>(
    session_id: String,
    parent_id: String,
    variant_index: u32,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("parent_id", &parent_id, MAX_MESSAGE_ID_BYTES)?;
    state
        .bridge
        .select_branch(&session_id, parent_id, variant_index)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_reject_tool<R: Runtime>(
    session_id: String,
    tool_call_id: String,
    reason: Option<String>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // F-068 / L4 (T7): bound tool_call_id and — only when present — reason.
    // `None` reason is the common case and must skip the size check.
    require_size("tool_call_id", &tool_call_id, MAX_TOOL_CALL_ID_BYTES)?;
    if let Some(r) = reason.as_deref() {
        require_size("reason", r, MAX_REJECT_REASON_BYTES)?;
    }
    state
        .bridge
        .reject_tool(&session_id, tool_call_id, reason)
        .await
        .map_err(|e| e.to_string())
}

/// Returns a fully-wired invoke handler registering all session bridge
/// commands. Called from `window_manager::run` when building the Tauri app.
pub fn build_invoke_handler<R: Runtime>() -> Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>
{
    Box::new(tauri::generate_handler![
        session_hello,
        session_subscribe,
        session_send_message,
        // F-391: Composer Stop / Esc cancel target.
        session_cancel,
        session_approve_tool,
        session_reject_tool,
        rerun_message,
        select_branch,
        delete_branch,
        get_persistent_approvals,
        save_approval,
        remove_approval,
        terminal_spawn,
        terminal_write,
        terminal_resize,
        terminal_kill,
        read_layouts,
        write_layouts,
        read_file,
        write_file,
        tree,
        lsp_start,
        lsp_stop,
        lsp_send,
        // F-137: background-agent lifecycle.
        start_background_agent,
        promote_background_agent,
        list_background_agents,
        // F-138: stop command completes the start/promote/list/stop quartet.
        stop_background_agent,
        rename_path,
        delete_path,
        // F-151: persistent settings store.
        get_settings,
        set_setting,
        // F-132: MCP commands — list / toggle / import (+ state-stream forwarder).
        list_mcp_servers,
        toggle_mcp_server,
        import_mcp_config,
    ])
}

/// Attach the `BridgeState` to an app builder. Used by `window_manager::run`.
pub fn manage_bridge<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<BridgeState>().is_none() {
        app.manage(BridgeState::new(SessionConnections::new()));
    }
}

/// Attach the `TerminalState` to an app builder. Used by `window_manager::run`
/// and by integration tests. Idempotent — if state is already present it is a
/// no-op, so test helpers that call both `build_invoke_handler` (for command
/// registration) and this function cannot double-initialize.
pub fn manage_terminals<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<TerminalState>().is_none() {
        app.manage(TerminalState::default());
    }
}

/// Attach the `LspState` to an app builder. Idempotent — parallels
/// [`manage_terminals`] so tests can wire both and production `window_manager`
/// can call once.
///
/// Also attaches an [`LspBootstrapState`] holding the bundled-registry
/// [`forge_lsp::Bootstrap`] (F-353). `lsp_start` resolves a webview-supplied
/// server id against this bootstrap's registry, then binds the resolved
/// binary path to the cache-root sandbox — the webview never names a
/// filesystem path.
///
/// On a host where the platform cache dir cannot be resolved
/// (`BootstrapError::NoCacheDir`), no state is attached and `lsp_start`
/// returns a plain string error on every invoke. Tests that want a custom
/// registry (e.g. pointing at the in-tree `forge-lsp-mock-stdio` fixture)
/// override the managed state *after* this call via
/// [`LspBootstrapState::override_for_tests`].
pub fn manage_lsp<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<LspState>().is_none() {
        app.manage(LspState::default());
    }
    if app.try_state::<LspBootstrapState>().is_none() {
        app.manage(LspBootstrapState::new());
    }
}

/// Tauri-managed [`forge_lsp::Bootstrap`] singleton. Wraps an `Arc` so the
/// async `lsp_start` handler can clone a reference without holding a mutex
/// guard across the `Server::from_registry` path-resolution work.
///
/// Initialized to the platform default (`Bootstrap::new()`), or `None` if
/// the host lacks a cache dir. Integration tests override this via
/// [`LspBootstrapState::override_for_tests`] so the `lsp_start` call
/// resolves to an in-tree fixture instead of making network I/O inevitable
/// for a real registered server.
pub struct LspBootstrapState {
    inner: Mutex<Option<Arc<LspBootstrap>>>,
}

impl Default for LspBootstrapState {
    fn default() -> Self {
        Self::new()
    }
}

impl LspBootstrapState {
    /// Populate from [`LspBootstrap::new`]. Swallows a `NoCacheDir` error
    /// and leaves the slot empty; `lsp_start` surfaces that to the webview
    /// as a string error instead of panicking app startup.
    pub fn new() -> Self {
        let inner = LspBootstrap::new().ok().map(Arc::new);
        Self {
            inner: Mutex::new(inner),
        }
    }

    /// Snapshot the current bootstrap, if any. Returns a clone of the
    /// `Arc` so handlers can drop the mutex guard before awaiting.
    pub fn snapshot(&self) -> Option<Arc<LspBootstrap>> {
        self.inner
            .lock()
            .expect("lsp bootstrap state poisoned")
            .clone()
    }

    /// Replace the managed bootstrap. Integration tests use this to
    /// substitute a tempdir-rooted `Bootstrap` with a single-spec
    /// `Registry` pointing at the stub LSP fixture.
    #[doc(hidden)]
    pub fn override_for_tests(&self, bootstrap: Arc<LspBootstrap>) {
        *self.inner.lock().expect("lsp bootstrap state poisoned") = Some(bootstrap);
    }
}

// ---------------------------------------------------------------------------
// F-036: persistent approval commands
//
// Three Tauri commands surface the approvals config to the Solid store:
//
// - `get_persistent_approvals(workspace_root)` — loads both the user and
//   workspace files, tags each entry with its level, and returns a flat list
//   the store can seed its whitelist with. Workspace wins on `scope_key`
//   collision (mirrors `forge-mcp::config::load_merged`).
// - `save_approval(entry, level, workspace_root)` — upserts the entry into
//   the matching file; no-op for `Session` (the frontend should not route
//   session-level approvals through IPC, but we handle it defensively).
// - `remove_approval(scope_key, level, workspace_root)` — drops the matching
//   entry from the specified tier.
//
// Both writes go through `ApprovalConfig` + the atomic-write helper in
// `forge-core::approvals`, so partial writes cannot produce a corrupted TOML
// file. Neither command is authz-gated to a specific session window: approval
// config is a user-level artifact, not per-session. The `session-*` capability
// glob still bounds who can invoke it at all.
// ---------------------------------------------------------------------------

/// Wire shape returned by `get_persistent_approvals`. Frontend stores it in
/// the per-session whitelist record keyed by `scope_key`, so the
/// `ApprovalPrompt`'s auto-approve path and the `WhitelistedPill`'s
/// provenance label can both read the persistence tier without re-querying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct PersistentApprovalEntry {
    pub scope_key: String,
    pub tool_name: String,
    pub label: String,
    pub level: ApprovalLevel,
}

impl PersistentApprovalEntry {
    fn from_entry(entry: ApprovalEntry, level: ApprovalLevel) -> Self {
        Self {
            scope_key: entry.scope_key,
            tool_name: entry.tool_name,
            label: entry.label,
            level,
        }
    }
}

/// Resolve the user-scope config dir, honoring the `webview-test` override
/// when present. Returns `Ok(None)` when neither the override nor the
/// platform resolution yields a path (extremely unusual — no `$HOME`, no
/// Known Folder). Callers should treat `Ok(None)` as "empty user config."
fn resolve_user_config_dir(state: &BridgeState) -> Option<PathBuf> {
    #[cfg(feature = "webview-test")]
    {
        if let Some(override_dir) = state.test_user_config_dir_override.as_ref() {
            return Some(override_dir.clone());
        }
    }
    #[cfg(not(feature = "webview-test"))]
    let _ = state;
    dirs::config_dir()
}

#[tauri::command]
pub async fn get_persistent_approvals<R: Runtime>(
    workspace_root: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<Vec<PersistentApprovalEntry>, String> {
    // F-051: only authenticated session/dashboard windows may invoke. The
    // default capability glob allows `session-*` and `dashboard`; per-command
    // label authz would add nothing here (the data is user-scoped, not
    // per-session), so we accept any window that cleared the ACL.
    require_window_label_in(&webview, &["dashboard"], true)?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;

    let workspace_cfg = load_workspace_config(std::path::Path::new(&workspace_root))
        .await
        .map_err(|e| e.to_string())?;

    let user_cfg = match resolve_user_config_dir(&state) {
        Some(dir) => load_user_config_in(&dir).await.map_err(|e| e.to_string())?,
        None => ApprovalConfig::default(),
    };

    // Workspace wins on `scope_key` collision with user (mirrors
    // `forge-mcp::config::load_merged`). Build a set of workspace keys so
    // user entries that duplicate are suppressed.
    let workspace_keys: std::collections::HashSet<String> = workspace_cfg
        .entries
        .iter()
        .map(|e| e.scope_key.clone())
        .collect();

    let mut out: Vec<PersistentApprovalEntry> =
        Vec::with_capacity(workspace_cfg.entries.len() + user_cfg.entries.len());
    for entry in workspace_cfg.entries {
        out.push(PersistentApprovalEntry::from_entry(
            entry,
            ApprovalLevel::Workspace,
        ));
    }
    for entry in user_cfg.entries {
        if workspace_keys.contains(&entry.scope_key) {
            continue;
        }
        out.push(PersistentApprovalEntry::from_entry(
            entry,
            ApprovalLevel::User,
        ));
    }
    Ok(out)
}

#[tauri::command]
pub async fn save_approval<R: Runtime>(
    entry: ApprovalEntry,
    level: ApprovalLevel,
    workspace_root: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &["dashboard"], true)?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;
    require_size("scope_key", &entry.scope_key, MAX_SCOPE_KEY_BYTES)?;
    let total = entry.scope_key.len() + entry.tool_name.len() + entry.label.len();
    if total > MAX_APPROVAL_ENTRY_BYTES {
        return Err(payload_too_large("entry", MAX_APPROVAL_ENTRY_BYTES));
    }

    match level {
        ApprovalLevel::Session => {
            // Session-level approvals are purely in-memory on the frontend.
            // Accept the call defensively (so a misrouted invoke is not an
            // error the user sees) but never touch disk.
            Ok(())
        }
        ApprovalLevel::Workspace => {
            let root = std::path::Path::new(&workspace_root);
            let mut cfg = load_workspace_config(root)
                .await
                .map_err(|e| e.to_string())?;
            upsert_entry(&mut cfg, entry);
            save_workspace_config(root, &cfg)
                .await
                .map_err(|e| e.to_string())
        }
        ApprovalLevel::User => {
            let dir = resolve_user_config_dir(&state)
                .ok_or_else(|| "could not resolve user config directory".to_string())?;
            let mut cfg = load_user_config_in(&dir).await.map_err(|e| e.to_string())?;
            upsert_entry(&mut cfg, entry);
            save_user_config_in(&dir, &cfg)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

#[tauri::command]
pub async fn remove_approval<R: Runtime>(
    scope_key: String,
    level: ApprovalLevel,
    workspace_root: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &["dashboard"], true)?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;
    require_size("scope_key", &scope_key, MAX_SCOPE_KEY_BYTES)?;

    match level {
        ApprovalLevel::Session => Ok(()),
        ApprovalLevel::Workspace => {
            let root = std::path::Path::new(&workspace_root);
            let mut cfg = load_workspace_config(root)
                .await
                .map_err(|e| e.to_string())?;
            cfg.entries.retain(|e| e.scope_key != scope_key);
            save_workspace_config(root, &cfg)
                .await
                .map_err(|e| e.to_string())
        }
        ApprovalLevel::User => {
            let Some(dir) = resolve_user_config_dir(&state) else {
                // No config dir resolvable — there is nothing to remove because
                // the load path returns empty in the same condition. Treat as a
                // no-op rather than an error so revoke-after-cold-start paths
                // don't surface a spurious failure.
                return Ok(());
            };
            let mut cfg = load_user_config_in(&dir).await.map_err(|e| e.to_string())?;
            cfg.entries.retain(|e| e.scope_key != scope_key);
            save_user_config_in(&dir, &cfg)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

/// Upsert by `scope_key`: replace the existing entry if present, otherwise
/// append. Keeps insertion order stable for the common "add once" case.
fn upsert_entry(cfg: &mut ApprovalConfig, entry: ApprovalEntry) {
    if let Some(existing) = cfg
        .entries
        .iter_mut()
        .find(|e| e.scope_key == entry.scope_key)
    {
        *existing = entry;
    } else {
        cfg.entries.push(entry);
    }
}

/// Lightweight window-label gate that allows either the dashboard or any
/// session window (`session-*` prefix) to invoke a command. Used by the F-036
/// approval commands which are user-scoped, not per-session, but should still
/// be unreachable from other surfaces. Keeping this as a separate helper from
/// [`require_window_label`] preserves that helper's strict single-label
/// semantics for F-051.
pub(crate) fn require_window_label_in<R: Runtime>(
    webview: &Webview<R>,
    exact: &[&str],
    allow_session_prefix: bool,
) -> Result<(), String> {
    let label = webview.label();
    if exact.contains(&label) {
        return Ok(());
    }
    if allow_session_prefix && label.starts_with("session-") {
        return Ok(());
    }
    Err(LABEL_MISMATCH_ERROR.to_string())
}

// ---------------------------------------------------------------------------
// F-125: Terminal commands (spawn / write / resize / kill)
//
// Scope. Four Tauri commands let a session webview drive a PTY-backed shell
// via `forge-term::TerminalSession`. Each spawned session is owned by exactly
// one webview label (the label that called `terminal_spawn`), and every
// subsequent write/resize/kill from a *different* label is rejected with the
// standard label-mismatch error. This is the F-051 invariant lifted to the
// terminal axis: a session-A webview cannot steer session-B's terminals.
//
// Event model. Bytes flow out on a per-webview Tauri event named
// `terminal:bytes`. Each payload carries the `terminal_id` so the JS renderer
// can fan out to the right xterm.js instance. Exit events carry a distinct
// `code` + `killed_by_drop` shape; the renderer uses them to detach the pane.
// The emit target is always `EventTarget::webview_window(owner_label)` — the
// label is stored at spawn time and never re-read from a payload, mirroring
// the `AppHandleSink` / F-062 discipline for session events.
//
// State. `TerminalState` is a Tauri-managed `Mutex<HashMap<TerminalId,
// TerminalEntry>>`. `TerminalEntry` holds the live `TerminalSession` plus the
// owning webview label. Drop on removal SIGTERMs the child (via forge-term's
// `impl Drop`).
// ---------------------------------------------------------------------------

/// Per-terminal metadata tracked by [`TerminalState`]. The `TerminalSession`
/// drops the child on removal; `owner_label` is the label of the webview that
/// spawned the terminal and is the *only* label permitted to subsequently
/// write/resize/kill it.
pub(crate) struct TerminalEntry {
    pub(crate) session: TerminalSession,
    pub(crate) owner_label: String,
}

/// Tauri-managed registry of live terminals. Scoped per app (one instance per
/// Tauri `App`), keyed by [`TerminalId`]. Internally a `Mutex<HashMap<..>>`:
/// every command path grabs the lock briefly, never across an `await`.
#[derive(Default)]
pub struct TerminalState {
    entries: Mutex<HashMap<TerminalId, TerminalEntry>>,
}

impl TerminalState {
    fn insert(&self, id: TerminalId, entry: TerminalEntry) {
        let mut guard = self.entries.lock().expect("terminal state poisoned");
        guard.insert(id, entry);
    }

    /// Remove the terminal at `id`, returning its `TerminalSession`. Dropping
    /// the returned session SIGTERMs the child (see forge-term `impl Drop`).
    fn remove_owned_by(
        &self,
        id: &TerminalId,
        caller_label: &str,
    ) -> Result<TerminalSession, String> {
        let mut guard = self.entries.lock().expect("terminal state poisoned");
        let Some(entry) = guard.get(id) else {
            return Err(format!("unknown terminal id: {id}"));
        };
        if entry.owner_label != caller_label {
            return Err(LABEL_MISMATCH_ERROR.to_string());
        }
        // Safe to remove: ownership matches. `remove` returns the entry.
        let entry = guard.remove(id).expect("presence checked above");
        Ok(entry.session)
    }

    /// Run `op` against the terminal at `id`, validating that `caller_label`
    /// owns it first. The closure receives a `&mut TerminalSession` so it can
    /// call `write` / `resize`. Returns the op's result; if ownership does not
    /// match, returns the label-mismatch error before touching the session.
    fn with_owned_session_mut<F, T>(
        &self,
        id: &TerminalId,
        caller_label: &str,
        op: F,
    ) -> Result<T, String>
    where
        F: FnOnce(&mut TerminalSession) -> Result<T, String>,
    {
        let mut guard = self.entries.lock().expect("terminal state poisoned");
        let Some(entry) = guard.get_mut(id) else {
            return Err(format!("unknown terminal id: {id}"));
        };
        if entry.owner_label != caller_label {
            return Err(LABEL_MISMATCH_ERROR.to_string());
        }
        op(&mut entry.session)
    }
}

/// Wire-format arguments for `terminal_spawn`. `shell` is optional; when
/// absent the shell picks `$SHELL` (or `/bin/sh` on unix / `cmd.exe` on
/// windows). `cols`/`rows` mirror `TerminalSize`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct TerminalSpawnArgs {
    pub terminal_id: TerminalId,
    /// Optional shell program override. When `None`, resolved per-platform via
    /// `$SHELL` → `/bin/sh` / `cmd.exe`.
    pub shell: Option<String>,
    /// Working directory for the spawned shell. Must be an existing directory
    /// readable to the forge-shell process.
    pub cwd: String,
    pub cols: u16,
    pub rows: u16,
}

/// Event payload emitted on the `terminal:bytes` channel. `data` is a byte
/// buffer; on the JS side it is a `number[]` that xterm.js consumes via
/// `terminal.write(new Uint8Array(data))`.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct TerminalBytesEvent {
    pub terminal_id: TerminalId,
    pub data: Vec<u8>,
}

/// Event payload emitted on the `terminal:exit` channel when the underlying
/// child reaps. `code` is `Some(n)` for a normal exit; `None` when the
/// process was killed by signal (Unix) or the exit code was otherwise
/// unavailable. `killed_by_drop` is `true` when `terminal_kill` (or pane
/// teardown) initiated the termination, so the renderer can distinguish a
/// user-issued `exit` from a container detach.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct TerminalExitEvent {
    pub terminal_id: TerminalId,
    pub code: Option<i32>,
    pub killed_by_drop: bool,
}

/// Event name emitted on every raw PTY chunk. Consumed by the JS side via
/// `listen<TerminalBytesEvent>('terminal:bytes', ...)`.
pub const TERMINAL_BYTES_EVENT: &str = "terminal:bytes";
/// Event name emitted once per terminal when the child reaps.
pub const TERMINAL_EXIT_EVENT: &str = "terminal:exit";

/// Resolve the default shell when the webview did not specify one. Honors
/// `$SHELL` first, then falls back to the platform's POSIX/NT default.
fn default_shell() -> String {
    if let Ok(s) = std::env::var("SHELL") {
        if !s.is_empty() {
            return s;
        }
    }
    if cfg!(windows) {
        "cmd.exe".to_string()
    } else {
        "/bin/sh".to_string()
    }
}

/// Forward every `TerminalEvent` from a `forge-term` receiver to the owning
/// webview. Spawned as a tokio task inside `terminal_spawn`. Terminates when
/// the sender (held by `TerminalSession`) drops — either via `terminal_kill`
/// or when the child reaps naturally.
fn spawn_event_forwarder<R: Runtime>(
    app: AppHandle<R>,
    owner_label: String,
    terminal_id: TerminalId,
    mut rx: tokio::sync::mpsc::Receiver<TerminalEvent>,
) {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                TerminalEvent::Bytes(data) => {
                    let payload = TerminalBytesEvent {
                        terminal_id: terminal_id.clone(),
                        data,
                    };
                    let target = EventTarget::webview_window(&owner_label);
                    if let Err(e) = app.emit_to(target, TERMINAL_BYTES_EVENT, payload) {
                        eprintln!("terminal:bytes emit failed: {e}");
                    }
                }
                TerminalEvent::Exit(status) => {
                    let payload = TerminalExitEvent {
                        terminal_id: terminal_id.clone(),
                        code: status.code,
                        killed_by_drop: status.killed_by_drop,
                    };
                    let target = EventTarget::webview_window(&owner_label);
                    if let Err(e) = app.emit_to(target, TERMINAL_EXIT_EVENT, payload) {
                        eprintln!("terminal:exit emit failed: {e}");
                    }
                }
            }
        }
    });
}

/// Spawn a new PTY-backed terminal. The calling webview's label is recorded
/// as the terminal's owner; all subsequent commands targeting this
/// `terminal_id` are rejected unless they come from the same webview.
///
/// Errors:
/// - label not `session-*` → `forbidden: window label mismatch`
///   (see `LABEL_MISMATCH_ERROR` in this module)
/// - oversize `cwd` / `shell` → size-cap error
/// - duplicate `terminal_id` → `"terminal id already registered"`
/// - shell spawn failure → surface the `forge-term` error
#[tauri::command]
pub async fn terminal_spawn<R: Runtime>(
    args: TerminalSpawnArgs,
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, TerminalState>,
) -> Result<(), String> {
    // F-125: terminals are session-scoped. Dashboard and any other label is
    // rejected. This mirrors the chat-pane authz shape on the terminal axis.
    require_window_label_in(&webview, &[], true)?;

    require_size("cwd", &args.cwd, MAX_TERMINAL_CWD_BYTES)?;
    if let Some(shell) = args.shell.as_deref() {
        require_size("shell", shell, MAX_TERMINAL_SHELL_BYTES)?;
    }

    // Reject duplicate ids early so a caller re-using an id doesn't leak a
    // zombie registration.
    {
        let guard = state.entries.lock().expect("terminal state poisoned");
        if guard.contains_key(&args.terminal_id) {
            return Err(format!(
                "terminal id already registered: {id}",
                id = args.terminal_id
            ));
        }
    }

    let program = args.shell.unwrap_or_else(default_shell);
    let spec = ShellSpec::new(program);
    let cwd = PathBuf::from(&args.cwd);
    let size = TerminalSize {
        cols: args.cols,
        rows: args.rows,
    };

    let owner_label = webview.label().to_string();
    let terminal_id = args.terminal_id.clone();

    let (session, rx) = TerminalSession::spawn(spec, cwd, size).map_err(|e| e.to_string())?;

    state.insert(
        terminal_id.clone(),
        TerminalEntry {
            session,
            owner_label: owner_label.clone(),
        },
    );

    spawn_event_forwarder(app, owner_label, terminal_id, rx);

    Ok(())
}

/// Write `data` to the PTY. The caller must own the terminal (its webview
/// label must match the owner recorded at spawn).
#[tauri::command]
pub async fn terminal_write<R: Runtime>(
    terminal_id: TerminalId,
    data: Vec<u8>,
    webview: Webview<R>,
    state: State<'_, TerminalState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &[], true)?;

    if data.len() > MAX_TERMINAL_WRITE_BYTES {
        return Err(payload_too_large("data", MAX_TERMINAL_WRITE_BYTES));
    }

    let caller_label = webview.label().to_string();
    state.with_owned_session_mut(&terminal_id, &caller_label, |session| {
        session.write(&data).map_err(|e| e.to_string())
    })
}

/// Resize the PTY window to `(cols, rows)`. Caller must own the terminal.
#[tauri::command]
pub async fn terminal_resize<R: Runtime>(
    terminal_id: TerminalId,
    cols: u16,
    rows: u16,
    webview: Webview<R>,
    state: State<'_, TerminalState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &[], true)?;
    let caller_label = webview.label().to_string();
    state.with_owned_session_mut(&terminal_id, &caller_label, |session| {
        session.resize(cols, rows).map_err(|e| e.to_string())
    })
}

/// Tear down the terminal (drops the session → SIGTERM + reap). Caller must
/// own the terminal. Emits a final `terminal:exit` event on the owner webview.
#[tauri::command]
pub async fn terminal_kill<R: Runtime>(
    terminal_id: TerminalId,
    webview: Webview<R>,
    state: State<'_, TerminalState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &[], true)?;
    let caller_label = webview.label().to_string();
    // Removing the entry drops its `TerminalSession` — forge-term's `impl Drop`
    // SIGTERMs the child and joins the reaper thread, which emits the final
    // `TerminalEvent::Exit` on the receiver that our forwarder converts into
    // `terminal:exit`.
    let session = state.remove_owned_by(&terminal_id, &caller_label)?;
    drop(session);
    Ok(())
}

// ---------------------------------------------------------------------------
// F-120: Layout persistence commands (read_layouts / write_layouts)
//
// `.forge/layouts.json` under the workspace root stores the serialized
// `GridContainer` tree for each named layout plus per-pane state (active file,
// scroll position, terminal PID). The frontend calls `read_layouts` on session
// mount and `write_layouts` on a 500 ms debounced layout change.
//
// Authz. Both commands allow the dashboard label or any `session-*` label. The
// artifact is a workspace-level file (not per-session), so binding it to a
// specific `session-{id}` window would block the dashboard from pre-rendering
// the last saved layout and would force two near-identical commands. Reusing
// the approval commands' gate (`require_window_label_in(&[], &["dashboard"], true)`)
// keeps the policy consistent with other workspace-scoped surfaces.
//
// Fallback semantics. Missing or corrupt files degrade to
// `Layouts::default()` — a single chat leaf — not an error. A JSON-parse
// failure from a half-written or hand-edited file would otherwise leave the
// user with a blank window on every session open, which is worse than losing
// the prior layout silently.
//
// Type placement. `Layouts`, `LayoutTree`, and `PaneState` live alongside the
// other ts-rs wire shapes in this module (`PersistentApprovalEntry`,
// `TerminalSpawnArgs`) rather than `forge-ipc` — that crate is the UDS framing
// layer and does not carry ts-rs. This is the consistent pattern for Tauri
// command wire shapes in the workspace.
// ---------------------------------------------------------------------------

/// Per-pane runtime state attached to a leaf node. All fields are optional so
/// a session can persist whatever subset is meaningful to its pane type — a
/// chat pane has no terminal PID; a terminal pane has no scroll position that
/// outlives the PTY.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct PaneState {
    /// For editor panes: the file path the pane was last focused on, relative
    /// to the workspace root. `None` for panes that don't address a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_file: Option<String>,
    /// For editor / chat scroll-back panes: the top scroll offset in pixels
    /// rounded to an integer. `None` when unknown or inapplicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll_top: Option<i64>,
    /// For terminal panes: the PID of the live child shell, if any. Carried
    /// through restart so the UI can re-attach rather than spawn a new PTY —
    /// the reattach decision itself lives in F-125's terminal module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_pid: Option<u32>,
}

/// Serialized form of the `GridContainer` tree. Mirrors the TS `LayoutNode`
/// discriminated union (`kind: "leaf" | "split"`) but adds `pane_type` on
/// leaves so the renderer can pick a pane implementation on rehydrate — the
/// runtime tree's `render` callback is a closure that cannot be serialized.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum LayoutTree {
    /// A terminal pane node. `id` is stable across sessions so per-pane state
    /// keys in `Layout.pane_state` stay valid after a tree edit.
    Leaf {
        id: String,
        /// Which pane implementation to mount on the leaf. Unknown values are
        /// rejected by serde at deserialize, so a future pane type must land
        /// as a variant here before it can be persisted.
        pane_type: PaneType,
    },
    /// An internal node splitting its area between `a` and `b`. `ratio` is
    /// the fraction of the container occupied by `a`, in `0.0..=1.0`.
    Split {
        id: String,
        direction: SplitDirection,
        ratio: f32,
        a: Box<LayoutTree>,
        b: Box<LayoutTree>,
    },
}

/// The pane implementations that may be mounted on a leaf. Kept as a typed
/// enum (not a free-form string) so an unknown variant from a future version
/// fails loudly on load and the fallback-to-default path fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum PaneType {
    Chat,
    Terminal,
    Editor,
    Files,
    AgentMonitor,
}

/// Axis of a split node. Mirrors the TS `'h' | 'v'` shape exactly — `h` means
/// the two children sit side-by-side horizontally, `v` stacked vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum SplitDirection {
    H,
    V,
}

/// One named layout in the workspace. `tree` is the GridContainer shape;
/// `pane_state` holds side-car state keyed by leaf id. `pane_state` is a
/// `HashMap` so ids removed from the tree can be garbage-collected by the
/// frontend without a schema change.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct Layout {
    pub tree: LayoutTree,
    #[serde(default)]
    pub pane_state: HashMap<String, PaneState>,
}

/// The on-disk shape of `.forge/layouts.json`. Multiple named layouts share a
/// workspace (e.g. "default", "split-editor", "terminal-focus"); `active` is
/// the key into `named` the UI should restore on next session open.
///
/// Unknown keys at any level are ignored on load — the TS and Rust shapes
/// evolve in lockstep but tolerate hand-edits that drop optional fields.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct Layouts {
    pub active: String,
    pub named: HashMap<String, Layout>,
}

impl Default for Layouts {
    fn default() -> Self {
        let mut named = HashMap::new();
        named.insert(
            "default".to_string(),
            Layout {
                tree: LayoutTree::Leaf {
                    id: "root".to_string(),
                    pane_type: PaneType::Chat,
                },
                pane_state: HashMap::new(),
            },
        );
        Self {
            active: "default".to_string(),
            named,
        }
    }
}

/// Resolve `<workspace_root>/.forge/layouts.json`. Does not create anything —
/// callers that write decide when to `create_dir_all`.
fn layouts_file_path(workspace_root: &std::path::Path) -> PathBuf {
    workspace_root.join(".forge").join("layouts.json")
}

/// Sibling `<path>.tmp` for the atomic tmp+rename write in `write_layouts`.
/// Mirrors the `tmp_path_for` helpers in `forge_core::approvals` and
/// `forge_core::settings` — same-directory by construction so the rename is
/// same-filesystem (POSIX-atomic). F-372 tracks promoting this to a shared
/// `forge_core::atomic_write`; this keeps the 90-min fix local.
fn layouts_tmp_path(path: &std::path::Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    file_name.push(".tmp");
    match path.parent() {
        Some(parent) => parent.join(file_name),
        None => PathBuf::from(file_name),
    }
}

/// Load `.forge/layouts.json` under `workspace_root`, degrading to
/// [`Layouts::default`] on any failure.
///
/// Degradation targets are:
/// - file missing (fresh workspace, first session open);
/// - file unreadable (permissions anomaly on a dev machine);
/// - file present but invalid JSON (user hand-edit, crash-during-write, or
///   a forward-incompatible variant we now reject).
///
/// A silent fallback is preferable to surfacing the error to the webview: a
/// failed read would leave the UI with no layout to mount and the user with a
/// blank window. Losing the persisted layout is recoverable; losing the
/// ability to open the session is not.
async fn load_layouts_from_disk(workspace_root: &std::path::Path) -> Layouts {
    let path = layouts_file_path(workspace_root);
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return Layouts::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Read the persisted layouts for `workspace_root`. Missing or corrupt files
/// return the default single-pane layout — a failed read would otherwise
/// leave the webview with no layout to mount and the user with a blank
/// window. Authz: dashboard or any `session-*` label.
#[tauri::command]
pub async fn read_layouts<R: Runtime>(
    workspace_root: String,
    webview: Webview<R>,
) -> Result<Layouts, String> {
    require_window_label_in(&webview, &["dashboard"], true)?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;
    Ok(load_layouts_from_disk(std::path::Path::new(&workspace_root)).await)
}

/// Persist `layouts` to `<workspace_root>/.forge/layouts.json`, creating the
/// `.forge/` directory on first save. Authz: dashboard or any `session-*`
/// label. Write failures (disk full, read-only mount) surface as `Err` — the
/// frontend debouncer will retry on the next layout change, so a transient
/// failure does not need a retry loop here.
///
/// F-363: mirrors the `approvals::save_to_path` / `settings::save_raw_to_path`
/// atomic tmp+rename pattern. The rename is atomic on POSIX for same-
/// filesystem targets (same directory here by construction), so a crash
/// between the write and the rename leaves either the prior `layouts.json`
/// or the new one on disk — never a partial JSON payload.
#[tauri::command]
pub async fn write_layouts<R: Runtime>(
    workspace_root: String,
    layouts: Layouts,
    webview: Webview<R>,
) -> Result<(), String> {
    require_window_label_in(&webview, &["dashboard"], true)?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;

    let forge_dir = std::path::Path::new(&workspace_root).join(".forge");
    tokio::fs::create_dir_all(&forge_dir)
        .await
        .map_err(|e| format!("create .forge dir: {e}"))?;

    let path = forge_dir.join("layouts.json");
    let tmp = layouts_tmp_path(&path);
    let bytes = serde_json::to_vec_pretty(&layouts).map_err(|e| format!("serialize: {e}"))?;
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| format!("write layouts.json.tmp: {e}"))?;
    tokio::fs::rename(&tmp, &path)
        .await
        .map_err(|e| format!("rename layouts.json.tmp: {e}"))
}

// ---------------------------------------------------------------------------
// F-122: filesystem commands for the EditorPane (read_file, write_file, tree).
//
// The webview-facing editor pane needs to read file contents into Monaco,
// write them back on save, and list the workspace tree for the file-picker
// that F-126 will surface. All three commands:
//
// 1. Require `session-{session_id}` window-label authz (F-051 / H10) — the
//    editor only runs inside a session window, so the session-scoped gate
//    is correct and keeps the dashboard from invoking these paths.
// 2. Look up `workspace_root` server-side from `SessionConnections`
//    (populated at `session_hello` from the daemon's `HelloAck.workspace`).
//    The signature deliberately does NOT accept `workspace_root` from the
//    webview: a compromised or buggy webview cannot widen its sandbox by
//    lying about the workspace root because the command never reads a
//    webview-supplied value. The cache was the closed-PR-279 gap.
// 3. Delegate size caps and symlink rejection to `forge-fs` (F-061 / M3) so
//    the filesystem trust boundary lives in one place.
// ---------------------------------------------------------------------------

/// F-122: per-field cap on byte payloads handed to `write_file`. Matches
/// `forge_fs::Limits::max_write_bytes` default (10 MiB); keep the pre-check
/// here so the Tauri frame bound in `forge_ipc::write_frame` (4 MiB) doesn't
/// short-circuit the cap with a more confusing "frame too large" error. The
/// actual enforcement authority is still `forge-fs`.
pub(crate) const MAX_EDITOR_WRITE_BYTES: usize = 10 * 1024 * 1024;

/// F-122: cap on `path` / `root` byte length. Same PATH_MAX envelope as
/// `MAX_WORKSPACE_ROOT_BYTES` — kept as a distinct constant so any future
/// divergence stays localized.
pub(crate) const MAX_FS_PATH_BYTES: usize = 4096;

/// Wire shape returned by `read_file`. `content` is UTF-8 (lossy on decode)
/// so the generated TS binding is a plain string — aligns with Monaco's
/// `ITextModel.getValue()`. `sha256` lets the frontend detect drift between
/// a buffer it believes it has saved and a reload result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub bytes: u32,
    pub sha256: String,
}

/// Wire shape for `tree`. The root is always returned; `children` is `None`
/// for non-directory entries and `Some(_)` for directories (empty vec when
/// the depth cap is hit or the directory is empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct TreeNodeDto {
    pub name: String,
    /// Absolute canonicalized path. The frontend joins this with a `file://`
    /// scheme when building Monaco URIs so a round-trip to `read_file` lands
    /// on the same on-disk object.
    pub path: String,
    pub kind: TreeKindDto,
    pub children: Option<Vec<TreeNodeDto>>,
}

/// Wire shape for [`TreeNodeDto::kind`]. Narrow on purpose — anything
/// unusual (block device, socket, FIFO) maps to `Other` so the wire shape is
/// stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum TreeKindDto {
    File,
    Dir,
    Symlink,
    Other,
}

impl From<forge_fs::TreeNode> for TreeNodeDto {
    fn from(node: forge_fs::TreeNode) -> Self {
        use forge_fs::NodeKind;
        let kind = match node.kind {
            NodeKind::File => TreeKindDto::File,
            NodeKind::Dir => TreeKindDto::Dir,
            NodeKind::Symlink => TreeKindDto::Symlink,
            NodeKind::Other => TreeKindDto::Other,
        };
        Self {
            name: node.name,
            path: node.path.to_string_lossy().into_owned(),
            kind,
            children: node
                .children
                .map(|cs| cs.into_iter().map(TreeNodeDto::from).collect()),
        }
    }
}

/// Build the `forge-fs` allowlist for a session's workspace from the trusted
/// cached canonical `workspace_root`. The glob form (`<root>/**` plus an
/// exact `<root>` match) mirrors the pattern used by the existing
/// `fs.read` / `fs.write` tool call paths.
fn workspace_allowlist(workspace_root: &std::path::Path) -> Vec<String> {
    let display = workspace_root.to_string_lossy();
    vec![format!("{display}/**"), display.into_owned()]
}

/// F-122: resolve the authoritative workspace root for `session_id`.
///
/// Returns the cached canonical path populated by `session_hello` at
/// handshake time. A webview cannot override this value — the commands
/// below never accept a `workspace_root` parameter. When the cache is
/// empty (session_hello not yet called), we return the same "not connected"
/// error shape as the bridge's write paths so a misordered UI call gets a
/// loud, specific message instead of a path-denied downstream.
async fn cached_workspace_root(
    state: &BridgeState,
    session_id: &str,
) -> Result<std::path::PathBuf, String> {
    state
        .bridge
        .connections()
        .workspace_root(session_id)
        .await
        .ok_or_else(|| {
            format!(
                "session {session_id} not connected: call session_hello before \
                 read_file/write_file/tree"
            )
        })
}

#[tauri::command]
pub async fn read_file<R: Runtime>(
    session_id: String,
    path: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<FileContent, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("path", &path, MAX_FS_PATH_BYTES)?;

    let workspace = cached_workspace_root(&state, &session_id).await?;
    let allowed = workspace_allowlist(&workspace);
    let limits = forge_fs::Limits::default();
    let result = forge_fs::read_file(&path, &allowed, &limits).map_err(|e| e.to_string())?;
    Ok(FileContent {
        path,
        content: result.content,
        bytes: result.bytes as u32,
        sha256: result.sha256,
    })
}

#[tauri::command]
pub async fn write_file<R: Runtime>(
    session_id: String,
    path: String,
    bytes: Vec<u8>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("path", &path, MAX_FS_PATH_BYTES)?;
    // Pre-check before `forge-fs` copies the buffer into the atomic-write
    // temp file. `forge-fs` also enforces; belt-and-braces keeps the error
    // source local and predictable.
    if bytes.len() > MAX_EDITOR_WRITE_BYTES {
        return Err(payload_too_large("bytes", MAX_EDITOR_WRITE_BYTES));
    }

    let workspace = cached_workspace_root(&state, &session_id).await?;
    let allowed = workspace_allowlist(&workspace);
    let limits = forge_fs::Limits::default();
    forge_fs::write_bytes(&path, &bytes, &allowed, &limits).map_err(|e| e.to_string())
}

/// `tree(session_id, root, depth?)` — list the filesystem subtree rooted at
/// `root`. Pass the workspace root itself as `root` to list the whole
/// workspace. `depth` defaults to 6 and is capped at 16.
///
/// `root` is validated server-side against the cached workspace allowlist —
/// a webview cannot escape its sandbox by supplying a `root` outside the
/// session's workspace.
#[tauri::command]
pub async fn tree<R: Runtime>(
    session_id: String,
    root: String,
    depth: Option<u32>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<TreeNodeDto, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("root", &root, MAX_FS_PATH_BYTES)?;

    let workspace = cached_workspace_root(&state, &session_id).await?;
    let allowed = workspace_allowlist(&workspace);
    // Cap depth at 16 so a pathological request can't walk a Linux-style
    // deep tree and return a megabyte of JSON. 16 is deep enough for the
    // workspaces Forge targets; the entry budget inside `forge-fs` is the
    // second line of defense.
    let requested = depth.unwrap_or(6).min(16);
    // F-126: the FilesSidebar uses `tree` to render the workspace; honor
    // `.gitignore` (plus `.ignore`, global gitignore, parent chains, and
    // hidden files) via the `ignore` crate so the sidebar matches what a
    // developer sees in VS Code. The non-ignored `list_tree` is retained
    // in `forge-fs` for other callers (e.g. agent tool paths that may need
    // to inspect gitignored files).
    let node =
        forge_fs::list_tree_gitignored(&root, &allowed, requested).map_err(|e| e.to_string())?;
    Ok(TreeNodeDto::from(node))
}

// ---------------------------------------------------------------------------
// F-123: LSP bridge commands (lsp_start / lsp_stop / lsp_send)
//
// Scope. Three Tauri commands connect the parent webview's iframe LSP client
// (F-121) to a `forge-lsp::Server` stdio subprocess. Each spawned server is
// owned by exactly one webview label (the label that called `lsp_start`),
// and every subsequent send/stop from a different label is rejected with the
// standard label-mismatch error — mirroring the F-125 terminal authz story on
// the LSP axis.
//
// Scope divergence. Architecture doc §3.7 describes `forge-lsp` as a
// management layer ("doesn't proxy LSP messages"). F-121 and this task
// reshape that: the iframe talks to `forge-lsp` through the parent webview's
// IPC bridge. F-148 is the doc-reconcile follow-up. The proxy is
// byte-transparent — we never parse LSP frames on the Rust side.
//
// Event model. Bytes flow out on a per-webview Tauri event named
// `lsp_message`. Each payload carries the `server` id so the JS renderer
// can route it to the right iframe. The emit target is always
// `EventTarget::webview_window(owner_label)` — the label is stored at spawn
// time and never re-read from a payload, mirroring `AppHandleSink`'s F-062
// discipline.
// ---------------------------------------------------------------------------

/// Event name for server → parent-webview LSP messages.
pub const LSP_MESSAGE_EVENT: &str = "lsp_message";

/// Size caps for LSP command inputs. Mirrors the terminal caps shape — LSP
/// frames can be large (initialize payloads with workspace folders + document
/// uris run tens of KiB), so the ceiling is generous but still bounded so a
/// compromised webview can't loop 1 MiB sends billing server memory.
pub(crate) const MAX_LSP_SERVER_ID_BYTES: usize = 128;
pub(crate) const MAX_LSP_MESSAGE_BYTES: usize = 512 * 1024;

/// Wire-format arguments for `lsp_start`.
///
/// F-353: the binary path is no longer part of the wire format. The webview
/// names a server by id; the shell resolves the binary through the bundled
/// [`forge_lsp::Registry`] plus [`forge_lsp::Bootstrap`]'s cache-root
/// sandbox. A compromised or buggy webview that forwarded an
/// attacker-controlled path used to land as `Command::new(path).spawn()`;
/// that surface is closed.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct LspStartArgs {
    /// Server identifier (matches `forge_lsp::ServerId` entries in the
    /// bundled registry). Used as the routing key on `lsp_message` events
    /// and as the registry lookup key for the binary path — the webview
    /// never names a filesystem path directly.
    pub server: String,
    /// Optional extra argv appended to the spec's declared args. Bounded
    /// via the server-id cap since each arg is expected to be short;
    /// oversize argv is a misuse.
    #[serde(default)]
    pub args: Vec<String>,
}

/// Event payload pushed on `lsp_message` when the server emits a frame.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct LspMessageEvent {
    /// Server that produced the message (matches `LspStartArgs.server`).
    pub server: String,
    /// Opaque JSON-RPC payload. Parsing happens inside the iframe — ts-rs
    /// emits `unknown` so the TS side is forced to narrow before use.
    #[ts(type = "unknown")]
    pub message: serde_json::Value,
}

/// Per-server bookkeeping tracked by [`LspState`]. The supervisor task is
/// aborted on entry removal; its Drop kills the child via the `kill_on_drop`
/// flag set inside `forge-lsp`.
pub(crate) struct LspEntry {
    /// Transport handle for `lsp_send`. Sends go through this, not the
    /// supervisor task.
    pub(crate) transport: Arc<dyn MessageTransport>,
    /// Webview label that called `lsp_start` — the only label permitted
    /// to subsequently send/stop this server.
    pub(crate) owner_label: String,
    /// Supervisor task. Aborting drops the child.
    pub(crate) supervisor: tauri::async_runtime::JoinHandle<()>,
    /// Event forwarder task. Aborting stops message delivery to the webview.
    pub(crate) forwarder: tauri::async_runtime::JoinHandle<()>,
}

/// Tauri-managed registry of live LSP servers. Scoped per app (one instance
/// per Tauri `App`), keyed by `server` id.
#[derive(Default)]
pub struct LspState {
    entries: Mutex<HashMap<String, LspEntry>>,
}

impl LspState {
    fn insert(&self, id: String, entry: LspEntry) -> Result<(), String> {
        let mut guard = self.entries.lock().expect("lsp state poisoned");
        if guard.contains_key(&id) {
            return Err(format!("lsp server already running: {id}"));
        }
        guard.insert(id, entry);
        Ok(())
    }

    fn remove_owned_by(&self, id: &str, caller_label: &str) -> Result<LspEntry, String> {
        let mut guard = self.entries.lock().expect("lsp state poisoned");
        let Some(entry) = guard.get(id) else {
            return Err(format!("unknown lsp server: {id}"));
        };
        if entry.owner_label != caller_label {
            return Err(LABEL_MISMATCH_ERROR.to_string());
        }
        let entry = guard.remove(id).expect("presence checked above");
        Ok(entry)
    }

    fn owned_transport(
        &self,
        id: &str,
        caller_label: &str,
    ) -> Result<Arc<dyn MessageTransport>, String> {
        let guard = self.entries.lock().expect("lsp state poisoned");
        let Some(entry) = guard.get(id) else {
            return Err(format!("unknown lsp server: {id}"));
        };
        if entry.owner_label != caller_label {
            return Err(LABEL_MISMATCH_ERROR.to_string());
        }
        Ok(entry.transport.clone())
    }
}

/// Forward every `ServerEvent::Message` from the supervisor to the owning
/// webview as an `lsp_message` Tauri event. Mirrors `spawn_event_forwarder`
/// for terminals: the target label is bound at spawn time and never re-read
/// from a payload, so a forged payload cannot redirect delivery.
fn spawn_lsp_forwarder<R: Runtime>(
    app: AppHandle<R>,
    owner_label: String,
    server_id: String,
    mut rx: tokio::sync::mpsc::Receiver<LspServerEvent>,
) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let LspServerEvent::Message(value) = event {
                let payload = LspMessageEvent {
                    server: server_id.clone(),
                    message: value,
                };
                let target = EventTarget::webview_window(&owner_label);
                if let Err(e) = app.emit_to(target, LSP_MESSAGE_EVENT, payload) {
                    eprintln!("lsp_message emit failed: {e}");
                }
            }
            // `Exited` / `GaveUp` events are observable only server-side
            // today; the iframe protocol surfaces restart transparently
            // because the transport reinstalls stdin between attempts.
        }
    })
}

/// Start a supervised LSP server. The calling webview's label is recorded
/// as the server's owner; subsequent commands targeting this `server` id
/// are rejected unless they come from the same webview.
///
/// F-353: the binary is resolved through the bundled [`forge_lsp::Registry`]
/// + [`forge_lsp::Bootstrap`]'s cache-root sandbox. The webview cannot
/// supply a filesystem path; an unknown `server` id returns
/// `"unknown lsp server: <id>"`; any registry entry whose resolved path
/// escapes the cache root is rejected with a sandbox error before
/// `Command::new` is ever called.
///
/// Errors:
/// - label not `session-*` → `forbidden: window label mismatch`
/// - oversize `server` → size-cap error
/// - duplicate `server` id → `"lsp server already running: <id>"`
/// - unknown `server` id → `"unknown lsp server: <id>"`
/// - lsp bootstrap not wired → `"lsp bootstrap unavailable"`
/// - spawn failure surfaces the `forge-lsp` error
#[tauri::command]
pub async fn lsp_start<R: Runtime>(
    args: LspStartArgs,
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, LspState>,
    bootstrap: State<'_, LspBootstrapState>,
) -> Result<(), String> {
    // LSP servers are session-scoped. Dashboard and any other label is
    // rejected — mirrors the terminal authz shape on the LSP axis.
    require_window_label_in(&webview, &[], true)?;

    require_size("server", &args.server, MAX_LSP_SERVER_ID_BYTES)?;

    let owner_label = webview.label().to_string();
    let server_id = args.server.clone();

    let bootstrap = bootstrap
        .snapshot()
        .ok_or_else(|| "lsp bootstrap unavailable".to_string())?;

    // Registry gate: a webview-supplied id must match a known spec. This
    // is what replaces the raw `binary_path` surface — the IPC cannot
    // name an executable, only an id the shell already trusts.
    let spec = bootstrap
        .registry()
        .get_by_str(&server_id)
        .ok_or_else(|| format!("unknown lsp server: {server_id}"))?;

    // Build the supervisor + transport before registering, so spawn failures
    // don't leave a zombie entry. `from_registry` enforces the cache-root
    // sandbox on the resolved binary path; any `BootstrapError::SandboxEscape`
    // surfaces as `ServerError::SandboxEscape` and never reaches `Command`.
    let mut supervisor =
        LspServer::from_registry(spec.id, &bootstrap, args.args).map_err(|e| e.to_string())?;
    let transport = supervisor.transport();
    let rx = supervisor
        .take_events()
        .ok_or_else(|| "lsp_start: event channel already taken".to_string())?;

    // Register before starting so `lsp_send` can observe the transport
    // immediately. The supervisor races spawn with the first send; the
    // transport itself returns `NotRunning` until the child is up.
    let forwarder = spawn_lsp_forwarder(app.clone(), owner_label.clone(), server_id.clone(), rx);

    let supervisor_handle = tauri::async_runtime::spawn(async move {
        // Errors here (bad path, etc.) are recorded on the event channel
        // via the supervisor's internal paths; we swallow the top-level
        // Result because the forwarder terminates when the channel closes.
        let _ = supervisor.start().await;
    });

    state.insert(
        server_id,
        LspEntry {
            transport,
            owner_label,
            supervisor: supervisor_handle,
            forwarder,
        },
    )?;

    Ok(())
}

/// Stop the server owned by the caller. Aborts the supervisor + forwarder
/// tasks — the child is killed via `tokio::process::Command::kill_on_drop`.
#[tauri::command]
pub async fn lsp_stop<R: Runtime>(
    server: String,
    webview: Webview<R>,
    state: State<'_, LspState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &[], true)?;
    require_size("server", &server, MAX_LSP_SERVER_ID_BYTES)?;
    let caller_label = webview.label().to_string();
    let entry = state.remove_owned_by(&server, &caller_label)?;
    entry.supervisor.abort();
    entry.forwarder.abort();
    Ok(())
}

/// Forward a JSON-RPC message to the server's stdin. Caller must own the
/// server. The message is opaque — this crate never parses LSP frames on
/// behalf of the iframe.
#[tauri::command]
pub async fn lsp_send<R: Runtime>(
    server: String,
    message: serde_json::Value,
    webview: Webview<R>,
    state: State<'_, LspState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &[], true)?;
    require_size("server", &server, MAX_LSP_SERVER_ID_BYTES)?;
    // Rough byte cap: serialize once and check length. Avoids building a
    // serialized frame twice (send path also serializes) by short-circuiting
    // oversize payloads before the transport touches stdin.
    let encoded_len = serde_json::to_vec(&message)
        .map(|v| v.len())
        .map_err(|e| format!("serialize lsp message: {e}"))?;
    if encoded_len > MAX_LSP_MESSAGE_BYTES {
        return Err(payload_too_large("message", MAX_LSP_MESSAGE_BYTES));
    }

    let caller_label = webview.label().to_string();
    let transport = state.owned_transport(&server, &caller_label)?;
    transport.send(message).await.map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// F-137: background-agent lifecycle commands.
//
// Three Tauri commands let a session webview drive top-level user-initiated
// agents that run alongside the active chat and surface in the Agent Monitor
// pane (see `docs/product/ai-ux.md` §10.6). Distinct from F-134's
// `agent.spawn` (sub-agents spawned by an agent as part of orchestration) —
// a background agent is started by the user and its lifecycle events flow
// onto the same `session:event` channel the daemon already uses, so the
// webview store picks up `BackgroundAgentStarted` / `BackgroundAgentCompleted`
// alongside every other `forge_core::Event` variant it already handles.
//
// State. `BgAgentState` is a Tauri-managed `Mutex<HashMap<session_id,
// BgAgentSession>>`. Each `BgAgentSession` owns:
//   - a `BackgroundAgentRegistry` (the session-scoped lifecycle owner),
//   - a forwarder `JoinHandle` that drains the registry's local broadcast
//     channel and re-emits each `forge_core::Event` as a `SessionEventPayload`
//     via `AppHandleSink` — the same path the daemon's `session:event`
//     subscription uses, so the webview does not see a new event name.
//
// Authz. Each command asserts `require_window_label(session-{id})`. The
// session-label binding is what guarantees a compromised or buggy
// session-A webview cannot drive session-B's background-agent lifecycle.
//
// Agent defs. Resolution happens against the workspace + user-home `.agents/*.md`
// files via `forge_agents::load_agents`, with the cached workspace root from
// `SessionConnections::workspace_root` as the authoritative source — a
// webview can't widen scope by injecting its own `workspace_root` because
// the commands never read a webview-supplied value.
// ---------------------------------------------------------------------------

/// Per-session background-agent bookkeeping owned by [`BgAgentState`].
pub(crate) struct BgAgentSession {
    pub(crate) registry: Arc<forge_session::BackgroundAgentRegistry>,
    /// Forwarder task: reads the registry's local broadcast channel and emits
    /// each event onto the session's `session:event` webview channel. Aborted
    /// when the `BgAgentSession` is removed from the map (none of the
    /// commands currently remove, but a future `session_disconnect` will).
    pub(crate) forwarder: tauri::async_runtime::JoinHandle<()>,
}

impl Drop for BgAgentSession {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

/// Tauri-managed map of live per-session background-agent registries.
/// One per Tauri App. The inner map is keyed by `session_id`.
#[derive(Default)]
pub struct BgAgentState {
    inner: Mutex<HashMap<String, Arc<BgAgentSession>>>,
}

impl BgAgentState {
    fn get(&self, session_id: &str) -> Option<Arc<BgAgentSession>> {
        let guard = self.inner.lock().expect("bg-agent state poisoned");
        guard.get(session_id).cloned()
    }

    fn insert(&self, session_id: String, entry: Arc<BgAgentSession>) {
        let mut guard = self.inner.lock().expect("bg-agent state poisoned");
        guard.insert(session_id, entry);
    }
}

/// Attach a fresh [`BgAgentState`] to the app. Idempotent — matches the
/// `manage_terminals` / `manage_lsp` pattern so `window_manager::run` can
/// call once and integration tests can opt in via a `make_app`-style
/// helper (see `tests/ipc_bg_agents.rs`).
pub fn manage_bg_agents<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<BgAgentState>().is_none() {
        app.manage(BgAgentState::default());
    }
}

/// F-137 test seam: construct a per-session `BgAgentSession` without
/// invoking any Tauri command. Integration tests use this to pre-populate
/// the registry for a session so they can drive `start` / `promote` /
/// `list` directly and observe the forwarder emitting events on the
/// session's webview channel.
///
/// Production code goes through [`resolve_bg_session`] (below) which
/// performs the same construction on first invoke.
#[cfg(feature = "webview-test")]
#[doc(hidden)]
pub fn install_bg_session_for_test<R: Runtime>(
    app: &AppHandle<R>,
    session_id: &str,
    registry: Arc<forge_session::BackgroundAgentRegistry>,
) {
    manage_bg_agents(app);
    let entry = Arc::new(new_bg_session(
        app.clone(),
        session_id.to_string(),
        registry,
    ));
    let state = app.state::<BgAgentState>();
    state.insert(session_id.to_string(), entry);
}

/// Build a fresh `BgAgentSession` wrapping `registry` and spawn its
/// forwarder. Factored out so the production resolution path and the
/// test-only seam stay in sync on the forwarder shape.
fn new_bg_session<R: Runtime>(
    app: AppHandle<R>,
    session_id: String,
    registry: Arc<forge_session::BackgroundAgentRegistry>,
) -> BgAgentSession {
    let mut events = registry.events();
    let sink_session_id = session_id.clone();
    let forwarder = tauri::async_runtime::spawn(async move {
        // A local monotonic seq so the webview sees well-ordered payloads
        // for the BackgroundAgent* events — the daemon-side `session:event`
        // stream emits its own increasing seq, but the two sources are
        // independent. Starting at 0 is fine: the webview UI uses the
        // event variant, not the seq, to update the Agent Monitor row.
        let mut seq: u64 = 0;
        let sink = AppHandleSink {
            app,
            session_id: sink_session_id.clone(),
        };
        loop {
            match events.recv().await {
                Ok(event) => {
                    seq += 1;
                    sink.emit(crate::bridge::SessionEventPayload {
                        session_id: sink_session_id.clone(),
                        seq,
                        event,
                    });
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // A slow consumer dropped events. Continue — the next
                    // `recv` will return the next live event. Missed
                    // lifecycle events are recoverable: the webview re-fetches
                    // via `list_background_agents` when the user opens the
                    // Agent Monitor.
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    BgAgentSession {
        registry,
        forwarder,
    }
}

/// Resolve (or lazily construct) the `BgAgentSession` for `session_id`.
///
/// On first invoke per session we:
///   1. look up the cached workspace root (populated by `session_hello`),
///   2. load agent defs via `forge_agents::load_agents`,
///   3. build a new `forge_agents::Orchestrator` + `BackgroundAgentRegistry`,
///   4. spawn the forwarder task and insert the entry into `BgAgentState`.
///
/// Subsequent invokes reuse the cached entry. Production callers never pass
/// a pre-built registry; tests inject one via `install_bg_session_for_test`.
async fn resolve_bg_session<R: Runtime>(
    app: &AppHandle<R>,
    state: &BridgeState,
    bg_state: &BgAgentState,
    session_id: &str,
) -> Result<Arc<BgAgentSession>, String> {
    if let Some(entry) = bg_state.get(session_id) {
        return Ok(entry);
    }

    // Lazy init — workspace root is the authoritative source for agent defs.
    let workspace_root = cached_workspace_root(state, session_id).await?;
    let user_home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));

    let defs = forge_agents::load_agents(&workspace_root, &user_home)
        .map_err(|e| format!("load agent defs: {e}"))?;

    let orchestrator = Arc::new(forge_agents::Orchestrator::new());
    let registry = Arc::new(forge_session::BackgroundAgentRegistry::new(
        orchestrator,
        Arc::new(defs),
    ));
    let entry = Arc::new(new_bg_session(
        app.clone(),
        session_id.to_string(),
        registry,
    ));
    bg_state.insert(session_id.to_string(), Arc::clone(&entry));
    Ok(entry)
}

/// Wire shape returned by `list_background_agents`. Mirrors
/// `forge_session::BgAgentSummary` but re-derives `TS` here so the generated
/// binding lands under `web/packages/ipc/src/generated/`. Keeping the
/// `#[derive(TS)]` at the Tauri boundary matches the pattern used by
/// `PersistentApprovalEntry`, `TerminalSpawnArgs`, etc.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct BgAgentSummary {
    pub id: String,
    pub agent_name: String,
    pub state: BgAgentStateDto,
}

/// Three-way lifecycle tag for the background-agent row. Mirrors
/// `forge_session::BgAgentState` at the wire boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum BgAgentStateDto {
    Running,
    Completed,
    Failed,
}

impl From<forge_session::BgAgentSummary> for BgAgentSummary {
    fn from(s: forge_session::BgAgentSummary) -> Self {
        let state = match s.state {
            forge_session::BgAgentState::Running => BgAgentStateDto::Running,
            forge_session::BgAgentState::Completed => BgAgentStateDto::Completed,
            forge_session::BgAgentState::Failed => BgAgentStateDto::Failed,
        };
        Self {
            id: s.id.to_string(),
            agent_name: s.agent_name,
            state,
        }
    }
}

/// F-137 size caps. `agent_name` mirrors the `AgentDef.name` glob — short by
/// convention. `prompt` shares the `MAX_MESSAGE_TEXT_BYTES` ceiling with
/// `session_send_message` so a user pasting a long seed message for a
/// background agent hits the same limit they already know from the chat
/// composer. `instance_id` is a 16-char hex handle.
pub(crate) const MAX_AGENT_NAME_BYTES: usize = 256;
pub(crate) const MAX_BG_PROMPT_BYTES: usize = MAX_MESSAGE_TEXT_BYTES;
pub(crate) const MAX_AGENT_INSTANCE_ID_BYTES: usize = 64;

/// Start a background agent.
///
/// Errors:
/// - unauthorized window label → `forbidden: window label mismatch`
/// - oversize `agent_name` / `prompt` → size-cap error
/// - unknown `agent_name` → `start_background_agent: unknown agent '<name>'`
/// - session not connected → same `cached_workspace_root` error as the
///   editor-pane commands
#[tauri::command]
pub async fn start_background_agent<R: Runtime>(
    session_id: String,
    agent_name: String,
    prompt: String,
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
    bg_state: State<'_, BgAgentState>,
) -> Result<String, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("agent_name", &agent_name, MAX_AGENT_NAME_BYTES)?;
    require_size("prompt", &prompt, MAX_BG_PROMPT_BYTES)?;

    let entry = resolve_bg_session(&app, &state, &bg_state, &session_id).await?;
    let id = entry
        .registry
        .start(&agent_name, Arc::from(prompt.as_str()))
        .await
        .map_err(|e| format!("start_background_agent: {e}"))?;
    Ok(id.to_string())
}

/// Promote a background agent to an active chat pane (observable state
/// change only: removes the id from the tracked set). The frontend
/// responds to the returned ack by mounting a new chat pane bound to the
/// instance id — pane geometry is a webview concern.
#[tauri::command]
pub async fn promote_background_agent<R: Runtime>(
    session_id: String,
    instance_id: String,
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
    bg_state: State<'_, BgAgentState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("instance_id", &instance_id, MAX_AGENT_INSTANCE_ID_BYTES)?;

    let entry = resolve_bg_session(&app, &state, &bg_state, &session_id).await?;
    let id = forge_core::AgentInstanceId::from_string(instance_id);
    entry.registry.promote(&id).await;
    Ok(())
}

/// Snapshot of the session's currently-tracked background agents.
#[tauri::command]
pub async fn list_background_agents<R: Runtime>(
    session_id: String,
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
    bg_state: State<'_, BgAgentState>,
) -> Result<Vec<BgAgentSummary>, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;

    let entry = resolve_bg_session(&app, &state, &bg_state, &session_id).await?;
    let rows = entry.registry.list().await;
    Ok(rows.into_iter().map(BgAgentSummary::from).collect())
}

// ---------------------------------------------------------------------------
// F-126: filesystem mutation commands for the FilesSidebar context menu.
//
// `rename_path` and `delete_path` are siblings of F-122's `read_file` /
// `write_file` / `tree`. They inherit the same authz + sandbox posture:
//
// 1. Session-scoped `require_window_label` gate — the FilesSidebar only
//    renders inside a session window.
// 2. Server-side cached `workspace_root` lookup (via `cached_workspace_root`,
//    the same helper F-122's commands use). The webview does NOT supply a
//    `workspace_root` parameter — a lying webview cannot widen its sandbox.
// 3. Path sandbox enforced by `forge-fs::rename` / `forge-fs::delete`, which
//    glob-match canonicalized inputs against the workspace allowlist.
//
// Appended at EOF per the concurrent-worktree convention (F-137 / F-144 are
// touching adjacent code simultaneously; keeping new additions at the bottom
// minimizes rebase conflicts).
// ---------------------------------------------------------------------------

/// Rename / move `from` -> `to` inside the session's workspace. Both paths
/// go through the `forge-fs` allowlist; a rename that would move an entry
/// outside the workspace (or move an outside entry in) is rejected with a
/// path-denied error.
///
/// Clobbering an existing destination is refused — the UI collects a fresh
/// name up-front, and a silent overwrite would break the audit trail.
#[tauri::command]
pub async fn rename_path<R: Runtime>(
    session_id: String,
    from: String,
    to: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("from", &from, MAX_FS_PATH_BYTES)?;
    require_size("to", &to, MAX_FS_PATH_BYTES)?;

    let workspace = cached_workspace_root(&state, &session_id).await?;
    let allowed = workspace_allowlist(&workspace);
    let limits = forge_fs::Limits::default();
    forge_fs::rename(&from, &to, &allowed, &limits).map_err(|e| e.to_string())
}

/// Delete the entry at `path` inside the session's workspace. Files are
/// removed via `remove_file`; directories are removed recursively via
/// `remove_dir_all`. Symlinked path components are rejected before any
/// filesystem mutation happens.
#[tauri::command]
pub async fn delete_path<R: Runtime>(
    session_id: String,
    path: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("path", &path, MAX_FS_PATH_BYTES)?;

    let workspace = cached_workspace_root(&state, &session_id).await?;
    let allowed = workspace_allowlist(&workspace);
    let limits = forge_fs::Limits::default();
    forge_fs::delete(&path, &allowed, &limits).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// F-151: user + workspace settings store
//
// Two Tauri commands expose the persistent settings to the frontend:
//
// - `get_settings(workspace_root) -> AppSettings` — loads both the user and
//   workspace files, deep-merges workspace onto user at TOML-tree granularity
//   (so a workspace file that declares only `[notifications]` does not
//   overwrite the user's `[windows]` preference), and returns the merged
//   shape.
// - `set_setting(key, value, level, workspace_root)` — writes a single
//   `(dotted_key, value)` into the requested tier's file, preserving every
//   other field that already lives there. Implemented as load → mutate the
//   raw toml tree → validate by deserializing into `AppSettings` → atomically
//   rewrite the file. A blind struct-serialize + rewrite would promote
//   defaults into the file for every absent field, defeating the merge
//   semantic.
//
// Authz mirrors F-036's approval commands: the dashboard window and any
// `session-*` window may invoke. Settings are a user-level artifact, not
// per-session, so there is no session-label check inside the command.
//
// Appended at EOF (Wave 2B-a convention, same as the F-144 fs commands
// above) — several parallel PRs are also touching this file, so new
// additions stay at the bottom to minimize rebase conflicts.
// ---------------------------------------------------------------------------

use forge_core::settings::{
    apply_setting_update, load_merged_in, save_user_settings_raw_in, save_workspace_settings_raw,
    workspace_settings_path, AppSettings,
};

/// Maximum accepted size for the `key` argument to `set_setting`. Dotted
/// keys are short (section + field name); 256 bytes leaves substantial
/// headroom without letting a compromised webview drive unbounded allocations.
pub(crate) const MAX_SETTING_KEY_BYTES: usize = 256;

/// Maximum accepted size for the `value` argument to `set_setting` when
/// serialized back to JSON. Settings values are scalars today; even the
/// longest foreseeable string (a path) is bounded by MAX_WORKSPACE_ROOT_BYTES.
/// 16 KiB is generous enough for any reasonable future array field while
/// still blocking large-payload abuse.
pub(crate) const MAX_SETTING_VALUE_BYTES: usize = 16 * 1024;

/// `level` argument for `set_setting`. Kept a typed enum (instead of a
/// raw string) so serde rejects typos at the IPC boundary and the reader
/// cannot construct an ambiguous tier.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsLevel {
    User,
    Workspace,
}

#[tauri::command]
pub async fn get_settings<R: Runtime>(
    workspace_root: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<AppSettings, String> {
    // Same authz model as F-036's approval commands: dashboard + any
    // session-* window may read.
    require_window_label_in(&webview, &["dashboard"], true)?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;

    let user_dir = resolve_user_config_dir(&state);
    load_merged_in(user_dir.as_deref(), std::path::Path::new(&workspace_root))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_setting<R: Runtime>(
    key: String,
    value: serde_json::Value,
    level: SettingsLevel,
    workspace_root: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label_in(&webview, &["dashboard"], true)?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;
    require_size("key", &key, MAX_SETTING_KEY_BYTES)?;

    // Enforce the value cap off the serialized form — the wire shape is JSON
    // and the eventual on-disk shape is TOML, so this is a conservative
    // upper bound on both.
    let value_str = serde_json::to_string(&value).map_err(|e| e.to_string())?;
    require_size("value", &value_str, MAX_SETTING_VALUE_BYTES)?;

    // JSON -> TOML: serde_json::Value's scalar shapes deserialize cleanly into
    // toml::Value for the types we accept (string, bool, number). Tagged
    // objects land as tables; null is rejected below because toml has no
    // null type and settings never store null.
    let toml_value: toml::Value =
        json_to_toml(value).ok_or_else(|| "value cannot be represented as TOML".to_string())?;

    match level {
        SettingsLevel::Workspace => {
            let root = std::path::Path::new(&workspace_root);
            let path = workspace_settings_path(root);
            let existing = tokio::fs::read_to_string(&path)
                .await
                .unwrap_or_else(|_| String::new());
            let updated =
                apply_setting_update(&existing, &key, toml_value).map_err(|e| e.to_string())?;
            // Write the `apply_setting_update` output verbatim — going back
            // through `AppSettings` + the struct-typed save would promote
            // every `#[serde(default)]` field into the file, erasing the
            // "absent means pick up the other tier / pick up the default"
            // invariant the merge layer relies on.
            save_workspace_settings_raw(root, &updated)
                .await
                .map_err(|e| e.to_string())
        }
        SettingsLevel::User => {
            let dir = resolve_user_config_dir(&state)
                .ok_or_else(|| "could not resolve user config directory".to_string())?;
            let path = forge_core::settings::user_settings_path_in(&dir);
            let existing = tokio::fs::read_to_string(&path)
                .await
                .unwrap_or_else(|_| String::new());
            let updated =
                apply_setting_update(&existing, &key, toml_value).map_err(|e| e.to_string())?;
            save_user_settings_raw_in(&dir, &updated)
                .await
                .map_err(|e| e.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// F-138: stop_background_agent — terminal transition on an instance.
//
// Mirrors `promote_background_agent` for authz + size-gate, but drives
// `Orchestrator::stop(id)` instead of the tracking-set flip. The registry's
// forwarder (set up in `new_bg_session` / `BackgroundAgentRegistry::start`)
// already observes the orchestrator's terminal event and re-emits it as
// `Event::BackgroundAgentCompleted` on the per-session broadcast. The
// session-scoped webview forwarder picks that up and emits
// `session:event` with the `background_agent_completed` shape — the same
// path F-137 pinned — so callers see Badge count flip and the configured
// `notifications.bg_agents` mode fire without a second emit here.
//
// Appended at EOF per the concurrent-worktree convention used by F-137 /
// F-144 / F-151.
// ---------------------------------------------------------------------------

/// Stop a running background agent.
///
/// Errors:
/// - unauthorized window label → `forbidden: window label mismatch`
/// - oversize `instance_id` → size-cap error
/// - unknown instance id → `stop_background_agent: unknown instance`
///
/// Idempotence: calling `stop` twice against the same id returns
/// `unknown instance` on the second call once the forwarder has dropped the
/// row. A concurrent caller racing the forwarder may observe either the
/// `unknown instance` error or the terminal state on `list`, both of which
/// are acceptable terminal observations.
#[tauri::command]
pub async fn stop_background_agent<R: Runtime>(
    session_id: String,
    instance_id: String,
    app: AppHandle<R>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
    bg_state: State<'_, BgAgentState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("instance_id", &instance_id, MAX_AGENT_INSTANCE_ID_BYTES)?;

    let entry = resolve_bg_session(&app, &state, &bg_state, &session_id).await?;
    let id = forge_core::AgentInstanceId::from_string(instance_id);
    entry
        .registry
        .orchestrator()
        .stop(&id)
        .await
        .map_err(|e| format!("stop_background_agent: {e}"))?;
    Ok(())
}

/// Convert a `serde_json::Value` into a `toml::Value`. Returns `None` for
/// JSON shapes TOML cannot represent (null; numbers that aren't finite).
/// Arrays and objects convert recursively; other scalars map 1:1.
fn json_to_toml(v: serde_json::Value) -> Option<toml::Value> {
    match v {
        serde_json::Value::Null => None,
        serde_json::Value::Bool(b) => Some(toml::Value::Boolean(b)),
        serde_json::Value::String(s) => Some(toml::Value::String(s)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(toml::Value::Integer(i))
            } else {
                n.as_f64().filter(|f| f.is_finite()).map(toml::Value::Float)
            }
        }
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(json_to_toml)
            .collect::<Option<Vec<_>>>()
            .map(toml::Value::Array),
        serde_json::Value::Object(map) => {
            let mut out = toml::value::Table::new();
            for (k, v) in map {
                out.insert(k, json_to_toml(v)?);
            }
            Some(toml::Value::Table(out))
        }
    }
}

// ---------------------------------------------------------------------------
// F-145: branch-variant deletion.
//
// Mirrors F-144's `select_branch` shape: session-scoped authz, size-capped
// `parent_id`, delegation to `bridge::delete_branch`. The daemon resolves the
// target and either emits `Event::BranchDeleted { parent, variant_index }`
// or — for a root-with-siblings delete — rejects the request and logs. The
// Tauri command returns `Ok(())` once the IPC frame is written; the outcome
// arrives through the event stream.
//
// Appended at the bottom of the file per the concurrent-worktree convention
// (F-137 / F-144 / F-126 / F-151 have all extended this file in parallel;
// keeping new additions at EOF minimizes rebase conflicts). Also registered
// in `build_invoke_handler` at the top of this file.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn delete_branch<R: Runtime>(
    session_id: String,
    parent_id: String,
    variant_index: u32,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("parent_id", &parent_id, MAX_MESSAGE_ID_BYTES)?;
    state
        .bridge
        .delete_branch(&session_id, parent_id, variant_index)
        .await
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// F-155: MCP Tauri commands — thin UDS wrappers over `SessionBridge`.
//
// F-132 shipped the command surface but ran two independent `McpManager`
// instances (one in the shell, one in the daemon). F-155 unifies them:
// the daemon now owns the single authoritative manager, and the shell's
// Tauri commands dispatch over the session's UDS bridge to reach it. A
// toggle issued from the UI mutates the same `McpManager` the session
// dispatcher consults for live tool calls, so the running-session
// correctness gap called out in the F-155 issue is closed.
//
// Surface:
//
// - `list_mcp_servers(session_id) -> Vec<McpServerInfo>` — sends
//   `IpcMessage::ListMcpServers` and returns the daemon's snapshot.
// - `toggle_mcp_server(session_id, name, enabled) -> McpToggleResult` —
//   sends `IpcMessage::ToggleMcpServer { name, enabled }`. Daemon
//   `enable`s (starts) or `disable`s (parks in `ServerState::Disabled`)
//   its authoritative manager. `McpStateEvent` transitions arrive on the
//   session event log as `Event::McpState` through the normal pipeline.
// - `import_mcp_config(session_id, source, apply) -> McpImportResult` —
//   sends `IpcMessage::ImportMcpConfig`. Daemon converts the third-party
//   config, merges on top of `<workspace>/.mcp.json`, and (when
//   `apply=true`) rewrites the file. `apply=false` is a dry-run.
//
// Authz uses `require_window_label(&webview, &format!("session-{id}"))`
// — MCP config is now a session-scoped operation (it consults the
// session's daemon) so only that session's window may drive it. This is
// a tightening compared with F-132's dashboard/session multi-label authz;
// callers that need the dashboard to reach MCP data can run the command
// through a session window (the dashboard already opens one per active
// session for event forwarding).
//
// The F-132 `mcp:state` Tauri event emitter is retired — `session:event`
// already carries `Event::McpState` for the webview.
// ---------------------------------------------------------------------------

/// Max accepted byte length for an MCP server `name` argument. Server names
/// live as JSON-object keys in `.mcp.json`; a sane ceiling well above any
/// realistic Cursor / Claude / VS Code config.
pub(crate) const MAX_MCP_SERVER_NAME_BYTES: usize = 256;

/// Max accepted byte length for a source slug on `import_mcp_config`.
/// Slugs are short (`vscode`, `cursor`, `claude`, ...); cap defensively.
pub(crate) const MAX_MCP_SLUG_BYTES: usize = 64;

/// List every MCP server the session daemon has configured, with its
/// current lifecycle state and cached tool list.
///
/// Dispatches `IpcMessage::ListMcpServers` over the session's UDS bridge
/// — the shell no longer runs its own `McpManager` (the "two independent
/// managers" bug F-132 inherited). The session daemon's
/// `McpManager::list()` snapshot arrives as
/// `IpcMessage::McpServersList` and is returned verbatim.
#[tauri::command]
pub async fn list_mcp_servers<R: Runtime>(
    session_id: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<Vec<forge_ipc::McpServerInfo>, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    state
        .bridge
        .list_mcp_servers(&session_id)
        .await
        .map_err(|e| e.to_string())
}

/// Toggle an MCP server on or off for the session's authoritative
/// manager.
///
/// `enabled` is the target state — `true` starts (or no-ops if already
/// running), `false` parks the server in `ServerState::Disabled` so
/// in-flight and subsequent tool calls to that server surface the
/// canonical `"MCP server <name> is disabled"` error. The daemon emits
/// the corresponding `McpStateEvent` through `state_stream()` →
/// `Event::McpState` on the session event log, so the webview sees the
/// transition through the normal `session:event` pipeline without
/// polling `list_mcp_servers`.
#[tauri::command]
pub async fn toggle_mcp_server<R: Runtime>(
    session_id: String,
    name: String,
    enabled: bool,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<forge_ipc::McpToggleResult, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("name", &name, MAX_MCP_SERVER_NAME_BYTES)?;
    state
        .bridge
        .toggle_mcp_server(&session_id, name, enabled)
        .await
        .map_err(|e| e.to_string())
}

/// Import an MCP server list from a third-party tool's config into the
/// workspace's universal `.mcp.json`.
///
/// `source` is one of the slugs accepted by
/// `forge_mcp::import::ImportSource::from_slug` (`vscode | cursor |
/// claude | continue | kiro | codex`). `apply=false` is a dry-run —
/// the daemon computes the merged server set and returns it without
/// rewriting the file; the UI can show a confirmation diff before
/// calling again with `apply=true`.
#[tauri::command]
pub async fn import_mcp_config<R: Runtime>(
    session_id: String,
    source: String,
    apply: bool,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<forge_ipc::McpImportResult, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("source", &source, MAX_MCP_SLUG_BYTES)?;
    state
        .bridge
        .import_mcp_config(&session_id, source, apply)
        .await
        .map_err(|e| e.to_string())
}
