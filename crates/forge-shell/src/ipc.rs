//! Tauri command surface for the session IPC bridge.
//!
//! Every command is a thin wrapper over [`crate::bridge::SessionBridge`],
//! plus an [`EventSink`] implementation that forwards payloads to the
//! webview via `AppHandle::emit("session:event", …)`.

use std::path::PathBuf;
use std::sync::Arc;

use forge_ipc::HelloAck;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};

use crate::bridge::{EventSink, SessionBridge, SessionConnections, SessionEventPayload};

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
pub async fn session_hello(
    session_id: String,
    socket_path: Option<String>,
    state: State<'_, BridgeState>,
) -> Result<HelloAck, String> {
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
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    let sink: Arc<dyn EventSink> = Arc::new(AppHandleSink { app });
    state
        .bridge
        .subscribe(&session_id, since.unwrap_or(0), sink)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_send_message(
    session_id: String,
    text: String,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    state
        .bridge
        .send_message(&session_id, text)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_approve_tool(
    session_id: String,
    tool_call_id: String,
    scope: String,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
    state
        .bridge
        .approve_tool(&session_id, tool_call_id, scope)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn session_reject_tool(
    session_id: String,
    tool_call_id: String,
    reason: Option<String>,
    state: State<'_, BridgeState>,
) -> Result<(), String> {
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
