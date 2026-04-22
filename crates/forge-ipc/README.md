# forge-ipc

The shared IPC types and framing used on both Forge boundaries: shell ↔ session over a Unix domain socket, and (via `ts-rs`-exported types in `forge-core`) Tauri ↔ webview. Defines the tagged `IpcMessage` envelope, the `Hello` / `HelloAck` handshake with `PROTO_VERSION` / `SCHEMA_VERSION` advertisement, the client-to-session command set (`SendUserMessage`, `ToolCallApproved`, `ToolCallRejected`, `Subscribe`), and the length-delimited framing helpers (`read_frame`, `write_frame`, `read_frame_with_deadline`) with a shared 4 MiB max frame size.

## Role in the workspace

- Depended on by: `forge-session`, `forge-shell`, `forge-cli` — every component that speaks the session protocol.
- Depends on: `tokio` (net + io-util + time), `serde`, `serde_json`, `anyhow`.

## Key types / entry points

- `PROTO_VERSION`, `SCHEMA_VERSION` — wire-format version constants advertised in the handshake.
- `IpcMessage` — tagged `enum` envelope: `Hello`, `HelloAck`, `Subscribe`, `Event`, `SendUserMessage`, `ToolCallApproved`, `ToolCallRejected`.
- `Hello` / `HelloAck` / `ClientInfo` — the connection handshake payloads (kind, pid, user; session id, workspace, started-at, current event seq, schema version).
- `Subscribe { since: u64 }` — request a replay-then-tail stream from a given event sequence.
- `IpcEvent { seq, event }` — server-to-client event frame.
- `SendUserMessage`, `ToolCallApproved`, `ToolCallRejected` — client-to-server commands.
- `read_frame` / `write_frame` — canonical async helpers over any `AsyncRead` / `AsyncWrite`, capped at 4 MiB. Every production call site and every integration test uses these directly.
- `read_frame_with_deadline` — `read_frame` wrapped in a `tokio::time::timeout`; used on the session accept path so a silent peer cannot stall a daemon task (F-354).

## Further reading

- [Crate architecture — `forge-ipc`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
- [Session UDS protocol (ADR-001)](../../docs/architecture/ADR-001-session-uds-protocol.md)
- [IPC contracts](../../docs/architecture/ipc-contracts.md)
