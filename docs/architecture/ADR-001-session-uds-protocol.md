# ADR-001: Session UDS Protocol

**Status:** Accepted  
**Date:** 2026-04-17

---

## Context

Forge spawns one `forged` session process per workspace. The GUI shell (`forge-shell`), CLI (`forge`), and any future remote client need a stable, versioned contract for attaching to a running session, replaying history, and exchanging messages in real time.

This ADR captures all decisions made for the Boundary 2 IPC layer (see `ipc-contracts.md §4.2`).

---

## Decisions

### 1. Transport: Unix domain sockets

**Decision.** Use Unix domain sockets (stream) on macOS and Linux. Use named pipes (`\\.\pipe\forge-sessions-<id>`) on Windows native (v1.3+); WSL uses UDS.

**Rationale.** UDS avoids TCP port allocation and provides OS-level access control without additional configuration. Named pipes offer an equivalent abstraction on Windows with similar security properties.

---

### 2. Framing: length-prefixed JSON

**Decision.** Every message is framed as `[u32 big-endian byte length][UTF-8 JSON body]`. Maximum frame size is **4 MiB**; frames exceeding this limit cause the session to close the connection and log the rejection.

**Rationale.** Length-prefix framing is unambiguous and cheap to parse — no delimiter scanning, no escape handling. JSON keeps the protocol human-readable and debuggable without an additional schema compiler. The 4 MiB cap prevents memory exhaustion from malformed or malicious clients.

---

### 3. Handshake and protocol version negotiation

**Decision.** On connect, the client sends a `Hello` message carrying `proto` (integer protocol version) and a `ClientIdentity`. The session responds with `HelloAck` containing `session_id`, `workspace`, `started_at`, `event_seq`, and `schema_version`. The client then sends a `Subscribe` message specifying replay semantics.

```json
// Client → session
{"t":"Hello","proto":1,"client":{"kind":"shell","pid":12345,"user":"alice"}}

// Session → client
{"t":"HelloAck","session_id":"a3f1b2c4","workspace":"/home/alice/code/acme-api","started_at":"2026-04-15T14:22:00Z","event_seq":1842,"schema_version":1}

// Client → session (three valid forms)
{"t":"Subscribe","since":1842}   // live only — no replay
{"t":"Subscribe","since":0}      // full replay + live
{"t":"Subscribe","since":1500}   // catch-up from seq 1500
```

**Rationale.** Including `proto` in `Hello` allows the session to reject incompatible clients with a clear error before any other traffic. Returning `event_seq` in `HelloAck` lets clients choose their subscription window without a separate round-trip.

---

### 4. Message types

**Decision.** All messages use a discriminated union with `t` as the tag field.

**Client → session:** `Hello`, `Subscribe`, `Unsubscribe`, `SendUserMessage`, `StopStream`, `RerunMessage`, `SelectBranch`, `CompactTranscript`, `ApproveToolCall`, `RejectToolCall`, `RevokeWhitelist`, `StartBackgroundAgent`, `ReadFile`, `WriteFile`, `ListTree`, `Tick`

**Session → client:** `HelloAck`, `Event`, `StateChanged`, `FileContent`, `Tree`, `Error`, `Ack`

`Tick` serves as a keepalive. `Ack` echoes a `corr` (correlation id) so callers can match responses to requests.

> **Implementation note.** The current `crates/forge-ipc` implements a subset of these types: `Hello`, `HelloAck`, `Subscribe`, `Event`, `SendUserMessage`, `ToolCallApproved`, `ToolCallRejected`. All remaining types are reserved; they appear in the spec as the full intended contract and will be implemented in subsequent milestones.

---

### 5. Event log persistence and the schema header convention

**Decision.** Events are written to `.forge/sessions/<id>/events.jsonl`. **The first line of every `.jsonl` file must be the schema header:**

```json
{"schema_version":1}
```

Every event receives a monotonic `seq` integer and is persisted **before** it is broadcast to clients. On restart the session replays from the log to recompute in-memory state.

Periodic snapshots (every 500 events or 5 minutes) are written to `snapshots/<seq>.msgpack` to accelerate replay. Snapshots are an optimization — correctness does not depend on them.

**Rationale.** Prepending a schema header to each file makes the format self-describing. Any reader can detect the schema version without consulting external metadata, and migrations can be applied before the first event is parsed. Writing before broadcast ensures no event is lost if the process crashes between persist and send.

---

### 6. Schema versioning and migrations

**Decision.** Forge refuses to open a `.jsonl` file without a recognized `schema_version` in the header. When the file's schema is below the current version, migration functions registered in `forge-core::migrations` run at session open before any events are replayed.

**Rationale.** Schema-header-gated migrations prevent silent data corruption. Registering migrations in code (rather than SQL or external scripts) keeps them colocated with the types they transform and enforces the migration path at compile time.

---

### 7. Multi-client semantics

**Decision.** Multiple clients may attach to the same session simultaneously (e.g., the GUI and `forge session tail`). Any client may send commands. The session records `ClientIdentity` alongside each resulting event. Conflicting commands resolve last-write-wins with a **50ms coalescing window** for identical approval messages.

**Rationale.** Broadcast architecture is simpler than session multiplexing and sufficient for the expected cardinality (2–3 clients). Coalescing duplicate approvals within 50ms prevents double-execution when the GUI and CLI send the same approval in quick succession.

---

## Consequences

- The protocol version (`proto`) must be incremented for any breaking change to the handshake or framing.
- The `schema_version` header must be incremented for any breaking change to event shape, and a migration function registered before the version is shipped.
- All IPC types live in `crates/forge-ipc/src/session.rs` and derive `Serialize`, `Deserialize`, and `TS` for TypeScript generation.
- The 4 MiB frame limit is a hard contract; callers must chunk large file payloads.
