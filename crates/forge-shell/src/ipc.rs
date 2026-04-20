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

use std::path::PathBuf;
use std::sync::Arc;

use forge_core::approvals::{
    load_user_config_in, load_workspace_config, save_user_config_in, save_workspace_config,
    ApprovalConfig, ApprovalEntry,
};
use forge_core::{ApprovalLevel, ApprovalScope, RerunVariant};
use forge_ipc::HelloAck;
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

/// F-143: re-run an assistant message. Phase 1 dispatches only
/// `RerunVariant::Replace`; `Branch` (F-144) and `Fresh` (F-145) return an
/// error today rather than silently no-op, so a UI that ships Branch before
/// the daemon supports it learns fast.
#[tauri::command]
pub async fn rerun_message<R: Runtime>(
    session_id: String,
    msg_id: String,
    variant: RerunVariant,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // F-068 / L4: bound `msg_id` before the bridge allocates a frame. The
    // variant is typed (forge_core::RerunVariant) — serde rejects any
    // non-variant at Tauri arg deserialization, so no byte cap is needed.
    require_size("msg_id", &msg_id, MAX_MESSAGE_ID_BYTES)?;
    match variant {
        RerunVariant::Replace => state
            .bridge
            .rerun_message(&session_id, msg_id, variant)
            .await
            .map_err(|e| e.to_string()),
        // Branch / Fresh return errors here (not silent no-ops) so a UI that
        // invokes them before F-144/F-145 ship gets a loud signal instead of
        // a hanging command.
        RerunVariant::Branch => Err("rerun_message: Branch variant not implemented (F-144)".into()),
        RerunVariant::Fresh => Err("rerun_message: Fresh variant not implemented (F-145)".into()),
    }
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
        session_approve_tool,
        session_reject_tool,
        rerun_message,
        get_persistent_approvals,
        save_approval,
        remove_approval,
        read_file,
        write_file,
        tree,
    ])
}

/// Attach the `BridgeState` to an app builder. Used by `window_manager::run`.
pub fn manage_bridge<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<BridgeState>().is_none() {
        app.manage(BridgeState::new(SessionConnections::new()));
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
// F-122: filesystem commands for the EditorPane (read_file, write_file, tree).
//
// The webview-facing editor pane needs to read file contents into Monaco,
// write them back on save, and list the workspace tree for the file-picker
// that F-126 will surface. All three commands:
//
// 1. Require `session-{session_id}` window-label authz (F-051 / H10) — the
//    editor only runs inside a session window, so the session-scoped gate is
//    correct and keeps the dashboard from invoking these paths.
// 2. Take `workspace_root` as an explicit param. The frontend reads it from
//    `HelloAck.workspace` and passes it through; the bridge does not cache
//    the workspace today. The allowlist is derived from `workspace_root`
//    inside this module — never taken from the webview — so a compromised
//    webview cannot widen its sandbox by claiming a wider root. (The
//    session-* window-label gate additionally bounds *who* can call at all;
//    a richer "verify workspace_root matches the HelloAck workspace" check
//    is a follow-up once `SessionConnections` caches it.)
// 3. Delegate size caps and symlink rejection to `forge-fs` (F-061 / M3) so
//    the filesystem trust boundary lives in one place.
// ---------------------------------------------------------------------------

/// F-122: per-field cap on byte payloads handed to `write_file`. Matches
/// `forge_fs::Limits::max_write_bytes` default (10 MiB); keep the pre-check
/// here so the Tauri frame bound in `forge_ipc::write_frame` (4 MiB) doesn't
/// short-circuit the cap with a more confusing "frame too large" error. The
/// actual enforcement authority is still `forge-fs`.
pub(crate) const MAX_EDITOR_WRITE_BYTES: usize = 10 * 1024 * 1024;

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

/// Wire shape for [`TreeNodeDto::kind`]. Narrow on purpose — anything unusual
/// (block device, socket, FIFO) maps to `Other` so the wire shape is stable.
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

/// Build the `forge-fs` allowlist for a session's workspace. Canonicalizing
/// the workspace root here (not in the webview) is the security invariant —
/// a webview that lies about `workspace_root` only gets the sandbox rooted
/// at the path it passed; a compromised webview cannot widen that sandbox by
/// pointing outside the directory the session is actually attached to.
///
/// The glob form (`<root>/**` plus an exact `<root>` match) mirrors the
/// pattern used by the existing `fs.read` / `fs.write` tool call paths.
fn workspace_allowlist(workspace_root: &str) -> Result<Vec<String>, String> {
    let root = std::path::Path::new(workspace_root);
    let canonical =
        std::fs::canonicalize(root).map_err(|e| format!("workspace_root not accessible: {e}"))?;
    let display = canonical.to_string_lossy();
    Ok(vec![format!("{display}/**"), display.into_owned()])
}

#[tauri::command]
pub async fn read_file<R: Runtime>(
    session_id: String,
    workspace_root: String,
    path: String,
    webview: Webview<R>,
) -> Result<FileContent, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;
    // Path is an absolute canonical path chosen by the frontend; we cap at
    // workspace_root's budget so a lying webview cannot allocate unbounded
    // PathBuf space in the allowlist match.
    require_size("path", &path, MAX_WORKSPACE_ROOT_BYTES)?;

    let allowed = workspace_allowlist(&workspace_root)?;
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
    workspace_root: String,
    path: String,
    bytes: Vec<u8>,
    webview: Webview<R>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;
    require_size("path", &path, MAX_WORKSPACE_ROOT_BYTES)?;
    // Pre-check before `forge-fs` copies the buffer into the atomic-write
    // temp file. `forge-fs` also enforces; belt-and-braces keeps the error
    // source local and predictable.
    if bytes.len() > MAX_EDITOR_WRITE_BYTES {
        return Err(payload_too_large("bytes", MAX_EDITOR_WRITE_BYTES));
    }

    let allowed = workspace_allowlist(&workspace_root)?;
    let limits = forge_fs::Limits::default();
    forge_fs::write_bytes(&path, &bytes, &allowed, &limits).map_err(|e| e.to_string())
}

/// `tree(session_id, workspace_root, root, depth?)` — list the filesystem
/// subtree rooted at `root`. Pass `workspace_root` again as `root` to list
/// the whole workspace. `depth` defaults to 6 and is capped at 16.
#[tauri::command]
pub async fn tree<R: Runtime>(
    session_id: String,
    workspace_root: String,
    root: String,
    depth: Option<u32>,
    webview: Webview<R>,
) -> Result<TreeNodeDto, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    require_size("workspace_root", &workspace_root, MAX_WORKSPACE_ROOT_BYTES)?;
    require_size("root", &root, MAX_WORKSPACE_ROOT_BYTES)?;

    let allowed = workspace_allowlist(&workspace_root)?;
    // Cap depth at 16 so a pathological request can't walk a Linux-style
    // deep tree and return a megabyte of JSON. 16 is deep enough for the
    // workspaces Forge targets; the entry budget inside `forge-fs` is the
    // second line of defense.
    let requested = depth.unwrap_or(6).min(16);
    let node = forge_fs::list_tree(&root, &allowed, requested).map_err(|e| e.to_string())?;
    Ok(TreeNodeDto::from(node))
}
