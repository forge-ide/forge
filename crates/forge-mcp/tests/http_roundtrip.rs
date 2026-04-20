//! F-129 integration test: drive the HTTP transport against a
//! `wiremock`-backed fake MCP server and verify
//!
//! * a POST round-trip returns a JSON-RPC response via `recv()`,
//! * an SSE GET response delivers a `data:` frame as an `HttpEvent::Message`,
//! * spec headers are propagated on both the POST and the SSE GET.

use std::collections::BTreeMap;
use std::time::Duration;

use forge_mcp::transport::{Http, HttpEvent};
use forge_mcp::{McpServerSpec, ServerKind};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn http_spec(url: &str, auth: &str) -> McpServerSpec {
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".to_string(), auth.to_string());
    McpServerSpec {
        kind: ServerKind::Http {
            url: url.to_string(),
            headers,
        },
    }
}

async fn recv_message(t: &mut Http) -> serde_json::Value {
    let ev = tokio::time::timeout(Duration::from_secs(10), t.recv())
        .await
        .expect("recv timed out")
        .expect("channel closed before message");
    match ev {
        HttpEvent::Message(v) => v,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_roundtrip_and_sse_notification() {
    let server = MockServer::start().await;

    // POST: respond with a JSON-RPC response for id=1.
    Mock::given(method("POST"))
        .and(path("/"))
        .and(header("authorization", "Bearer token"))
        .and(header("content-type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "protocolVersion": "2024-11-05", "capabilities": {} }
        })))
        .mount(&server)
        .await;

    // GET: respond with two SSE `data:` frames. wiremock doesn't stream
    // over time, it delivers the full body and closes — that's enough to
    // exercise frame splitting + JSON parsing + forwarding.
    let sse_body =
        "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n\n\
                    data: {\"jsonrpc\":\"2.0\",\"method\":\"ping\"}\n\n";
    Mock::given(method("GET"))
        .and(path("/"))
        .and(header("authorization", "Bearer token"))
        .and(header("accept", "text/event-stream"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(sse_body),
        )
        .mount(&server)
        .await;

    let url = server.uri();
    let mut t = Http::connect(&http_spec(&url, "Bearer token"))
        .await
        .expect("connect");

    // Kick a POST and expect the response to surface on recv().
    t.send(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }))
    .await
    .expect("POST send");

    // Collect three messages — order between POST response and SSE
    // frames is racy (the GET runs on a reader task from connect time),
    // so we gather by id/method rather than assume order.
    let mut got_post_response = false;
    let mut got_tools_changed = false;
    let mut got_ping = false;

    for _ in 0..3 {
        let v = recv_message(&mut t).await;
        if v.get("id") == Some(&serde_json::json!(1)) {
            assert_eq!(v["result"]["protocolVersion"], "2024-11-05");
            got_post_response = true;
        } else if v.get("method") == Some(&serde_json::json!("notifications/tools/list_changed")) {
            got_tools_changed = true;
        } else if v.get("method") == Some(&serde_json::json!("ping")) {
            got_ping = true;
        } else {
            panic!("unexpected message {v}");
        }
    }

    assert!(got_post_response, "POST response must surface on recv");
    assert!(got_tools_changed, "first SSE notification must surface");
    assert!(got_ping, "second SSE notification must surface");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_2xx_post_surfaces_as_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    // GET is required for the reader task to not spam backoff logs; stub
    // a trivial SSE stream so it terminates cleanly.
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(""),
        )
        .mount(&server)
        .await;

    let t = Http::connect(&http_spec(&server.uri(), "Bearer token"))
        .await
        .expect("connect");

    let err = t
        .send(serde_json::json!({"jsonrpc":"2.0","id":1}))
        .await
        .expect_err("500 must surface");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("500"),
        "error should mention the HTTP status: {msg}"
    );
}
