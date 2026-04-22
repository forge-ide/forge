//! F-371: central tracing field-name schema for `forge-session` and
//! `forge-shell`.
//!
//! Every structured log emitted from those two crates uses a field name
//! exported by this module. Pinning the names in one place keeps the
//! schema consistent across crates so operators can filter reliably
//! (e.g. `| jq 'select(.session_id == "…")'`) without having to know
//! which crate produced the event.
//!
//! # Target naming
//!
//! Targets follow the `forge_<crate>::<module>` scheme already established
//! by `forge_mcp::*` / `forge_lsp::*` and extended by `forge_agents::*`
//! (F-373). Concretely:
//!
//! - `forge_shell::ipc::authz` — label-gate rejection in
//!   `require_window_label` / `require_window_label_in`
//! - `forge_shell::ipc` — Tauri emit failures (session, terminal, LSP)
//! - `forge_session::server` — connection loop, turn errors, archive
//! - `forge_session::mcp` — `.mcp.json` load + server start
//! - `forge_session::bg_agents` — background-agent lifecycle transitions
//!
//! # Field names
//!
//! Prefer the constants below to raw string literals at emission sites so
//! a rename (e.g. `session_id` → `session`) ripples through `cargo check`
//! instead of silently diverging. Field semantics:
//!
//! - [`SESSION_ID`] — the daemon-side session id (`SessionId` string form)
//! - [`WINDOW_LABEL`] — the calling webview's label (`session-<id>` or `dashboard`)
//! - [`COMMAND`] — the Tauri command name for authz rejections
//! - [`EXPECTED`] / [`ACTUAL`] — the label an authz gate wanted vs. what it got
//! - [`TERMINAL_ID`] — PTY identifier in `forge_term` commands
//! - [`SERVER_ID`] — LSP server id (F-353) or MCP server name
//! - [`INSTANCE_ID`] — `AgentInstanceId` for background-agent lifecycle
//! - [`AGENT_NAME`] — `AgentDef.name` (matches F-373's `forge-agents` schema)

/// Daemon-side session id. Threaded into every `forge-session` emission
/// that happens in the context of one session.
pub const SESSION_ID: &str = "session_id";

/// Calling webview label (`session-<id>` or `dashboard`). Present on every
/// `forge-shell` emit-failure log so the operator can identify the window
/// whose event sink rejected the payload.
pub const WINDOW_LABEL: &str = "window_label";

/// Tauri command name on the authz-rejection path (F-051).
pub const COMMAND: &str = "command";

/// Label an authz gate required.
pub const EXPECTED: &str = "expected";

/// Label the authz gate actually observed on the webview.
pub const ACTUAL: &str = "actual";

/// PTY identifier from `forge_term` (F-125).
pub const TERMINAL_ID: &str = "terminal_id";

/// LSP server id (F-353) or MCP server name. Both surfaces log under
/// `server_id` so a downstream filter works uniformly.
pub const SERVER_ID: &str = "server_id";

/// `AgentInstanceId` for background-agent and orchestrator lifecycle logs.
/// Matches the field `forge-agents` emits under `forge_agents::orchestrator`
/// (F-373), so joining shell-side and daemon-side records is a single
/// field match.
pub const INSTANCE_ID: &str = "instance_id";

/// `AgentDef.name` (e.g. `"writer"`, `"reviewer"`).
pub const AGENT_NAME: &str = "agent_name";
