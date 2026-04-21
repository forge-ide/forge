//! Provider-agnostic tool descriptor.
//!
//! A `Tool` is the shape sessions hand to a provider's `ChatRequest` — a
//! name, a human-readable description, and a JSON-Schema describing the
//! tool's input. It is deliberately minimal and free of provider SDK
//! types; each provider impl is responsible for translating it into its
//! own wire format.
//!
//! The `read_only` hint lets the session layer enable parallel tool calls
//! for tool sets that do not mutate external state (MCP servers carry
//! this as an annotation; built-in tools set it at registration time).
//!
//! This type also backs the F-130 MCP tool unification — `forge_mcp`
//! produces `Tool` values from server `tools/list` responses so the
//! session doesn't have to know anything about MCP.

use serde::{Deserialize, Serialize};

/// A callable tool the model can invoke.
///
/// Sessions assemble a list of these from every enabled source — built-in
/// tools and every connected MCP server — and hand the list to a
/// provider. The session also uses `read_only` to decide whether the
/// provider should be allowed to batch tool calls in parallel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tool {
    /// Unique name. For MCP tools we namespace with the server name
    /// (`"<server>.<tool>"`) so two servers exposing the same tool name
    /// do not collide in the session's dispatch table.
    pub name: String,
    /// One-line description shown to the model. Providers may truncate.
    pub description: String,
    /// JSON-Schema for the tool's argument object. Providers forward
    /// this verbatim on their tool-declaration surface.
    pub input_schema: serde_json::Value,
    /// `true` if calling the tool never mutates external state.
    ///
    /// For MCP tools this reflects the server-declared `readOnlyHint`
    /// annotation. When the annotation is absent, callers default to
    /// `false` (mutating) — safer than assuming a tool is side-effect
    /// free. Sessions that only include read-only tools in a turn can
    /// allow the provider to fan them out concurrently.
    pub read_only: bool,
}
