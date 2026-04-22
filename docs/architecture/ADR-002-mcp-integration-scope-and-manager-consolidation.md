# ADR-002: MCP Integration — Scope and Manager Consolidation

**Status:** Accepted
**Date:** 2026-04-21

---

## Context

Phase 2 introduced first-class MCP server lifecycle management into `forged`. Three decisions made under F-155 set the shape of that integration and are load-bearing for every downstream consumer (the session dispatcher, the IPC event stream, the GUI state pills, the CLI `forge mcp` commands).

This ADR records them so future contributors do not re-litigate the layering, re-split the manager, or drop `Disabled` as "just another failure mode."

Cross-references: `crate-architecture.md §3.3` (forge-mcp responsibilities), `ipc-contracts.md` (the `Event::McpState` wire form).

---

## Decisions

### 1. MCP lifecycle types live in `forge-core`, not `forge-mcp`

**Decision.** `ServerState` and `McpStateEvent` are defined in `crates/forge-core/src/mcp_state.rs` and re-exported unchanged from `forge-mcp` so external callers keep reading `forge_mcp::ServerState` / `forge_mcp::McpStateEvent`.

**Rationale.** `forge_core::Event` needs an `McpState(McpStateEvent)` variant (see the `Event` enum in `crate-architecture.md §3.1`) because the session event log is the single source of truth for every observable session transition and MCP lifecycle transitions have to ride the same log. If the types lived in `forge-mcp`, `forge-core` would need a `forge-mcp` dependency to construct the variant — but `forge-mcp` already depends on `forge-core` for the shared type vocabulary, which is a cycle. Moving the two enums up the stack into `forge-core` breaks the cycle without forcing `forge-mcp` callers to re-import from a new path. The re-export preserves the existing public surface.

See `crates/forge-core/src/mcp_state.rs:1-8` for the module-level doc that records this rationale inline.

---

### 2. Single `McpManager` per daemon

**Decision.** `forged` instantiates exactly one `McpManager` at startup; every session and every agent orchestrator shares that instance via an `Arc<McpManager>`. There is no per-session manager, no per-agent manager, and no per-scope manager.

**Rationale.** MCP servers are OS processes (stdio transport) or HTTP endpoints (http transport) with non-trivial spawn cost and stateful handshakes. A per-session manager would:

- re-spawn the same server for every session in the workspace, multiplying process count by session count
- fight itself on the user-scope `~/.mcp.json` — two managers trying to own the same stdio pipe
- make the restart-policy bookkeeping (5 tries per 10 minutes, see `§3.3`) per-session instead of per-server, which does not match user intent

A single manager per daemon lets servers be shared across sessions, keeps restart accounting server-scoped, and matches the mental model of "the MCP servers for this workspace" rather than "the MCP servers for this session."

Scope separation — workspace vs. user-scope servers — is handled inside the manager via `Scope`, not by instantiating multiple managers.

---

### 3. `ServerState::Disabled` is distinct from `Failed`

**Decision.** `ServerState` has five variants: `Starting`, `Healthy`, `Degraded`, `Failed`, `Disabled`. `Disabled` is not modelled as `Failed { reason: "stopped" }` or `Failed { reason: "disabled" }` — it is its own variant.

**Rationale.** `Disabled` carries semantics that `Failed` cannot:

- **User intent.** `Disabled` means the user explicitly toggled the server off. `Failed` means the runtime gave up on it (restart window exhausted, crash beyond policy). Collapsing both into `Failed` erases the intent signal the UI needs to render them differently (a disabled toggle vs. a red error banner).
- **Error string contract.** `McpManager::call` on a disabled server must return the literal string `"MCP server <name> is disabled"`. The running-session toggle test asserts against that exact text so the CLI and GUI render a consistent message. `Failed` servers produce transport-specific error strings; reusing that path would break the contract.
- **Re-enable transition.** `toggle_mcp_server(name, true)` needs to know whether to transition `Disabled → Starting` (the re-enable path) or to leave `Failed` alone (the user must explicitly restart a crashed server). A single discriminant on the state enum makes this an exhaustive match instead of a string parse on `reason`.

See `crates/forge-core/src/mcp_state.rs:14-42` for the variant-level doc comments that pin each of these semantics to the enum definition.

---

## Consequences

- `forge-mcp` keeps its public surface (`pub use forge_core::mcp_state::*`) — existing callers do not move.
- Every `Event::McpState` in the event log is constructed in `forge-core` code paths; `forge-mcp` produces values but the enum definition stays upstream.
- Adding a new `ServerState` variant is a breaking change to the TS-generated type and must bump the IPC protocol version per `ADR-001 §3`.
- A future multi-tenant daemon (multiple workspaces served by one `forged`) would need to revisit decision 2 — but that scope expansion is not currently planned.
- `toggle_mcp_server(name, false)` writes `Disabled` to the state stream; it does not shadow-delete the server from `forge_mcp::McpManager::list()`. The list entry stays visible so the GUI can render the "off" toggle next to the server name.
