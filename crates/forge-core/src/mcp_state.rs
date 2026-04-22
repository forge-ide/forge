//! MCP lifecycle state types shared across the IPC boundary.
//!
//! F-155 moved these out of `forge-mcp` so `forge_core::Event` can carry an
//! `McpState(McpStateEvent)` variant without creating a forge-core →
//! forge-mcp dependency cycle. `forge-mcp` re-exports them unchanged, so
//! external callers still reach for `forge_mcp::ServerState` /
//! `forge_mcp::McpStateEvent`.
//!
//! F-380 normalized the wire shape:
//! - `ts: SystemTime` → `at: DateTime<Utc>` (matches every other
//!   timestamp-carrying event — single field name across the union).
//! - `ServerState` serde tag `state` → `type` (single discriminator name
//!   across `Event`, `ServerState`, and `StepOutcome`).

use chrono::{DateTime, Utc};
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
#[serde(tag = "type", rename_all = "snake_case")]
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
    /// Wall-clock timestamp (F-380: renamed from `ts: SystemTime`). Serializes
    /// as an RFC3339 string via chrono's `Serialize` impl, matching every
    /// other event timestamp across `forge_core::Event`. ts-rs emits the
    /// field as a plain `string` so the TS mirror carries the same shape.
    #[ts(type = "string")]
    pub at: DateTime<Utc>,
}
