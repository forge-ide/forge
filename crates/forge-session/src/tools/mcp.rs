//! F-132: MCP server tool adapter.
//!
//! Wraps a single namespaced MCP tool (`"<server>.<tool>"`) behind the
//! `Tool` trait so the session dispatcher can invoke it alongside the
//! built-in `fs.*` / `shell.exec` / `agent.spawn` handlers. The server
//! name and unqualified tool name are split once at registration; the
//! adapter keeps an `Arc<McpManager>` and calls `mgr.call(server, tool,
//! args)` on every invoke.
//!
//! `approval_preview` renders a one-line summary that identifies the
//! server explicitly (`"MCP {server}.{tool}"`) so the approval prompt is
//! distinguishable from built-ins. `read_only` reflects the MCP
//! `readOnlyHint` annotation the manager already extracted when it
//! cached the tool list — pass-through, no re-inspection here.

use std::sync::Arc;

use async_trait::async_trait;
use forge_core::ApprovalPreview;
use forge_mcp::McpManager;
use serde_json::{json, Value};

use crate::tools::{Tool, ToolCtx};

/// One `Tool` per MCP-advertised tool. Registered into the session
/// dispatcher at turn-start from `McpManager::list()` — we snapshot the
/// tools at the beginning of each turn so a mid-turn `tools/list`
/// refresh cannot change the dispatch table under the running loop.
pub struct McpTool {
    /// Server name (the key in `.mcp.json`).
    server: String,
    /// Unqualified tool name as the MCP server advertises it.
    tool: String,
    /// The fully-namespaced name (`"<server>.<tool>"`) — what providers
    /// see on the wire and what `ToolDispatcher` keys on. Pre-computed
    /// so `name()` is a `&str` on an owned field.
    full_name: String,
    /// One-line description from `tools/list`.
    description: String,
    /// `readOnlyHint` from the MCP `annotations` object. Missing
    /// annotation defaults to `false` — see
    /// `forge_mcp::manager::parse_tools_list`.
    read_only: bool,
    /// Shared manager handle. Cheap to clone (the manager itself is
    /// `Arc`-wrapped internally), so per-tool registration is zero-cost.
    manager: Arc<McpManager>,
}

impl McpTool {
    /// Construct an adapter for a single namespaced MCP tool. The
    /// `full_name` must be `"<server>.<tool>"`; panics are avoided by
    /// returning `None` for a name with no dot. Kept fallible (rather
    /// than asserting) so a future bad tools/list frame cannot crash
    /// the session.
    pub fn new(
        full_name: String,
        description: String,
        read_only: bool,
        manager: Arc<McpManager>,
    ) -> Option<Self> {
        let (server, tool) = full_name.split_once('.')?;
        Some(Self {
            server: server.to_string(),
            tool: tool.to_string(),
            full_name: full_name.clone(),
            description,
            read_only,
            manager,
        })
    }
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.full_name
    }

    fn approval_preview(&self, _args: &Value) -> ApprovalPreview {
        // Keep the preview terse and identify the source so the user
        // can distinguish an MCP-sourced prompt from a built-in one.
        // The description text is intentionally short — the approval UI
        // renders the full args payload separately.
        let summary = if self.description.is_empty() {
            format!("MCP {}", self.full_name)
        } else {
            format!("MCP {}: {}", self.full_name, self.description)
        };
        ApprovalPreview {
            description: summary,
        }
    }

    fn read_only(&self) -> bool {
        self.read_only
    }

    async fn invoke(&self, args: &Value, _ctx: &ToolCtx) -> Value {
        // The manager surfaces JSON-RPC errors as `Err`; uniformize into
        // the `{ "error": "..." }` shape the session-turn loop uses to
        // decide `StepOutcome::Error`, so MCP errors are indistinguishable
        // from built-in tool errors downstream. Success-path payloads are
        // passed through verbatim (they're already JSON values).
        match self
            .manager
            .call(&self.server, &self.tool, args.clone())
            .await
        {
            Ok(v) => v,
            Err(err) => json!({ "error": format!("mcp: {err:#}") }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_name_without_dot() {
        let mgr = Arc::new(McpManager::new(Default::default()));
        assert!(McpTool::new("no-dot".into(), String::new(), false, mgr).is_none());
    }

    #[test]
    fn new_splits_server_and_tool() {
        let mgr = Arc::new(McpManager::new(Default::default()));
        let tool =
            McpTool::new("github.create_issue".into(), "desc".into(), true, mgr).expect("parse");
        assert_eq!(tool.server, "github");
        assert_eq!(tool.tool, "create_issue");
        assert_eq!(tool.name(), "github.create_issue");
        assert!(tool.read_only());
        assert!(tool
            .approval_preview(&json!({}))
            .description
            .contains("github.create_issue"));
    }

    #[test]
    fn new_handles_dotted_tool_names() {
        // MCP tool names may themselves contain dots; the split is only
        // at the *first* dot — everything after belongs to the server.
        let mgr = Arc::new(McpManager::new(Default::default()));
        let tool =
            McpTool::new("srv.nested.tool.name".into(), String::new(), false, mgr).expect("parse");
        assert_eq!(tool.server, "srv");
        assert_eq!(tool.tool, "nested.tool.name");
    }

    #[tokio::test]
    async fn invoke_without_running_server_surfaces_error_shape() {
        // No servers configured — `call` fails with an "unknown MCP
        // server" error; the adapter must wrap it into the canonical
        // `{ "error": "..." }` envelope so downstream step-outcome
        // classification treats it identically to a built-in tool error.
        let mgr = Arc::new(McpManager::new(Default::default()));
        let tool = McpTool::new("missing.tool".into(), String::new(), false, mgr).expect("parse");
        let ctx = ToolCtx::default();
        let v = tool.invoke(&json!({}), &ctx).await;
        assert!(v.get("error").is_some(), "expected error envelope, got {v}");
        let msg = v.get("error").and_then(Value::as_str).unwrap_or_default();
        assert!(msg.contains("mcp:"), "prefix missing: {msg}");
    }
}
