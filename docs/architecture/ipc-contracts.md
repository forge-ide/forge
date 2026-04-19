# IPC Contracts

> Extracted from IMPLEMENTATION.md §4-5 — the two IPC boundaries, Tauri commands, events, UDS framing, handshake, and full message types

---

## 4. IPC contracts

Forge has **two distinct IPC boundaries**. They must not be confused.

```
  ┌──────────────────────┐
  │  Webview (Solid)     │
  │  TypeScript          │
  └──────────┬───────────┘
             │  Boundary 1: Tauri commands + events
             │  (in-process, JSON over Tauri IPC)
  ┌──────────▼───────────┐
  │  forge-shell (Rust)  │
  │  Tauri host          │
  └──────────┬───────────┘
             │  Boundary 2: UDS + length-prefixed JSON
             │  (cross-process, multiple sessions)
  ┌──────────▼───────────┐    ┌───────────────────┐    ┌───────────────────┐
  │ forged session #1    │    │ forged session #2 │    │ forged session #3 │
  └──────────────────────┘    └───────────────────┘    └───────────────────┘
```

### 4.1 Boundary 1: Tauri ↔ webview

> **Phase 1 coverage.** Phase 1 ships the following Tauri commands: `session_hello`, `session_subscribe`, `session_send_message`, `session_approve_tool`, `session_reject_tool`, `session_list`, `open_session`, `provider_status`. Every other command listed below is reserved for subsequent milestones and is not wired in Phase 1. See ADR-001 §4 for the corresponding subset note on the UDS boundary.

**Pattern.** Tauri `command` handlers for request/response (webview → host) and Tauri `events` for push (host → webview).

#### Commands (webview → host)

All commands live in `forge-shell/src/commands.rs`. Types in `forge-ipc/src/tauri.rs` are derived with `ts-rs` and regenerated into `web/packages/ipc/src/generated.ts`.

```rust
// Session lifecycle
#[tauri::command] async fn list_sessions(filter: SessionFilter) -> Result<Vec<SessionSummary>, IpcError>;
#[tauri::command] async fn open_session(workspace: PathBuf, target: SessionTarget, persistence: Option<SessionPersistence>) -> Result<SessionId, IpcError>;
#[tauri::command] async fn attach_session(id: SessionId) -> Result<SessionHandle, IpcError>;
#[tauri::command] async fn detach_session(id: SessionId) -> Result<(), IpcError>;
#[tauri::command] async fn kill_session(id: SessionId) -> Result<(), IpcError>;
#[tauri::command] async fn archive_session(id: SessionId) -> Result<(), IpcError>;  // manual trigger

pub enum SessionTarget {
    Agent(AgentId),
    Provider { id: ProviderId, model: Option<String> },
}
pub enum SessionFilter { Active, Archived, All }

// Message flow
#[tauri::command] async fn send_message(session: SessionId, text: String, context: Vec<ContextRef>, provider_override: Option<ProviderOverride>) -> Result<MessageId, IpcError>;
#[tauri::command] async fn stop_stream(session: SessionId) -> Result<(), IpcError>;
#[tauri::command] async fn rerun_message(session: SessionId, msg: MessageId, variant: RerunVariant, provider_override: Option<ProviderOverride>) -> Result<MessageId, IpcError>;
#[tauri::command] async fn select_branch(session: SessionId, parent: MessageId, variant: MessageId) -> Result<(), IpcError>;
#[tauri::command] async fn compact_transcript(session: SessionId) -> Result<CompactReport, IpcError>;

pub enum RerunVariant { Replace, Branch, Fresh }

// Approval
#[tauri::command] async fn approve_tool_call(session: SessionId, id: ToolCallId, scope: ApprovalScope) -> Result<(), IpcError>;
#[tauri::command] async fn reject_tool_call(session: SessionId, id: ToolCallId, reason: Option<String>) -> Result<(), IpcError>;
#[tauri::command] async fn revoke_whitelist(session: SessionId, pattern: ApprovalScope) -> Result<(), IpcError>;

// Workspace
#[tauri::command] async fn list_workspaces() -> Result<Vec<WorkspaceSummary>, IpcError>;
#[tauri::command] async fn read_file(session: SessionId, path: PathBuf, range: Option<Range>) -> Result<FileContent, IpcError>;
#[tauri::command] async fn tree(session: SessionId, path: PathBuf, depth: u32) -> Result<TreeNode, IpcError>;

// Providers / skills / MCP / agents
#[tauri::command] async fn list_providers() -> Result<Vec<ProviderSummary>, IpcError>;
#[tauri::command] async fn login_provider(id: ProviderId, api_key: SecretString) -> Result<(), IpcError>;
#[tauri::command] async fn list_mcp_servers(scope: Scope) -> Result<Vec<McpServerInfo>, IpcError>;
#[tauri::command] async fn toggle_mcp_server(id: McpId, enabled: bool) -> Result<(), IpcError>;
#[tauri::command] async fn import_mcp_config(source: ImportSource, dest_scope: Scope) -> Result<ImportReport, IpcError>;
#[tauri::command] async fn list_skills(scope: Scope) -> Result<Vec<SkillInfo>, IpcError>;
#[tauri::command] async fn list_agents(scope: Scope) -> Result<Vec<AgentInfo>, IpcError>;

// Background agents
#[tauri::command] async fn start_background_agent(session: SessionId, agent: AgentId, initial_message: String) -> Result<AgentInstanceId, IpcError>;
#[tauri::command] async fn promote_background_agent(session: SessionId, id: AgentInstanceId) -> Result<(), IpcError>;

// Memory (opt-in feature)
#[tauri::command] async fn get_memory_enabled() -> Result<bool, IpcError>;
#[tauri::command] async fn set_memory_enabled(enabled: bool) -> Result<(), IpcError>;
#[tauri::command] async fn read_memory(agent: AgentId) -> Result<Option<String>, IpcError>;

// Usage
#[tauri::command] async fn usage_summary(range: UsageRange, group_by: GroupBy) -> Result<UsageReport, IpcError>;

// Containers
#[tauri::command] async fn list_containers() -> Result<Vec<ContainerSummary>, IpcError>;
#[tauri::command] async fn container_logs(id: ContainerId, tail: u32) -> Result<String, IpcError>;
#[tauri::command] async fn stop_container(id: ContainerId) -> Result<(), IpcError>;

// Settings
#[tauri::command] async fn get_setting(key: String) -> Result<JsonValue, IpcError>;
#[tauri::command] async fn set_setting(key: String, value: JsonValue) -> Result<(), IpcError>;

// Dev server (static HTML preview)
#[tauri::command] async fn preview_start(workspace: PathBuf, entry: Option<PathBuf>) -> Result<PreviewInfo, IpcError>;
#[tauri::command] async fn preview_stop() -> Result<(), IpcError>;
```

**Every command.**
- Is `async`
- Returns `Result<T, IpcError>` where `IpcError` is a tagged enum with display strings
- Has a TS-generated type at `web/packages/ipc/src/generated.ts`
- Is wrapped in a typed client helper in `web/packages/ipc/src/client.ts`

#### Events (host → webview)

Events are emitted via Tauri's `app_handle.emit_all("forge://event", payload)`. Every event has a `kind` discriminator.

```rust
pub enum ShellEvent {
    SessionEvent { session: SessionId, event: SessionEvent },   // see §5
    SessionStateChanged { session: SessionId, state: SessionState },
    ProviderStateChanged { id: ProviderId, state: ProviderState },
    McpStateChanged { id: McpId, state: McpState },
    UsageTick { provider: ProviderId, delta: UsageDelta },
    ContainerStateChanged { id: ContainerId, state: ContainerState },
    BackgroundAgentNotification { session: SessionId, id: AgentInstanceId, kind: BgNotifyKind },
    NotificationPosted { level: Level, title: String, body: Option<String> },
}
```

The webview subscribes with Solid's `createResource` or a dedicated subscription helper.

Events are fire-and-forget; critical state is also fetchable via commands for late-join cases.

### 4.2 Boundary 2: shell ↔ session (UDS)

See §5 for the full protocol. Shell maintains **one UDS connection per open session**. Events from the session are translated into `ShellEvent::SessionEvent` and forwarded to the webview.

### 4.3 Type generation

All IPC types live in `crates/forge-ipc/src/`:
```
forge-ipc/
  src/
    lib.rs       # re-exports
    tauri.rs     # Tauri boundary types
    session.rs   # UDS boundary types (§5)
    common.rs    # shared IDs, enums, primitives
```

Every type that crosses a boundary derives `#[derive(Serialize, Deserialize, TS)]` and `#[ts(export)]`. The script `scripts/gen-ts-types.sh` runs `cargo test` with `ts-rs` flags, then copies the generated `.ts` files into `web/packages/ipc/src/generated/`.

CI fails if generated types drift.

---

## 5. Session process protocol

The UDS protocol between shell and session. This is a firm contract — agents, headless CLI, and future remote session support all depend on it.

### 5.1 Transport

- Unix domain socket (stream) on Mac/Linux
- Named pipes (`\\.\pipe\forge-sessions-<id>`) on Windows (native v1.3; WSL uses UDS)
- **Length-prefixed JSON frames**: `[u32 big-endian length][UTF-8 JSON body]`
- Max frame size: 4 MiB (reject larger; session closes connection and logs)

### 5.2 Handshake

On connect, the **client** (shell, CLI, or other) sends:
```json
{"t":"Hello","proto":1,"client":{"kind":"shell","pid":12345,"user":"alice"}}
```
The **session** responds:
```json
{"t":"HelloAck","session_id":"a3f1b2c4","workspace":"/home/alice/code/acme-api","started_at":"2026-04-15T14:22:00Z","event_seq":1842,"schema_version":1}
```
The client then sends either:
```json
{"t":"Subscribe","since":1842}           // live only
{"t":"Subscribe","since":0}              // full replay + live
{"t":"Subscribe","since":1500}           // catch-up from seq
```

### 5.3 Message types

> **Phase 1 coverage.** Phase 1 implements the following `IpcMessage` variants in `crates/forge-ipc`: `Hello`, `HelloAck`, `Subscribe`, `Event`, `SendUserMessage`, `ToolCallApproved`, `ToolCallRejected`. Every other variant listed below is reserved for subsequent milestones and is not wired in Phase 1. See ADR-001 §4 for the same subset note alongside its rationale.

Full discriminated union, `t` is the tag.

**Client → session:**
```
Hello, Subscribe, Unsubscribe,
SendUserMessage { text, context, provider_override?, branch_parent? },
StopStream,
RerunMessage { msg_id, variant, provider_override? },
SelectBranch { parent, variant },
CompactTranscript,
ApproveToolCall { id, scope },
RejectToolCall { id, reason? },
RevokeWhitelist { scope },
StartBackgroundAgent { agent, initial_message },
ReadFile { path, range? },
WriteFile { path, content },
ListTree { path, depth },
Tick                                   // keepalive
```

**Session → client:**
```
HelloAck, Event { seq, event },       // `event` is a Core::Event from §3.1
StateChanged { state },
FileContent { path, content, sha },
Tree { path, node },
Error { code, message, corr? },
Ack { corr }                           // correlation id echo
```

### 5.4 Event log persistence

- `.forge/sessions/<id>/events.jsonl` is the canonical log
- **First line of every file is the schema header:** `{"schema_version": 1}`
- Every emitted event gets a monotonic `seq` integer; persisted before send
- On restart (including post-archive reactivation), the session replays from the log, recomputing in-memory state
- Periodic snapshots (every 500 events or 5 minutes) go to `snapshots/<seq>.msgpack` to accelerate replay — optimization, not a requirement for correctness
- Clients can subscribe from any `seq`; session streams everything after

### 5.5 Schema versioning and migrations

- The first line schema header governs how the rest of the file is interpreted
- Forge refuses to read a jsonl file without a recognized `schema_version`
- Schema bumps come with migration functions registered in `forge-core::migrations`
- Migrations run at session open when the file's schema is below current

### 5.6 Multi-client semantics

- Multiple shells can attach to one session (the GUI and a `forge session tail` simultaneously)
- Any can send commands; the session logs `ClientIdentity` alongside the resulting event
- Conflicting commands resolve last-write-wins with a 50ms coalescing window for identical approvals
