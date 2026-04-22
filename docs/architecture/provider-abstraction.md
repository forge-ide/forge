# Provider Abstraction

> Extracted from IMPLEMENTATION.md §6 — unified chat request, provider-specific translation, streaming chunks, and tool classification

---

## 6. Provider abstraction

### 6.1 Unified chat request

```rust
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Message>,
    pub tools: Vec<ToolDef>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub system: Option<String>,                    // workspace AGENTS.md + agent prompt + memory
    pub stop_on_tool: bool,
    pub parallel_tool_calls: bool,                  // true if only read-only tools declared
}

pub struct Message {
    pub role: Role,
    pub blocks: Vec<Block>,
}

pub enum Block {
    Text(String),
    ToolCall { id: ToolCallId, name: String, args: Value },
    ToolResult { id: ToolCallId, result: Value, is_error: bool },
    Reference(ContextRef),
}
```

### 6.2 Provider-specific translation
Each provider implementation has a `translate.rs` module that maps `Block::Reference` and tool defs into the provider's native format, and normalizes the streaming response back into `ChatChunk` variants.

### 6.3 Tool classification

Every tool has a `read_only: bool` bit. Built-in tools (`fs.read`, `fs.list`, `fs.write`, `shell.exec`, `agent.spawn`, etc.) declare this at definition time. MCP tools inherit from the server's `readOnly` hint when advertised; otherwise default to `false` (mutating, safe).

Forge uses this to decide whether a batch of tool calls in one turn can run in parallel.

### 6.4 MCP tools

MCP-advertised tools are adapted to the session `Tool` trait by `McpTool` (`crates/forge-session/src/tools/mcp.rs`). One adapter instance is registered per advertised tool at turn-start from an `McpManager::list()` snapshot, so a mid-turn `tools/list` refresh cannot change the dispatch table under the running loop.

Conventions the adapter enforces — any MCP-specific handling elsewhere in the stack should assume these shapes:

1. **Naming (`<server>.<tool>`, split at the first dot).** The adapter's `name()` returns the fully-namespaced string that providers see on the wire and that `ToolDispatcher` keys on. `McpTool::new` splits once at the first `.` — everything after belongs to the tool, so MCP tool names may themselves contain dots (e.g. `srv.nested.tool.name` → server `srv`, tool `nested.tool.name`). A name with no dot is rejected (returns `None`) rather than panicking.

2. **Adapter location.** `crates/forge-session/src/tools/mcp.rs` — wraps a single namespaced MCP tool behind `Tool`, holds an `Arc<McpManager>`, and delegates every `invoke` to `manager.call(server, tool, args)`.

3. **`readOnlyHint` inheritance.** `read_only()` is a pass-through of the `readOnlyHint` annotation `McpManager` extracts when caching `tools/list` (see `forge_mcp::manager::parse_tools_list`). A missing annotation defaults to `false` (treated as mutating — the conservative choice for parallel-tool-call eligibility). The adapter does no re-inspection.

4. **Approval-preview format.** `approval_preview` emits `"MCP <server>.<tool>: <description>"` (or just `"MCP <server>.<tool>"` when the description is empty) so the approval UI can distinguish an MCP-sourced prompt from a built-in one at a glance. The description is deliberately terse — the approval UI renders the full args payload separately.

5. **Error envelope.** `invoke` uniformizes JSON-RPC failures into `{ "error": "mcp: <detail>" }`. Success-path payloads are passed through verbatim. The envelope shape matches built-in tool errors so the session-turn loop's `StepOutcome::Error` classification treats MCP errors identically to built-in errors downstream.
