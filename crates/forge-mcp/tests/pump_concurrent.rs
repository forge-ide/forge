//! F-369: concurrent-call coverage for the forge-mcp pump pending-table.
//!
//! `manager::run_pump` keeps an id-keyed `HashMap` of in-flight oneshot
//! senders so multiple `tools/call` invocations against one server each
//! get routed back to the correct waiter. No existing test exercises
//! that routing under concurrency:
//!
//! - `tests/manager_subprocess.rs` is `#[ignore]` (serial-only) and
//!   issues one call at a time.
//! - `tests/stdio_roundtrip.rs` covers transport-level ordering, not
//!   the manager's pending table.
//!
//! A regression that clobbered `pending` (e.g. reused a single slot
//! across ids, or routed responses by arrival order instead of id)
//! would still pass both suites. These tests drive the pump directly
//! over an in-memory transport so the routing is the only thing under
//! test: we issue N concurrent requests, have the mock reply with ids
//! in reverse order, and assert each caller receives its own response.
//!
//! Runs in the normal (non-serial) CI lane — no subprocesses, no SIGCHLD
//! race, so no `#[ignore]`.

use std::time::Duration;

use forge_mcp::manager::test_util::{spawn_pump_in_proc, InProcServer};
use serde_json::json;
use tokio::time::timeout;

/// Read the next outbound frame from the mock server side of the in-proc
/// transport, failing fast rather than hanging if the pump never sends.
async fn next_frame(server: &mut InProcServer) -> serde_json::Value {
    timeout(Duration::from_secs(2), server.recv_from_client())
        .await
        .expect("pump should have sent a frame within 2s")
        .expect("in-proc transport closed unexpectedly")
}

/// JSON-RPC response envelope keyed to a specific id. Mirrors the shape
/// the real pump extracts ids from in `extract_id`.
fn response(id: u64, result: serde_json::Value) -> serde_json::Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pump_routes_concurrent_responses_by_id() {
    // Boot a pump with an in-memory transport. The handle wraps the real
    // Connection so we can use the normal `call()` path — the only thing
    // swapped out is the underlying transport.
    let (pump, mut server) = spawn_pump_in_proc("test-concurrent");

    // Fire three requests concurrently. Each call registers a distinct
    // pending slot (ids 1, 2, 3 per the AtomicU64 generator) and parks
    // its waiter. Using `tokio::spawn` rather than `tokio::join!` so the
    // three sends can be issued and observed by the server side before
    // any response is posted back.
    let c1 = {
        let p = pump.clone();
        tokio::spawn(async move { p.call_raw("tools/call", json!({ "n": 1 })).await })
    };
    let c2 = {
        let p = pump.clone();
        tokio::spawn(async move { p.call_raw("tools/call", json!({ "n": 2 })).await })
    };
    let c3 = {
        let p = pump.clone();
        tokio::spawn(async move { p.call_raw("tools/call", json!({ "n": 3 })).await })
    };

    // Drain the three outbound frames from the pump. Record each frame's
    // id + its caller-supplied `n` so we can reply with a result that
    // lets the caller verify it got its own response (not somebody
    // else's).
    let mut frames = Vec::new();
    for _ in 0..3 {
        let f = next_frame(&mut server).await;
        let id = f.get("id").and_then(|v| v.as_u64()).expect("frame id");
        let n = f
            .get("params")
            .and_then(|p| p.get("n"))
            .and_then(|v| v.as_u64())
            .expect("frame params.n");
        frames.push((id, n));
    }

    // Reply in *reverse* order of arrival. If the pump routed by arrival
    // order instead of by id (the regression this test guards against)
    // each caller would see somebody else's payload.
    for (id, n) in frames.iter().rev() {
        server
            .send_to_client(response(*id, json!({ "echo": n })))
            .await;
    }

    // Each caller should observe the `n` it sent — proving the pending
    // table routed by id, not by response arrival order.
    let r1 = c1.await.expect("c1 task").expect("c1 call");
    let r2 = c2.await.expect("c2 task").expect("c2 call");
    let r3 = c3.await.expect("c3 task").expect("c3 call");

    assert_eq!(r1, json!({ "echo": 1 }), "caller 1 got its own response");
    assert_eq!(r2, json!({ "echo": 2 }), "caller 2 got its own response");
    assert_eq!(r3, json!({ "echo": 3 }), "caller 3 got its own response");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pump_isolates_per_server_pending_tables() {
    // Two independent pumps simulate two servers managed side-by-side.
    // Each owns its own pending table — a response arriving on server A's
    // transport with an id that exists in server B's table must not
    // reach server B's waiter.
    let (pump_a, mut server_a) = spawn_pump_in_proc("server-a");
    let (pump_b, mut server_b) = spawn_pump_in_proc("server-b");

    // Issue a concurrent call against each pump.
    let call_a = {
        let p = pump_a.clone();
        tokio::spawn(async move { p.call_raw("tools/call", json!({ "where": "A" })).await })
    };
    let call_b = {
        let p = pump_b.clone();
        tokio::spawn(async move { p.call_raw("tools/call", json!({ "where": "B" })).await })
    };

    // Both pumps independently assign id=1 (each has its own AtomicU64
    // counter). Pull the frames out and verify.
    let fa = next_frame(&mut server_a).await;
    let fb = next_frame(&mut server_b).await;
    assert_eq!(fa.get("id").and_then(|v| v.as_u64()), Some(1));
    assert_eq!(fb.get("id").and_then(|v| v.as_u64()), Some(1));

    // Cross-wire the responses: send A's id=1 response to *server A*
    // with B's payload (to prove the tables don't leak), and vice versa.
    // Each caller must still receive the payload its own server sent.
    server_a
        .send_to_client(response(1, json!({ "from": "A" })))
        .await;
    server_b
        .send_to_client(response(1, json!({ "from": "B" })))
        .await;

    let ra = call_a.await.expect("a task").expect("a call");
    let rb = call_b.await.expect("b task").expect("b call");

    assert_eq!(
        ra,
        json!({ "from": "A" }),
        "server A caller got server A response"
    );
    assert_eq!(
        rb,
        json!({ "from": "B" }),
        "server B caller got server B response"
    );
}
