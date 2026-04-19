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

use std::path::PathBuf;
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
pub struct BridgeState {
    pub bridge: SessionBridge,
}

impl BridgeState {
    pub fn new(connections: SessionConnections) -> Self {
        Self {
            bridge: SessionBridge::new(connections),
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
    socket_path: Option<String>,
    webview: Webview<R>,
    state: State<'_, BridgeState>,
) -> Result<HelloAck, String> {
    require_window_label(&webview, &format!("session-{session_id}"))?;
    // TODO(F-052): validate socket_path here — see issue #94.
    let socket_path = socket_path.as_deref().map(PathBuf::from);
    state
        .bridge
        .hello(&session_id, socket_path.as_deref())
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
