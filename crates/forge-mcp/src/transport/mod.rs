//! Transports carry JSON-RPC 2.0 traffic to a single MCP server.
//!
//! Each transport exposes the same minimal shape: a connect constructor, a
//! `send` for outbound requests/notifications, and a `recv` that yields
//! inbound messages plus a terminal event when the remote end disappears.
//! The [`McpManager`][crate_manager] (F-130) multiplexes these transports
//! across servers; transports themselves are single-connection primitives.
//!
//! Log-field truncation lives in [`forge_core::process::truncate`]; the
//! stdio transport uses it transitively via [`ManagedStdioChild`], and
//! the http transport (`http.rs`) imports it directly for its own
//! malformed-frame / oversize-body warns. The previously local copies
//! were folded out by issue #522.
//!
//! [crate_manager]: https://docs.rs/forge-mcp/latest/forge_mcp/
//! [`ManagedStdioChild`]: forge_core::process::ManagedStdioChild

pub mod http;
pub mod ssrf;
pub mod stdio;

pub use http::{Http, HttpEvent};
pub use stdio::{Stdio, StdioEvent};
