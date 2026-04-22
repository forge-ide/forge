//! Integration test for F-123 DoD: "Integration test spawns a stub LSP
//! (echo server) and verifies initialize/shutdown round-trip."
//!
//! We spawn `forge-lsp-mock-stdio` (the fixture declared in this crate's
//! Cargo.toml) under [`forge_lsp::Server`], send an `initialize` JSON-RPC
//! request, expect an `InitializeResult` on the event channel, then send
//! `shutdown` and expect the child to reap with a clean exit event.
//!
//! The mock fixture exists so CI does not need a real language server
//! installed — see rule #4 of the task brief.

use std::path::PathBuf;
use std::time::Duration;

use forge_lsp::{Server, ServerEvent};
use tokio::time::timeout;

/// Path to the fixture binary built by cargo when this integration test is
/// compiled. `CARGO_BIN_EXE_*` is populated for binaries declared in the
/// same crate's `Cargo.toml`.
fn mock_stdio_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_forge-lsp-mock-stdio"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn initialize_and_shutdown_round_trip() {
    // Spawn the supervisor against the mock fixture. We take the event
    // receiver up-front so the driver task doesn't race the supervisor
    // producing events.
    let mut server = Server::new(mock_stdio_path(), Vec::new());
    let transport = server.transport();
    let mut events = server.take_events().expect("event receiver available");

    // Run `start` on a background task so we can drive I/O from the test.
    let supervisor = tokio::spawn(async move { server.start().await });

    // Give the supervisor a moment to spawn the child + install stdin.
    // `send` returns `NotRunning` until install completes; retry until
    // it succeeds or a short deadline expires.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let attempt = transport
            .send(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {}
            }))
            .await;
        if attempt.is_ok() {
            break;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("initialize send never succeeded: {attempt:?}");
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Expect an `InitializeResult` as the first Message event.
    let msg = timeout(Duration::from_secs(5), events.recv())
        .await
        .expect("initialize response within 5s")
        .expect("channel should yield at least one event");
    let value = match msg {
        ServerEvent::Message(v) => v,
        other => panic!("expected Message, got {other:?}"),
    };
    assert_eq!(
        value.get("id").and_then(|v| v.as_u64()),
        Some(1),
        "response id must match request"
    );
    let caps = value
        .get("result")
        .and_then(|v| v.get("capabilities"))
        .expect("result.capabilities present");
    assert!(caps.is_object(), "capabilities is a JSON object");

    // Now issue `shutdown` — fixture replies + exits.
    transport
        .send(serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "shutdown",
            "params": null
        }))
        .await
        .expect("shutdown send");

    // Drain until we see the shutdown reply, then the terminal Exited
    // event the supervisor emits once the child reaps.
    let mut saw_shutdown_ack = false;
    let mut saw_exit = false;
    let drain_deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while tokio::time::Instant::now() < drain_deadline {
        match timeout(
            drain_deadline.saturating_duration_since(tokio::time::Instant::now()),
            events.recv(),
        )
        .await
        {
            Ok(Some(ServerEvent::Message(v))) => {
                if v.get("id").and_then(|x| x.as_u64()) == Some(2) {
                    saw_shutdown_ack = true;
                }
            }
            Ok(Some(ServerEvent::Exited { .. })) => {
                saw_exit = true;
                break;
            }
            Ok(Some(ServerEvent::GaveUp { .. })) => break,
            Ok(Some(ServerEvent::Malformed { .. })) => continue,
            Ok(None) => break,
            Err(_) => break,
        }
    }
    assert!(saw_shutdown_ack, "must receive shutdown ack");
    assert!(saw_exit, "must surface Exited after child reaps");

    // Supervisor may still be sleeping on backoff; abort so the test
    // returns promptly. We've already proven the round-trip.
    supervisor.abort();
}
