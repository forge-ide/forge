//! Tauri command surface for the session IPC bridge.
//!
//! Every command is a thin wrapper over [`crate::bridge::SessionBridge`],
//! plus an [`EventSink`] implementation that forwards payloads to the
//! webview via `AppHandle::emit("session:event", …)`.
//!
//! **Authorization (F-051 / H10):** each session command requires the
//! calling webview's label to equal `format!("session-{session_id}")`.
//! Window labels are set by `window_manager` at window creation and cannot
//! be forged from webview JS, so they serve as the per-window authenticator
//! binding a session's control channel to its review channel. Mismatches
//! return [`LABEL_MISMATCH_ERROR`] and never reach the daemon.

use std::sync::Arc;

use forge_ipc::HelloAck;
use tauri::{AppHandle, Emitter, Manager, Runtime, State, Webview};

use crate::bridge::{EventSink, SessionBridge, SessionConnections, SessionEventPayload};

/// F-051 / H10: structured error returned when the calling webview's label
/// does not match the expected owner for a command's scope. Kept as a plain
/// `String` so it matches every `#[tauri::command]`'s existing `Err(String)`
/// wire shape — never a panic.
pub(crate) const LABEL_MISMATCH_ERROR: &str = "forbidden: window label mismatch";

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

/// Event sink that forwards session events to the webview under the
/// `session:event` event name.
struct AppHandleSink<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> EventSink for AppHandleSink<R> {
    fn emit(&self, payload: SessionEventPayload) {
        if let Err(e) = self.app.emit("session:event", payload) {
            eprintln!("session:event emit failed: {e}");
        }
    }
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
    let sink: Arc<dyn EventSink> = Arc::new(AppHandleSink { app });
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
