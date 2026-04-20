//! Runtime tests for `Orchestrator` / `AgentInstance` (F-133).
//!
//! These cover the DoD items:
//! - spawn returns an `AgentInstance` and registers it
//! - stop terminates an instance cleanly and moves it out of `Running`
//! - `state_stream()` yields `Spawned` before `Completed` in order
//! - user-scope `isolation: Trusted` is rejected at spawn with a typed error
//!   (`Error::IsolationViolation`), independent of parse-time rejection

use forge_agents::{
    AgentDef, AgentScope, Error, InstanceState, Isolation, Orchestrator, SpawnContext,
};
use futures::StreamExt;

fn process_def(name: &str) -> AgentDef {
    AgentDef {
        name: name.to_string(),
        description: None,
        body: String::new(),
        allowed_paths: vec![],
        isolation: Isolation::Process,
    }
}

fn trusted_def(name: &str) -> AgentDef {
    AgentDef {
        name: name.to_string(),
        description: None,
        body: String::new(),
        allowed_paths: vec![],
        isolation: Isolation::Trusted,
    }
}

#[tokio::test]
async fn spawn_returns_instance_and_registers_it() {
    let orch = Orchestrator::new();
    let def = process_def("helper");

    let inst = orch
        .spawn(def.clone(), SpawnContext::user())
        .await
        .expect("spawn should succeed for Process isolation");

    assert_eq!(inst.def.name, "helper");
    assert_eq!(inst.state, InstanceState::Running);
    // Registered: we can look it up by id.
    assert!(
        orch.get(&inst.id).await.is_some(),
        "orchestrator should register the spawned instance"
    );
}

#[tokio::test]
async fn stop_terminates_instance_cleanly() {
    let orch = Orchestrator::new();
    let inst = orch
        .spawn(process_def("helper"), SpawnContext::user())
        .await
        .unwrap();

    orch.stop(&inst.id).await.expect("stop should succeed");

    let after = orch
        .get(&inst.id)
        .await
        .expect("instance still in registry");
    assert_eq!(
        after.state,
        InstanceState::Completed,
        "stop should drive the instance to Completed"
    );
}

#[tokio::test]
async fn state_stream_emits_spawned_then_completed_in_order() {
    let orch = Orchestrator::new();
    let mut stream = orch.state_stream();

    let inst = orch
        .spawn(process_def("flow"), SpawnContext::user())
        .await
        .unwrap();
    orch.stop(&inst.id).await.unwrap();

    // Drop the orchestrator so the broadcast channel closes and the stream
    // ends after draining.
    drop(orch);

    let mut events = Vec::new();
    while let Some(ev) = stream.next().await {
        if let Ok(ev) = ev {
            events.push(ev);
        }
    }

    assert!(
        matches!(events.first(), Some(forge_agents::AgentEvent::Spawned { id, .. }) if *id == inst.id),
        "first event must be Spawned for this instance, got: {events:?}"
    );
    assert!(
        matches!(events.last(), Some(forge_agents::AgentEvent::Completed { id, .. }) if *id == inst.id),
        "last event must be Completed for this instance, got: {events:?}"
    );
    assert!(
        events
            .iter()
            .position(|e| matches!(e, forge_agents::AgentEvent::Spawned { .. }))
            < events
                .iter()
                .position(|e| matches!(e, forge_agents::AgentEvent::Completed { .. })),
        "Spawned must precede Completed"
    );
}

#[tokio::test]
async fn spawn_rejects_trusted_isolation_for_user_scope() {
    let orch = Orchestrator::new();
    let def = trusted_def("evil");

    let err = orch
        .spawn(def, SpawnContext::user())
        .await
        .expect_err("spawn must reject isolation: trusted for user scope");

    assert!(
        matches!(err, Error::IsolationViolation { .. }),
        "expected IsolationViolation, got: {err:?}"
    );
    assert!(
        err.to_string().to_lowercase().contains("trusted"),
        "error message should mention 'trusted': {err}"
    );
}

#[tokio::test]
async fn spawn_allows_trusted_isolation_for_builtin_scope() {
    // Built-in skills are allowed to spawn trusted instances — this is the
    // escape hatch the typed runtime check protects.
    let orch = Orchestrator::new();

    let inst = orch
        .spawn(trusted_def("builtin"), SpawnContext::built_in())
        .await
        .expect("built-in scope may run trusted isolation");

    assert_eq!(inst.def.isolation, Isolation::Trusted);
    assert_eq!(inst.state, InstanceState::Running);
}

#[tokio::test]
async fn failed_instance_emits_failed_event_and_state() {
    // A test-only helper: fail a running instance. This proves the Failed
    // terminal state and corresponding event are reachable without
    // implementing a real step executor (that lands in F-134).
    let orch = Orchestrator::new();
    let mut stream = orch.state_stream();

    let inst = orch
        .spawn(process_def("doomed"), SpawnContext::user())
        .await
        .unwrap();

    orch.fail(&inst.id, "synthetic failure".to_string())
        .await
        .unwrap();

    let after = orch.get(&inst.id).await.unwrap();
    assert!(matches!(after.state, InstanceState::Failed { .. }));

    drop(orch);
    let mut events = Vec::new();
    while let Some(Ok(ev)) = stream.next().await {
        events.push(ev);
    }
    assert!(
        events
            .iter()
            .any(|e| matches!(e, forge_agents::AgentEvent::Failed { id, .. } if *id == inst.id)),
        "Failed event should be emitted, got: {events:?}"
    );
}

#[tokio::test]
async fn step_events_are_emitted_between_spawn_and_completion() {
    // Confirms the event vocabulary carries StepStarted/StepFinished (per DoD
    // `AgentEvent::{Spawned,StepStarted,StepFinished,Completed,Failed}`). For
    // the runtime foundation we expose `record_step_started / record_step_finished`
    // so downstream call sites (sub-agent spawning, provider turns) can emit
    // them — F-134 composes on top.
    let orch = Orchestrator::new();
    let mut stream = orch.state_stream();

    let inst = orch
        .spawn(process_def("stepper"), SpawnContext::user())
        .await
        .unwrap();
    orch.record_step_started(&inst.id, "think".to_string())
        .await
        .unwrap();
    orch.record_step_finished(&inst.id, "think".to_string())
        .await
        .unwrap();
    orch.stop(&inst.id).await.unwrap();
    drop(orch);

    let mut events = Vec::new();
    while let Some(Ok(ev)) = stream.next().await {
        events.push(ev);
    }

    let positions = |want: fn(&forge_agents::AgentEvent) -> bool| events.iter().position(want);
    let spawned = positions(|e| matches!(e, forge_agents::AgentEvent::Spawned { .. })).unwrap();
    let started = positions(|e| matches!(e, forge_agents::AgentEvent::StepStarted { .. })).unwrap();
    let finished =
        positions(|e| matches!(e, forge_agents::AgentEvent::StepFinished { .. })).unwrap();
    let completed = positions(|e| matches!(e, forge_agents::AgentEvent::Completed { .. })).unwrap();

    assert!(
        spawned < started && started < finished && finished < completed,
        "event order Spawned < StepStarted < StepFinished < Completed, got: {events:?}"
    );
}

#[tokio::test]
async fn scope_marker_is_independent_of_parse_time_check() {
    // The runtime check must be enforced on programmatically-constructed
    // defs, not just parser output. We construct an AgentDef manually with
    // Trusted isolation (bypassing the parser) and confirm spawn still
    // rejects it under user scope. This is the DoD's "enforced again at
    // runtime to be safe" clause.
    let orch = Orchestrator::new();
    let def = AgentDef {
        name: "bypass".to_string(),
        description: None,
        body: String::new(),
        allowed_paths: vec![],
        isolation: Isolation::Trusted,
    };
    let result = orch
        .spawn(
            def,
            SpawnContext {
                scope: AgentScope::User,
                ..Default::default()
            },
        )
        .await;
    assert!(matches!(result, Err(Error::IsolationViolation { .. })));
}
