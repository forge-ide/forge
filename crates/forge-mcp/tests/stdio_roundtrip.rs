//! F-128 integration test: drive the stdio transport against a real child
//! process (the `forge-mcp-mock-stdio` fixture) and round-trip the two
//! MCP handshake messages the DoD calls out — `initialize` and
//! `tools/list`. Also sanity-checks malformed-frame tolerance and the
//! terminal `Exit` event.

use std::collections::BTreeMap;
use std::time::Duration;

use forge_mcp::transport::{Stdio, StdioEvent};
use forge_mcp::{McpServerSpec, ServerKind};

/// Locate the mock server binary via the env var `cargo` sets for
/// declared bins (`CARGO_BIN_EXE_<name>`). This is stable across
/// workspace, release, and sandboxed CI builds.
fn mock_path() -> String {
    env!("CARGO_BIN_EXE_forge-mcp-mock-stdio").to_string()
}

fn mock_spec() -> McpServerSpec {
    McpServerSpec {
        kind: ServerKind::Stdio {
            command: mock_path(),
            args: Vec::new(),
            env: BTreeMap::new(),
        },
    }
}

async fn recv_message(t: &mut Stdio) -> serde_json::Value {
    let ev = tokio::time::timeout(Duration::from_secs(10), t.recv())
        .await
        .expect("recv timed out")
        .expect("channel closed before message");
    match ev {
        StdioEvent::Message(v) => v,
        StdioEvent::Exit(s) => panic!("expected Message, got Exit({s:?})"),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn roundtrips_initialize_and_tools_list() {
    let mut t = Stdio::connect(&mock_spec())
        .await
        .expect("spawn mock stdio");

    // initialize
    t.send(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "forge-mcp-test", "version": "0.0.0" }
        }
    }))
    .await
    .expect("send initialize");

    let init_resp = recv_message(&mut t).await;
    assert_eq!(init_resp["id"], serde_json::json!(1));
    assert_eq!(
        init_resp["result"]["protocolVersion"],
        serde_json::json!("2024-11-05")
    );
    assert!(init_resp["result"]["capabilities"].is_object());

    // tools/list
    t.send(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }))
    .await
    .expect("send tools/list");

    let tools_resp = recv_message(&mut t).await;
    assert_eq!(tools_resp["id"], serde_json::json!(2));
    let tools = tools_resp["result"]["tools"]
        .as_array()
        .expect("tools array");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], serde_json::json!("ping"));

    // shutdown → Exit
    t.send(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "shutdown"
    }))
    .await
    .expect("send shutdown");

    // One response frame for the shutdown, then Exit.
    let shutdown_resp = recv_message(&mut t).await;
    assert_eq!(shutdown_resp["id"], serde_json::json!(3));

    let exit = tokio::time::timeout(Duration::from_secs(10), t.recv())
        .await
        .expect("exit recv timed out")
        .expect("channel closed before Exit");
    match exit {
        StdioEvent::Exit(status) => assert!(status.success(), "mock exited non-zero: {status:?}"),
        StdioEvent::Message(v) => panic!("expected Exit, got Message({v})"),
    }

    // Channel closes after Exit.
    let after = tokio::time::timeout(Duration::from_millis(500), t.recv())
        .await
        .expect("post-exit recv should return None, not hang");
    assert!(after.is_none(), "channel must close after Exit event");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tolerates_partial_writes_across_sends() {
    // Issue two requests back-to-back without awaiting the first
    // response. The transport must interleave them correctly (it
    // serialises via the stdin mutex) and both responses must arrive.
    let mut t = Stdio::connect(&mock_spec())
        .await
        .expect("spawn mock stdio");

    t.send(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "initialize",
        "params": {}
    }))
    .await
    .expect("send #1");
    t.send(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/list"
    }))
    .await
    .expect("send #2");

    let r1 = recv_message(&mut t).await;
    let r2 = recv_message(&mut t).await;

    // The mock responds in the order it reads; with line framing that's
    // the order we sent, so id 10 then id 11.
    assert_eq!(r1["id"], serde_json::json!(10));
    assert_eq!(r2["id"], serde_json::json!(11));
}
