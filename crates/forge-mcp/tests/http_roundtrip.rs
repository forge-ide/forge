//! F-129 integration test: drive the HTTP transport against a
//! `wiremock`-backed fake MCP server and verify
//!
//! * a POST round-trip returns a JSON-RPC response via `recv()`,
//! * an SSE GET response delivers a `data:` frame as an `HttpEvent::Message`,
//! * spec headers are propagated on both the POST and the SSE GET.
//!
//! F-361: also verifies symmetric terminal-event behaviour. When the SSE
//! reader's reconnect loop saturates (sustained failure), the transport
//! must emit [`HttpEvent::Closed`] so the manager can flip the server to
//! `Degraded` within milliseconds — matching the stdio contract — rather
//! than waiting up to 30s for the next health-check tick.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use forge_mcp::manager::LifecycleTuning;
use forge_mcp::transport::http::MAX_SSE_FRAME_BYTES;
use forge_mcp::transport::{Http, HttpEvent};
use forge_mcp::{McpManager, McpServerSpec, ServerKind, ServerState};
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
        HttpEvent::Closed(reason) => panic!("expected Message, got Closed({reason})"),
        HttpEvent::Malformed { bytes_discarded } => {
            panic!("expected Message, got Malformed({bytes_discarded})")
        }
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

/// F-361: when the SSE GET keeps failing, the reader must eventually give
/// up and surface a terminal `HttpEvent::Closed`. Without this the
/// manager's `pump_exit` channel is dead for HTTP and a crashed remote
/// server only becomes visible on the 30s health-check tick. Here every
/// GET returns 503 so the reader backs off and never recovers — the
/// transport must emit `Closed` and then close the receiver.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse_sustained_failure_surfaces_terminal_closed_event() {
    let server = MockServer::start().await;

    // Stub a POST so `Http::connect` has a partner for the outbound path;
    // the test itself only exercises the GET/SSE reader's failure path.
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    // Every GET returns 503 — no SSE stream will ever open. The reader
    // should retry up to the sustained-failure threshold, then emit
    // `HttpEvent::Closed`.
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&server)
        .await;

    let mut t = Http::connect(&http_spec(&server.uri(), "Bearer token"))
        .await
        .expect("connect");

    // Budget: reader backoff is initial 100ms, doubling to 30s cap. Three
    // consecutive failures fit well under a couple seconds even with
    // jitter on slow CI runners.
    let ev = tokio::time::timeout(Duration::from_secs(10), t.recv())
        .await
        .expect("timed out waiting for HttpEvent::Closed")
        .expect("channel closed before Closed event surfaced");

    match ev {
        HttpEvent::Closed(reason) => {
            assert!(
                !reason.is_empty(),
                "Closed event must carry a non-empty reason"
            );
            // Dropping the handle aborts the reader and releases the
            // reqwest client; we don't assert channel auto-close here
            // because `Http::send` keeps a live sender clone for POST
            // response forwarding. The manager treats `Closed` itself
            // as the terminal signal and drops the transport.
        }
        HttpEvent::Message(v) => {
            panic!("expected HttpEvent::Closed after sustained failure, got Message({v})")
        }
        HttpEvent::Malformed { bytes_discarded } => {
            panic!("expected HttpEvent::Closed after sustained failure, got Malformed({bytes_discarded})")
        }
    }
}

/// F-361 regression at the manager layer. A dead HTTP MCP server (503 on
/// both POST and GET) must surface as `Degraded` via the transport's
/// terminal event, not the 30s health-check tick. We set the health
/// interval high enough that any Degraded we observe cannot have come
/// from a health ping — it can only have come from `pump_exit` firing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manager_degrades_http_server_on_sustained_reconnect_failure() {
    let server = MockServer::start().await;

    // Every POST (including `initialize` / `tools/list` handshake) and
    // every GET returns 503. The manager should flip `Degraded` via the
    // SSE reader's terminal `Closed` event or the handshake failure —
    // either path is driven by the transport, not the health tick.
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&server)
        .await;

    let mut headers = BTreeMap::new();
    headers.insert("Authorization".into(), "Bearer token".into());
    let spec = McpServerSpec {
        kind: ServerKind::Http {
            url: server.uri(),
            headers,
        },
    };

    let mut cfg = BTreeMap::new();
    cfg.insert("remote".to_string(), spec);

    // Health-check interval is pinned high enough that any Degraded we
    // observe inside the test budget cannot have come from a health
    // ping — it can only have come from the transport's terminal event.
    let tuning = LifecycleTuning {
        health_check_interval: Duration::from_secs(60),
    };
    let mgr = McpManager::with_tuning(cfg, tuning);

    mgr.start("remote").await.expect("start remote");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list = mgr.list().await;
        let entry = list
            .iter()
            .find(|s| s.name == "remote")
            .expect("remote entry");
        if matches!(entry.state, ServerState::Degraded { .. }) {
            break;
        }
        if Instant::now() >= deadline {
            panic!(
                "remote server did not reach Degraded; last state = {:?}",
                entry.state
            );
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    mgr.stop("remote").await.expect("stop remote");
}

/// F-348: URL credentials (query-string tokens, `user:pass@` userinfo) must
/// never appear in the error returned by `Http::send`. The MCP server URL is
/// needed inside reqwest but every user-facing emission routes through the
/// `redacted()` helper, which strips query, fragment, and userinfo.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn post_error_redacts_query_string_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(""),
        )
        .mount(&server)
        .await;

    // Append a secret as a query-string token, mirroring signed-URL /
    // personal-dev-proxy patterns called out in the threat model.
    let url_with_token = format!("{}/?access_token=shhh-no-logging", server.uri());
    let t = Http::connect(&http_spec(&url_with_token, "Bearer token"))
        .await
        .expect("connect");

    let err = t
        .send(serde_json::json!({"jsonrpc":"2.0","id":1}))
        .await
        .expect_err("500 must surface");
    let msg = format!("{err:#}");
    assert!(
        !msg.contains("shhh-no-logging"),
        "query-string token must not appear in error: {msg}"
    );
    assert!(
        !msg.contains("access_token"),
        "query key must not appear in error: {msg}"
    );
    assert!(
        msg.contains("500"),
        "error should still name the HTTP status: {msg}"
    );
}

/// F-348: sustained SSE failure emits `HttpEvent::Closed(reason)`. That
/// reason string is broadcast by the manager as `Degraded { reason }`, so
/// it absolutely must not carry URL credentials.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse_closed_reason_redacts_query_string_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(503).set_body_string("down"))
        .mount(&server)
        .await;

    let url_with_token = format!("{}/?access_token=shhh-no-broadcast", server.uri());
    let mut t = Http::connect(&http_spec(&url_with_token, "Bearer token"))
        .await
        .expect("connect");

    let ev = tokio::time::timeout(Duration::from_secs(10), t.recv())
        .await
        .expect("timed out waiting for HttpEvent::Closed")
        .expect("channel closed before Closed event surfaced");

    match ev {
        HttpEvent::Closed(reason) => {
            assert!(
                !reason.contains("shhh-no-broadcast"),
                "Closed reason must not carry the token: {reason}"
            );
            assert!(
                !reason.contains("access_token"),
                "Closed reason must not carry the query key: {reason}"
            );
        }
        HttpEvent::Message(v) => {
            panic!("expected HttpEvent::Closed after sustained failure, got Message({v})")
        }
        HttpEvent::Malformed { bytes_discarded } => {
            panic!("expected HttpEvent::Closed after sustained failure, got Malformed({bytes_discarded})")
        }
    }
}

/// F-347 DoD regression: an SSE response that streams 16 MiB of bytes
/// without emitting an event boundary (`\n\n` or `\r\n\r\n`) must not
/// drive the reader's accumulator past `MAX_SSE_FRAME_BYTES`. The
/// transport must surface `HttpEvent::Malformed { bytes_discarded >= cap }`
/// and the sustained-failure reconnect loop bounds subsequent runs,
/// eventually emitting `HttpEvent::Closed`.
///
/// Fixture: wiremock serves a single 16 MiB response body that is all
/// `data: xxx...` with no trailing `\n\n`. Because wiremock writes the
/// body in one shot before the connection closes, every reconnect gets
/// the same hostile payload and trips the cap again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse_sixteen_mib_no_boundary_surfaces_malformed() {
    let server = MockServer::start().await;

    // POST: harmless 200 so the transport can `connect` cleanly.
    Mock::given(method("POST"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    // 16 MiB of `data: ` bytes with no event boundary. Every byte is an
    // ASCII letter so neither `\n\n` nor `\r\n\r\n` ever appears — this
    // is the pathological DoS shape the pre-fix reader accumulated into
    // `buf` without bound.
    let hostile_body: Vec<u8> = {
        let mut v = Vec::with_capacity(16 * 1024 * 1024);
        v.extend_from_slice(b"data: ");
        while v.len() < 16 * 1024 * 1024 {
            v.push(b'A');
        }
        v
    };

    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_bytes(hostile_body),
        )
        .mount(&server)
        .await;

    let mut t = Http::connect(&http_spec(&server.uri(), "Bearer token"))
        .await
        .expect("connect");

    let mut saw_malformed = false;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(10), t.recv()).await {
            Ok(Some(HttpEvent::Malformed { bytes_discarded })) => {
                assert!(
                    bytes_discarded >= MAX_SSE_FRAME_BYTES,
                    "bytes_discarded must be >= cap: {bytes_discarded}",
                );
                saw_malformed = true;
                break;
            }
            // Closed can land if the sustained-failure threshold is
            // reached before Malformed on a slow CI runner; treat that
            // as a failure because the DoD specifically asks for the
            // named Malformed error shape.
            Ok(Some(HttpEvent::Closed(reason))) => {
                panic!("expected Malformed before Closed; got Closed({reason})")
            }
            Ok(Some(HttpEvent::Message(v))) => {
                panic!("unexpected Message event in over-cap SSE fixture: {v}")
            }
            Ok(None) | Err(_) => break,
        }
    }

    assert!(
        saw_malformed,
        "over-cap SSE frame must surface HttpEvent::Malformed",
    );
}
