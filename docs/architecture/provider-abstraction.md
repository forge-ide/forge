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
