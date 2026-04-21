//! F-154 end-to-end integration test for `McpManager` + stdio transport +
//! a real spawned MCP subprocess.
//!
//! Scope: exercise the manager composition that unit tests in
//! `manager::tests` deliberately avoid (they cover ladder / admission /
//! API-error logic in isolation). This test runs the manager against the
//! `forge-mcp-mock-stdio` fixture binary, asserting:
//!
//! 1. `start()` drives Starting → Healthy via a real `initialize` +
//!    `tools/list` handshake against the child,
//! 2. the mock's `crash` method forces an abrupt process exit, the
//!    lifecycle observes the transport close, publishes `Degraded`,
//!    sleeps the first rung of the backoff ladder, and re-enters
//!    Starting → Healthy on the restart,
//! 3. a `tools/call` round-trip routes the request to the child and
//!    surfaces the echoed response payload back through `McpManager::call`.
//!
//! ## Why `#[ignore]`
//!
//! Tokio's process reaper runs one global SIGCHLD handler per process.
//! When `cargo test -p forge-mcp` runs multiple test binaries in
//! parallel, a second binary's child exits can race the reaper that a
//! previous binary installed, producing spurious stdio transport
//! failures in this test. The manager is not at fault — it's a
//! test-harness interaction. CI runs this test under
//! `cargo test -- --ignored --test-threads=1`, i.e. a single test
//! binary with a single test thread, which avoids the race entirely.
//!
//! Do not lift the `#[ignore]` to make it run under the parallel suite.
//! See `crates/forge-mcp/README.md` for the full background.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use forge_mcp::manager::LifecycleTuning;
use forge_mcp::{McpManager, McpServerSpec, ServerKind, ServerState};
use futures::StreamExt;

/// Locate the mock server binary via the env var `cargo` sets for
/// declared bins (`CARGO_BIN_EXE_<name>`). Same mechanism the stdio
/// transport integration test uses.
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

/// Poll `McpManager::list()` until the named server reaches the expected
/// predicate state, or fail with the terminal state observed. Returns
/// the final [`ServerState`] so callers can make further assertions
/// (e.g. that the `Degraded` reason names the transport-close event).
async fn wait_for_state<F>(
    mgr: &McpManager,
    name: &str,
    mut pred: F,
    budget: Duration,
    label: &str,
) -> ServerState
where
    F: FnMut(&ServerState) -> bool,
{
    let deadline = Instant::now() + budget;
    loop {
        let list = mgr.list().await;
        let entry = list
            .iter()
            .find(|s| s.name == name)
            .unwrap_or_else(|| panic!("server {name:?} not in list for {label}"));
        if pred(&entry.state) {
            return entry.state.clone();
        }
        if Instant::now() >= deadline {
            panic!(
                "timed out waiting for {label}; last state = {:?}",
                entry.state
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "F-154: runs only under `cargo test -- --ignored --test-threads=1` (serial) due to tokio SIGCHLD reaper race across parallel test binaries"]
async fn manager_drives_real_subprocess_through_crash_restart_and_tool_call() {
    let mut cfg = BTreeMap::new();
    cfg.insert("mock".to_string(), mock_spec());

    // Compress the health-check cadence so a restart round-trip fits
    // inside the test's overall budget. The restart-backoff ladder is
    // not tunable (first rung is 1s) — we don't try to drive more than
    // one rung, which keeps total wall time bounded to ~5s.
    let tuning = LifecycleTuning {
        health_check_interval: Duration::from_millis(50),
    };
    let mgr = McpManager::with_tuning(cfg, tuning);

    // Subscribe BEFORE start so the broadcast channel doesn't drop the
    // pre-subscription Starting / Healthy transitions. We only use the
    // stream to observe ordering — the definitive assertions go through
    // `list()` (the same snapshot the IPC layer sees).
    let mut events = mgr.state_stream();

    mgr.start("mock").await.expect("start mock");

    // (1) Starting → Healthy
    let state = wait_for_state(
        &mgr,
        "mock",
        |s| matches!(s, ServerState::Healthy),
        Duration::from_secs(5),
        "initial Healthy",
    )
    .await;
    assert!(matches!(state, ServerState::Healthy));

    // Cached tool list should have the mock's single `ping` tool,
    // server-namespaced as `mock.ping`.
    let list = mgr.list().await;
    let tools = &list
        .iter()
        .find(|s| s.name == "mock")
        .expect("mock entry")
        .tools;
    assert_eq!(tools.len(), 1, "mock exposes exactly one tool");
    assert_eq!(tools[0].name, "mock.ping", "tool name is server-namespaced");
    assert!(tools[0].read_only, "ping carries readOnlyHint: true");

    // (2) tool call round-trips through the real subprocess. The mock
    // echoes `{ tool, args }` back as the JSON-RPC result so we can
    // assert both routing and response plumbing.
    let call_result = mgr
        .call("mock", "ping", serde_json::json!({ "from": "F-154" }))
        .await
        .expect("tools/call round-trips through the manager");
    assert_eq!(
        call_result,
        serde_json::json!({
            "tool": "ping",
            "args": { "from": "F-154" }
        }),
        "mock must echo tool name and arguments",
    );

    // (3) Force a transport-close event by calling the mock's special
    // "crash" tool — the mock exits 1 without replying when a
    // `tools/call` names `crash` as the tool. The lifecycle driver
    // publishes `Degraded { reason }` (transport died) rather than
    // `Failed`: `Failed` is reserved for restart-cap exhaustion or
    // manual stop. The issue DoD's "assert Failed" after a single
    // crash directly contradicts "assert restart triggers" that
    // follows, so the honest state-machine reading we test here is
    // Healthy → Degraded → Starting → Healthy.
    //
    // The crash call either returns an error (pump died before routing
    // a response) or times out on its own oneshot; either is fine —
    // what we assert is the observable state transition that follows.
    let _ = mgr.call("mock", "crash", serde_json::json!({})).await;

    let degraded_state = wait_for_state(
        &mgr,
        "mock",
        |s| matches!(s, ServerState::Degraded { .. }),
        Duration::from_secs(5),
        "Degraded after crash",
    )
    .await;
    match &degraded_state {
        ServerState::Degraded { reason } => {
            assert!(
                reason.contains("transport")
                    || reason.contains("health")
                    || reason.contains("exit"),
                "Degraded reason should name the transport/health/exit event, got {reason:?}",
            );
        }
        other => panic!("expected Degraded, got {other:?}"),
    }

    // (4) Restart fires per the backoff ladder. First rung is 1s; we
    // give a generous 10s budget for the restart round-trip (1s sleep
    // + process spawn + handshake + first health check).
    let state = wait_for_state(
        &mgr,
        "mock",
        |s| matches!(s, ServerState::Healthy),
        Duration::from_secs(10),
        "Healthy after restart",
    )
    .await;
    assert!(matches!(state, ServerState::Healthy));

    // Drain the event stream non-blockingly to make sure we saw at
    // least one Starting → Healthy → Degraded → Starting → Healthy
    // sequence on the broadcast side. We don't assert exact ordering
    // across all events (broadcast is lossy under lag) — we just
    // confirm both Healthy and Degraded appeared.
    let mut saw_healthy = false;
    let mut saw_degraded = false;
    // Non-blocking drain: poll the stream for up to 100ms to surface
    // whatever's queued.
    let drain_deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < drain_deadline {
        match tokio::time::timeout(Duration::from_millis(50), events.next()).await {
            Ok(Some(ev)) => match ev.state {
                ServerState::Healthy => saw_healthy = true,
                ServerState::Degraded { .. } => saw_degraded = true,
                _ => {}
            },
            Ok(None) => break,
            Err(_) => {}
        }
    }
    assert!(saw_healthy, "state_stream should have emitted Healthy");
    assert!(saw_degraded, "state_stream should have emitted Degraded");

    // (5) Clean up: stop parks the server and tears down the driver.
    mgr.stop("mock").await.expect("stop mock");
    let final_state = wait_for_state(
        &mgr,
        "mock",
        |s| matches!(s, ServerState::Failed { .. }),
        Duration::from_secs(2),
        "Failed after stop",
    )
    .await;
    match final_state {
        ServerState::Failed { reason } => {
            assert_eq!(reason, "stopped", "stop() reason is literal 'stopped'");
        }
        other => panic!("expected Failed, got {other:?}"),
    }
}
