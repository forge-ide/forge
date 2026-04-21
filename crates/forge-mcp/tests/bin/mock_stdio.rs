//! Minimal echo-style MCP server for the stdio transport integration test.
//!
//! Reads line-delimited JSON-RPC 2.0 requests on stdin and writes
//! canned responses on stdout. Recognised methods:
//!
//! - `initialize` -> `{ "capabilities": {} }`
//! - `tools/list` -> `{ "tools": [ { "name": "ping", ... } ] }`
//!
//! Anything else produces a JSON-RPC error with code `-32601`. `ping`
//! method is exposed purely so the test can assert a non-trivial shape.
//!
//! This binary is a *test fixture*, not shipped in any release. It lives
//! under `tests/bin/` so the workspace `cargo build` doesn't pick it up
//! unless explicitly requested — declared in `Cargo.toml` with `test =
//! false, doc = false` so rustdoc and the default test harness ignore it.

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
        // The test also exercises the malformed-line path by sending the
        // literal string "GARBAGE" — skip it silently so the transport
        // can observe the malformed frame on its side and move on.
        let req: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");

        let response = match method {
            "initialize" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "serverInfo": { "name": "forge-mcp-mock-stdio", "version": "0.0.0" }
                }
            }),
            "tools/list" => serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": [
                        {
                            "name": "ping",
                            "description": "returns pong",
                            "inputSchema": { "type": "object" },
                            "annotations": { "readOnlyHint": true }
                        }
                    ]
                }
            }),
            "tools/call" => {
                // Echo the arguments back so the F-130 manager integration
                // test can assert `tool_name` + `args` round-tripped.
                let tool_name = req
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                let args = req
                    .get("params")
                    .and_then(|p| p.get("arguments"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tool": tool_name, "args": args }
                })
            }
            "shutdown" => {
                // Reply, flush, then drop off the read loop so the parent
                // observes a clean Exit event.
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {}
                });
                let _ = writeln!(out, "{}", resp);
                let _ = out.flush();
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
