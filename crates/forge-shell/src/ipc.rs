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
//! return [`LABEL_MISMATCH_ERROR`] and never reach the daemon.
//!
//! **Webview isolation (F-062 / M10 / T5):** the event sink targets a single
//! webview (`session-{session_id}`) instead of broadcasting app-wide. Prior
//! to this fix, every session window (and the dashboard) received every
//! session's events; the trust boundary was enforced client-side in the
//! Solid store. The per-sink `session_id` is bound at construction in
//! `session_subscribe` (already label-authenticated), not re-read from the
//! event payload.

use std::sync::Arc;

use forge_ipc::HelloAck;
use tauri::{AppHandle, Emitter, EventTarget, Manager, Runtime, State, Webview};

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
/// A single command may bind additive validations in the future
/// (e.g. F-069 typed-enum validation on `scope` will layer on top of the
/// size check here).
pub(crate) const MAX_MESSAGE_TEXT_BYTES: usize = 128 * 1024;
pub(crate) const MAX_TOOL_CALL_ID_BYTES: usize = 64;
pub(crate) const MAX_APPROVAL_SCOPE_BYTES: usize = 256;
pub(crate) const MAX_REJECT_REASON_BYTES: usize = 1024;

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
/// `webview-test`-gated [`Self::test_socket_override`] field. The field is
/// absent from production builds entirely.
pub struct BridgeState {
    pub bridge: SessionBridge,
    #[cfg(feature = "webview-test")]
    pub test_socket_override: Option<std::path::PathBuf>,
}

impl BridgeState {
    pub fn new(connections: SessionConnections) -> Self {
        Self {
            bridge: SessionBridge::new(connections),
            #[cfg(feature = "webview-test")]
            test_socket_override: None,
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

/// Test-only constructor for [`AppHandleSink`]. Gated behind the
/// `webview-test` feature so production builds cannot reach into the sink.
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
    scope: String,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // F-068 / L4 (T7): tool_call_id is a short opaque handle; scope is a
    // short enum-ish string. Both are bounded here before F-069's typed-enum
    // validation lands on `scope`.
    require_size("tool_call_id", &tool_call_id, MAX_TOOL_CALL_ID_BYTES)?;
    require_size("scope", &scope, MAX_APPROVAL_SCOPE_BYTES)?;
    state
        .bridge
        .approve_tool(&session_id, tool_call_id, scope)
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

/// Returns a fully-wired invoke handler registering all five session bridge
/// commands. Called from `window_manager::run` when building the Tauri app.
pub fn build_invoke_handler<R: Runtime>() -> Box<dyn Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync>
{
    Box::new(tauri::generate_handler![
        session_hello,
        session_subscribe,
        session_send_message,
        session_approve_tool,
        session_reject_tool,
    ])
}

/// Attach the `BridgeState` to an app builder. Used by `window_manager::run`.
pub fn manage_bridge<R: Runtime>(app: &AppHandle<R>) {
    if app.try_state::<BridgeState>().is_none() {
        app.manage(BridgeState::new(SessionConnections::new()));
    }
}
