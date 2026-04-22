//! Transports carry JSON-RPC 2.0 traffic to a single MCP server.
//!
//! Each transport exposes the same minimal shape: a connect constructor, a
//! `send` for outbound requests/notifications, and a `recv` that yields
//! inbound messages plus a terminal event when the remote end disappears.
//! The [`McpManager`][crate_manager] (F-130) multiplexes these transports
//! across servers; transports themselves are single-connection primitives.
//!
//! [crate_manager]: https://docs.rs/forge-mcp/latest/forge_mcp/

pub mod http;
pub mod ssrf;
pub mod stdio;

pub use http::{Http, HttpEvent};
pub use stdio::{Stdio, StdioEvent};

/// Cap a log field at `max` bytes so a runaway frame can't flood the log
/// ring. Input is assumed UTF-8; slicing is done on a char boundary via
/// `char_indices` to avoid panicking on multi-byte glyphs.
///
/// Shared by both stdio and http transports. F-375 collapsed the previously
/// byte-identical copies in `stdio.rs` and `http.rs` into this single site.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    for (i, _) in s.char_indices() {
        if i > max {
            break;
        }
        end = i;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::truncate;

    #[test]
    fn truncate_is_utf8_safe() {
        let s = "a".repeat(600) + "é";
        let out = truncate(&s, 300);
        assert!(out.ends_with('…'));
        // And it must parse as valid UTF-8 (the slice is on a boundary).
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    #[test]
    fn truncate_is_noop_below_cap() {
        let s = "short";
        assert_eq!(truncate(s, 64), "short");
    }
}
