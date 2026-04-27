//! F-371: structured tracing across `BackgroundAgentRegistry` lifecycle.
//!
//! Pins target + field shape for each emission site so a future rename or
//! a silent regression breaks the test instead of operator log filters.
//! Pattern mirrors `forge-agents/tests/orchestrator.rs` (F-373).

mod common;

use std::sync::Arc;
use std::time::Duration;

use forge_agents::{AgentDef, Isolation, Orchestrator};
use forge_session::BackgroundAgentRegistry;

fn def(name: &str) -> AgentDef {
    AgentDef {
        name: name.to_string(),
        description: None,
        body: String::new(),
        allowed_paths: vec![],
        isolation: Isolation::Process,
        memory_enabled: false,
    }
}

fn registry() -> BackgroundAgentRegistry {
    let orch = Arc::new(Orchestrator::new());
    let defs = Arc::new(vec![def("writer")]);
    BackgroundAgentRegistry::new(orch, defs)
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn start_emits_lifecycle_log_with_instance_id_and_agent_name() {
    let _g = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    let reg = registry();
    let id = reg.start("writer", Arc::from("go")).await.unwrap();

    let logs = common::drain_capture();
    assert!(
        logs.contains("forge_session::bg_agents"),
        "expected target forge_session::bg_agents, got logs: {logs}"
    );
    assert!(
        logs.contains(&format!("instance_id={id}")),
        "expected instance_id field, got logs: {logs}"
    );
    assert!(
        logs.contains("agent_name=\"writer\"") || logs.contains("agent_name=writer"),
        "expected agent_name field, got logs: {logs}"
    );
    assert!(
        logs.contains("started"),
        "expected lifecycle marker 'started', got logs: {logs}"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn unknown_agent_emits_warn_with_agent_name() {
    let _g = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    let reg = registry();
    let _ = reg.start("missing-agent", Arc::from("p")).await;

    let logs = common::drain_capture();
    assert!(
        logs.contains("forge_session::bg_agents"),
        "expected target forge_session::bg_agents, got logs: {logs}"
    );
    assert!(
        logs.contains("WARN"),
        "expected WARN level on unknown-agent rejection, got: {logs}"
    );
    assert!(
        logs.contains("missing-agent"),
        "expected the unknown agent name in the log, got: {logs}"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn completion_event_emits_lifecycle_log() {
    let _g = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    let reg = registry();
    let id = reg.start("writer", Arc::from("p")).await.unwrap();

    // Drain any spawn-side logs so the next capture is the completion path only.
    let _ = common::drain_capture();

    reg.orchestrator().stop(&id).await.unwrap();

    // Give the forwarder task a tick to receive the terminal event and emit.
    let mut logs = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let snapshot = common::drain_capture();
        logs.push_str(&snapshot);
        if logs.contains("completed") && logs.contains(&format!("instance_id={id}")) {
            break;
        }
    }

    assert!(
        logs.contains("forge_session::bg_agents"),
        "expected target forge_session::bg_agents, got: {logs}"
    );
    assert!(
        logs.contains(&format!("instance_id={id}")),
        "expected instance_id field on completion log, got: {logs}"
    );
    assert!(
        logs.contains("completed"),
        "expected lifecycle marker 'completed', got: {logs}"
    );
}

#[allow(clippy::await_holding_lock)]
#[tokio::test]
async fn failure_path_emits_warn_lifecycle_log() {
    let _g = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    let reg = registry();
    let id = reg.start("writer", Arc::from("p")).await.unwrap();

    let _ = common::drain_capture();
    reg.orchestrator()
        .fail(&id, "boom".to_string())
        .await
        .unwrap();

    let mut logs = String::new();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
        let snapshot = common::drain_capture();
        logs.push_str(&snapshot);
        if logs.contains("failed") && logs.contains(&format!("instance_id={id}")) {
            break;
        }
    }

    assert!(
        logs.contains("forge_session::bg_agents"),
        "expected target forge_session::bg_agents, got: {logs}"
    );
    assert!(
        logs.contains("WARN"),
        "expected WARN level on the failure log, got: {logs}"
    );
    assert!(
        logs.contains("failed"),
        "expected lifecycle marker 'failed', got: {logs}"
    );
    assert!(
        logs.contains(&format!("instance_id={id}")),
        "expected instance_id field on failure log, got: {logs}"
    );
}

// Source-level eprintln! audit lives in `eprintln_audit.rs` so it can cover
// every file in `src/` (server.rs, bg_agents.rs, …) behind one assertion.
