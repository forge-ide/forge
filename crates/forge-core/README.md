# forge-core

Canonical types, identifiers, and shared utilities for the Forge workspace. Every other Forge crate depends on this; it depends on nothing Forge-specific. It defines the strongly-typed identifiers used across IPC and persistence (`SessionId`, `WorkspaceId`, `AgentId`, `MessageId`, `ToolCallId`, `ProviderId`), the append-only `Event` model and `EventLog` writer that backs the session log, the workspace/`.forge/` path layout helpers, and the `ForgeError` / `Result` alias every other crate returns.

## Role in the workspace

- Depended on by: `forge-providers`, `forge-session`, `forge-shell`, `forge-cli`, `forge-fs` (transitively), `forge-ipc` consumers, and ultimately every other Forge crate.
- Depends on: only third-party crates (`serde`, `tokio`, `chrono`, `thiserror`, `anyhow`, `ts-rs`, `toml`, `rand`).

## Key types / entry points

- `ids::{SessionId, WorkspaceId, AgentId, AgentInstanceId, MessageId, ToolCallId, ProviderId}` — wire-format identifiers, all `ts-rs`-exported for the TypeScript IPC layer.
- `event::Event` — the append-only event variant emitted to `events.jsonl`; re-exported alongside `ApprovalPreview`, `ApprovalSource`, `ContextRef`, and `EndReason`.
- `event_log::{EventLog, read_since, MAX_LINE_BYTES}` — the bounded NDJSON writer/reader pair used by `forge-session`.
- `transcript::Transcript` — in-memory replay of an event stream.
- `types::{ApprovalScope, CompactTrigger, SessionPersistence, SessionState}` — the small enum vocabulary that flows through approvals, compaction, and session lifecycle.
- `roster::{RosterEntry, RosterScope, ScopedRosterEntry, McpId}` — F-591 catalog wire shapes shared by the `list_*(scope)` IPC commands.
- `workspace` / `workspaces` — workspace root detection and `.forge/` directory management.
- `error::{ForgeError, Result}` — the workspace-wide error type and result alias.

## Further reading

- [Crate architecture — `forge-core`](../../docs/architecture/crate-architecture.md#31-forge-core)
- [IPC contracts](../../docs/architecture/ipc-contracts.md)
