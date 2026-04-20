//! Minimal stub LSP server for the `forge-lsp` stdio transport integration
//! test. Mirrors `forge-mcp/tests/bin/mock_stdio.rs`: a dependency-free
//! line-delimited JSON-RPC echo that the test spawns in place of a real
//! language server (so CI doesn't depend on `rust-analyzer` / `gopls` / etc.
//! being installed — see rule #4 in the task brief).
//!
//! Framing. Real LSP wire uses `Content-Length: N\r\n\r\n<json>`. The
//! `forge-lsp` `MessageTransport` bridge is byte-transparent: it doesn't
//! parse LSP headers, it just shuttles bytes between the iframe's LSP client
//! and the server's stdio. That makes this fixture free to pick any framing
//! both ends agree on. Line-delimited JSON-RPC is enough to prove the
//! round-trip: the test writes a newline-terminated `initialize` request,
//! expects an `InitializeResult`, then writes `shutdown` and expects a clean
//! child exit.
//!
//! Recognised methods:
//!
//! - `initialize` → `{ "capabilities": {}, "serverInfo": {...} }`
//! - `shutdown`   → `{}` followed by an orderly exit(0)
//!
//! Anything else produces a JSON-RPC `-32601` error. The fixture exits on
//! stdin EOF or on `shutdown`; the parent uses drop-on-transport to reap
//! if the test aborts mid-stream.
//!
//! This binary is a *test fixture*, not shipped. Declared in Cargo.toml
//! with `test = false, doc = false` so rustdoc and the default test harness
//! ignore it.

use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            // Malformed frames — skip silently so the transport layer can
            // observe its own error path without the fixture forcing an exit.
            Err(_) => continue,
        };

        let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "capabilities": {},
                    "serverInfo": { "name": "forge-lsp-mock-stdio", "version": "0.0.0" }
                }
            }),
            "shutdown" => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {}
                });
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
                // Orderly exit so the parent reads a clean status.
                break;
            }
            _ => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }),
        };

        if writeln!(out, "{}", response).is_err() {
            break;
        }
        if out.flush().is_err() {
            break;
        }
    }
}
