//! MCP lifecycle state types shared across the IPC boundary.
//!
//! F-155 moved these out of `forge-mcp` so `forge_core::Event` can carry an
//! `McpState(McpStateEvent)` variant without creating a forge-core →
//! forge-mcp dependency cycle. `forge-mcp` re-exports them unchanged, so
//! external callers still reach for `forge_mcp::ServerState` /
//! `forge_mcp::McpStateEvent`.

use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Lifecycle state of one MCP server, as seen by consumers of
/// `McpManager::state_stream` and `McpManager::list`.
///
/// `Disabled` (F-155) is distinct from `Failed { reason: "stopped" }` — it
/// marks an explicit user toggle-off so the manager's `call()` path can
/// surface the literal error text `"MCP server <name> is disabled"` that
/// the running-session toggle test asserts against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ServerState {
    /// Spawn/connect in progress (transport connect → `initialize`
    /// handshake not yet complete).
    Starting,
    /// Healthy and responsive — last health-check succeeded.
    Healthy,
    /// A health-check failed once. The manager will restart after
    /// backoff; subsequent repeated failures inside the window
    /// transition to [`ServerState::Failed`].
    Degraded { reason: String },
    /// Terminal until the user re-enables the server — restart window
    /// exhausted or the server crashed beyond policy.
    Failed { reason: String },
    /// F-155: explicitly disabled by the user (toggle-off). Distinct
    /// from `Failed` so `McpManager::call` can surface the canonical
    /// "server disabled" error string and so `toggle_mcp_server(name,
    /// true)` knows to transition back through `Starting`.
    Disabled { reason: String },
}

/// A record emitted on the state stream when a server transitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct McpStateEvent {
    /// Server name, matches the key in the loaded spec map.
    pub server: String,
    /// New state the server transitioned to.
    pub state: ServerState,
    /// Wall-clock timestamp. Included so the UI can order events
    /// coming from multiple servers even when the stream lags. ts-rs:
    /// emitted as `unknown` so the frontend treats the field opaquely
    /// and reads it as a monotonic ordering key.
    #[ts(type = "unknown")]
    pub ts: SystemTime,
}
