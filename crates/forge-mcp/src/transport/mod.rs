//! Transports carry JSON-RPC 2.0 traffic to a single MCP server.
//!
//! Each transport exposes the same minimal shape: a connect constructor, a
//! `send` for outbound requests/notifications, and a `recv` that yields
//! inbound messages plus a terminal event when the remote end disappears.
//! The [`McpManager`][crate_manager] (F-130) multiplexes these transports
//! across servers; transports themselves are single-connection primitives.
//!
//! [crate_manager]: https://docs.rs/forge-mcp/latest/forge_mcp/

pub mod stdio;

pub use stdio::{Stdio, StdioEvent};
