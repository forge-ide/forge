//! F-137 IPC integration tests for `start_background_agent` /
//! `promote_background_agent` / `list_background_agents`.
//!
//! These exercise the invoke → `BackgroundAgentRegistry` → `session:event`
//! emit path via `tauri::test::get_ipc_response`. They:
//!
//! - verify authz (dashboard / wrong-session windows are rejected),
//! - prove `start` spawns an instance and surfaces it in `list`,
//! - prove the registry's `BackgroundAgentStarted` / `BackgroundAgentCompleted`
//!   events are forwarded onto the session's `session:event` Tauri channel
//!   (the same channel the daemon uses — no new event name),
//! - prove `promote` removes the id from `list` but leaves the underlying
//!   orchestrator instance alive.
//!
//! Registries are injected via the `webview-test`-gated
//! `install_bg_session_for_test` seam so the tests don't need a live daemon
//! or a real `workspace_root` / `.agents/*.md` tree on disk.

#![cfg(feature = "webview-test")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_agents::{AgentDef, Isolation, Orchestrator as AgentOrchestrator};
use forge_session::BackgroundAgentRegistry;
use forge_shell::bridge::SessionConnections;
use forge_shell::ipc::{
    build_invoke_handler, install_bg_session_for_test, manage_bg_agents, BridgeState,
};
use serde_json::Value;
use tauri::test::{get_ipc_response, mock_builder, mock_context, noop_assets, INVOKE_KEY};
use tauri::{Listener, Manager};

const LABEL_MISMATCH: &str = "forbidden: window label mismatch";

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    let app = mock_builder()
        .invoke_handler(build_invoke_handler())
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app");
    // Session bridge state is required so label-mismatch assertions reach
    // our size/authz layer rather than a missing-resource panic.
    app.manage(BridgeState::new(SessionConnections::new()));
    // BgAgentState is lazily created by the production code path, but the
    // `install_bg_session_for_test` seam below expects it to exist.
    manage_bg_agents(&app.handle().clone());
    app
}

fn make_window(
    app: &tauri::App<tauri::test::MockRuntime>,
    label: &str,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .build()
        .expect("mock window")
}

fn invoke(
    window: &tauri::WebviewWindow<tauri::test::MockRuntime>,
    cmd: &str,
    payload: serde_json::Value,
) -> Result<tauri::ipc::InvokeResponseBody, String> {
    get_ipc_response(
        window,
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::Json(payload),
            headers: Default::default(),
            invoke_key: INVOKE_KEY.to_string(),
        },
    )
    .map_err(|v| match v {
        serde_json::Value::String(s) => s,
        other => other.to_string(),
    })
}

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

/// Construct a fresh registry pre-populated with two defs (`writer`,
/// `reviewer`) and install it on the app for `session_id`.
fn install_registry(
    app: &tauri::App<tauri::test::MockRuntime>,
    session_id: &str,
) -> Arc<BackgroundAgentRegistry> {
    let orchestrator = Arc::new(AgentOrchestrator::new());
    let registry = Arc::new(BackgroundAgentRegistry::new(
        Arc::clone(&orchestrator),
        Arc::new(vec![def("writer"), def("reviewer")]),
    ));
    install_bg_session_for_test(&app.handle().clone(), session_id, Arc::clone(&registry));
    registry
}

// ---------------------------------------------------------------------------
// Authorization: F-051 / H10 parity with the other session-scoped commands.
// ---------------------------------------------------------------------------

#[test]
fn dashboard_window_invoking_start_background_agent_is_rejected() {
    let app = make_app();
    install_registry(&app, "sess-a");
    let window = make_window(&app, "dashboard");

    let err = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-a",
            "agentName": "writer",
            "prompt": "go",
        }),
    )
    .expect_err("dashboard window must not start bg agents");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn cross_session_start_is_rejected_with_label_mismatch() {
    // Session-A webview tries to start a bg agent under session-B.
    // Authz fires before bridge lookup → we see LABEL_MISMATCH, never a
    // "missing registry" fall-through.
    let app = make_app();
    install_registry(&app, "session-B");
    let window = make_window(&app, "session-A");

    let err = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "session-B",
            "agentName": "writer",
            "prompt": "go",
        }),
    )
    .expect_err("session-A must not start agents under session-B");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn cross_session_list_is_rejected_with_label_mismatch() {
    let app = make_app();
    install_registry(&app, "session-B");
    let window = make_window(&app, "session-A");

    let err = invoke(
        &window,
        "list_background_agents",
        serde_json::json!({ "sessionId": "session-B" }),
    )
    .expect_err("session-A must not list session-B's bg agents");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn cross_session_promote_is_rejected_with_label_mismatch() {
    let app = make_app();
    install_registry(&app, "session-B");
    let window = make_window(&app, "session-A");

    let err = invoke(
        &window,
        "promote_background_agent",
        serde_json::json!({
            "sessionId": "session-B",
            "instanceId": "deadbeefcafebabe",
        }),
    )
    .expect_err("session-A must not promote session-B's agents");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

#[test]
fn cross_session_stop_is_rejected_with_label_mismatch() {
    // F-138: session-A webview must not stop session-B's background agents.
    // Authz fires before `orchestrator.stop`, so we see LABEL_MISMATCH rather
    // than a "missing registry" fall-through.
    let app = make_app();
    install_registry(&app, "session-B");
    let window = make_window(&app, "session-A");

    let err = invoke(
        &window,
        "stop_background_agent",
        serde_json::json!({
            "sessionId": "session-B",
            "instanceId": "deadbeefcafebabe",
        }),
    )
    .expect_err("session-A must not stop session-B's agents");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// DoD: start + list + completion event forwarding + promote round trip.
// ---------------------------------------------------------------------------

#[test]
fn start_registers_an_instance_and_list_surfaces_it_as_running() {
    let app = make_app();
    install_registry(&app, "sess-start");
    let window = make_window(&app, "session-sess-start");

    let start_res = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-start",
            "agentName": "writer",
            "prompt": "draft release notes",
        }),
    )
    .expect("start must succeed under matching label with registered def");

    let start_json = start_res
        .deserialize::<serde_json::Value>()
        .expect("start returns a JSON string id");
    let instance_id = start_json.as_str().expect("id should be a string");
    assert_eq!(
        instance_id.len(),
        16,
        "AgentInstanceId hex is 16 chars, got: {instance_id}"
    );

    // list_background_agents must surface the new instance with agent_name
    // and Running state. The forwarded ts-rs type is `BgAgentSummary`.
    let list_res = invoke(
        &window,
        "list_background_agents",
        serde_json::json!({ "sessionId": "sess-start" }),
    )
    .expect("list must succeed");
    let rows: serde_json::Value = list_res.deserialize().unwrap();
    let arr = rows.as_array().expect("list returns an array");
    assert_eq!(arr.len(), 1, "exactly one running bg agent; got: {rows}");
    assert_eq!(arr[0]["id"], instance_id);
    assert_eq!(arr[0]["agent_name"], "writer");
    assert_eq!(arr[0]["state"], "Running");
}

#[test]
fn unknown_agent_name_returns_typed_error_not_panic() {
    let app = make_app();
    install_registry(&app, "sess-unknown");
    let window = make_window(&app, "session-sess-unknown");

    let err = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-unknown",
            "agentName": "does-not-exist",
            "prompt": "go",
        }),
    )
    .expect_err("unknown agent must return a typed error");
    assert!(
        err.contains("unknown agent") && err.contains("does-not-exist"),
        "expected typed unknown-agent error, got: {err}"
    );
}

/// DoD: "`BackgroundAgentStarted` and `BackgroundAgentCompleted` events
/// forward to the webview IPC." The forwarder task spawned at
/// registry-install time re-emits the registry's local broadcast events
/// onto the session's `session:event` channel so the frontend sees both
/// lifecycle events without needing to subscribe to a second channel.
///
/// We drive completion by calling `orchestrator.stop(id)` directly on the
/// registry's orchestrator handle — there is no step executor yet, and
/// exposing `Orchestrator::stop` on the trait is deliberate so tests (and a
/// future `stop_background_agent` command) have one uniform path.
#[tokio::test(flavor = "multi_thread")]
async fn start_and_completion_events_reach_the_webview_via_session_event_channel() {
    use std::sync::Mutex;

    let app = make_app();
    let registry = install_registry(&app, "sess-events");
    let window = make_window(&app, "session-sess-events");

    // Install the listener BEFORE invoking `start` so the `Started` event
    // is not lost to a listener-registration race. Tauri's `listen` is
    // synchronous and hooks up before the next invoke emits anything, but
    // the forwarder is an independent tokio task — a window listener set
    // up after `start` returns can miss the event that was already emitted.
    let collected: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let saw_completed: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let collected_ev = Arc::clone(&collected);
    let completed_flag = Arc::clone(&saw_completed);
    let _listener = window.listen("session:event", move |ev| {
        let payload: Value = match serde_json::from_str(ev.payload()) {
            Ok(v) => v,
            Err(_) => return,
        };
        if payload["event"]["type"] == "background_agent_completed" {
            *completed_flag.lock().unwrap() = true;
        }
        collected_ev.lock().unwrap().push(payload);
    });

    // Start the agent via IPC — this triggers `BackgroundAgentStarted`
    // through the forwarder.
    let start_res = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-events",
            "agentName": "writer",
            "prompt": "work through the queue",
        }),
    )
    .expect("start must succeed");
    let instance_id = start_res
        .deserialize::<serde_json::Value>()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    let id = forge_core::AgentInstanceId::from_string(instance_id.clone());

    // Drive completion directly on the orchestrator. The registry's
    // internal forwarder observes the orchestrator lifecycle stream and
    // converts the `Completed` event into `Event::BackgroundAgentCompleted`
    // on its local channel, which the per-session webview forwarder then
    // re-emits onto `session:event`.
    registry.orchestrator().stop(&id).await.unwrap();

    // Poll until we observe completion or hit the 2 s ceiling. The two
    // broadcast hops (registry forwarder → per-session forwarder → Tauri
    // event channel) race against the test's current thread; `yield_now`
    // lets background tasks make progress before we check.
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if *saw_completed.lock().unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let events = collected.lock().unwrap().clone();
    let mut saw_started = false;
    let mut saw_completed_local = false;
    for payload in &events {
        let ty = payload["event"]["type"].as_str().unwrap_or("");
        let ev_id = payload["event"]["id"].as_str().unwrap_or("");
        match ty {
            "background_agent_started" if ev_id == instance_id => saw_started = true,
            "background_agent_completed" if ev_id == instance_id => saw_completed_local = true,
            _ => {}
        }
    }

    assert!(
        saw_started,
        "BackgroundAgentStarted must forward to the session:event channel; got events: {events:?}"
    );
    assert!(
        saw_completed_local,
        "BackgroundAgentCompleted must forward after orchestrator.stop — got events: {events:?}"
    );

    // After completion, list must be empty (tracked set was cleared by the
    // registry forwarder when it observed the terminal event).
    let list_res = invoke(
        &window,
        "list_background_agents",
        serde_json::json!({ "sessionId": "sess-events" }),
    )
    .expect("list must succeed post-completion");
    let rows: serde_json::Value = list_res.deserialize().unwrap();
    assert_eq!(
        rows.as_array().unwrap().len(),
        0,
        "completed agents must drop from list, got: {rows}"
    );
}

#[test]
fn promote_removes_from_list_without_stopping_underlying_instance() {
    let app = make_app();
    let registry = install_registry(&app, "sess-promote");
    let window = make_window(&app, "session-sess-promote");

    let start_res = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-promote",
            "agentName": "writer",
            "prompt": "p",
        }),
    )
    .expect("start must succeed");
    let instance_id = start_res
        .deserialize::<serde_json::Value>()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // Pre-promote: list includes the id.
    let rows: serde_json::Value = invoke(
        &window,
        "list_background_agents",
        serde_json::json!({ "sessionId": "sess-promote" }),
    )
    .unwrap()
    .deserialize()
    .unwrap();
    assert_eq!(rows.as_array().unwrap().len(), 1);

    // Promote.
    invoke(
        &window,
        "promote_background_agent",
        serde_json::json!({
            "sessionId": "sess-promote",
            "instanceId": instance_id.clone(),
        }),
    )
    .expect("promote must succeed");

    // Post-promote: list is empty, but the underlying orchestrator
    // instance is still registered (promotion is UX re-attribution, not a
    // lifecycle transition).
    let rows: serde_json::Value = invoke(
        &window,
        "list_background_agents",
        serde_json::json!({ "sessionId": "sess-promote" }),
    )
    .unwrap()
    .deserialize()
    .unwrap();
    assert_eq!(
        rows.as_array().unwrap().len(),
        0,
        "promoted agent must drop from list"
    );

    let id = forge_core::AgentInstanceId::from_string(instance_id);
    let still_alive = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async move { registry.orchestrator().get(&id).await });
    assert!(
        still_alive.is_some(),
        "promote must not stop the orchestrator instance — UX re-attribution only"
    );
}

/// F-138: `stop_background_agent` drives `Orchestrator::stop(id)` through the
/// registry. Because the registry's forwarder already converts a terminal
/// lifecycle event into `BackgroundAgentCompleted` and drops the id from the
/// tracked set, a successful stop must (a) return Ok, (b) emit the
/// `background_agent_completed` event on `session:event`, and (c) drop the id
/// from `list_background_agents`.
#[tokio::test(flavor = "multi_thread")]
async fn stop_completes_instance_and_emits_completion_event() {
    use std::sync::Mutex;

    let app = make_app();
    install_registry(&app, "sess-stop");
    let window = make_window(&app, "session-sess-stop");

    let collected: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let saw_completed: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
    let collected_ev = Arc::clone(&collected);
    let completed_flag = Arc::clone(&saw_completed);
    let _listener = window.listen("session:event", move |ev| {
        let payload: Value = match serde_json::from_str(ev.payload()) {
            Ok(v) => v,
            Err(_) => return,
        };
        if payload["event"]["type"] == "background_agent_completed" {
            *completed_flag.lock().unwrap() = true;
        }
        collected_ev.lock().unwrap().push(payload);
    });

    let start_res = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-stop",
            "agentName": "writer",
            "prompt": "keep working",
        }),
    )
    .expect("start must succeed");
    let instance_id = start_res
        .deserialize::<serde_json::Value>()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    invoke(
        &window,
        "stop_background_agent",
        serde_json::json!({
            "sessionId": "sess-stop",
            "instanceId": instance_id.clone(),
        }),
    )
    .expect("stop must succeed under matching label");

    // Poll for the completion event (two broadcast hops need to drain).
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if *saw_completed.lock().unwrap() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    let events = collected.lock().unwrap().clone();
    let saw = events.iter().any(|p| {
        p["event"]["type"] == "background_agent_completed"
            && p["event"]["id"].as_str() == Some(instance_id.as_str())
    });
    assert!(
        saw,
        "stop_background_agent must fan out BackgroundAgentCompleted on session:event; got: {events:?}"
    );

    // list must no longer surface the id — the registry's forwarder drops it
    // when it observes the orchestrator's terminal event.
    let list_res = invoke(
        &window,
        "list_background_agents",
        serde_json::json!({ "sessionId": "sess-stop" }),
    )
    .expect("list must succeed post-stop");
    let rows: serde_json::Value = list_res.deserialize().unwrap();
    assert_eq!(
        rows.as_array().unwrap().len(),
        0,
        "stopped agent must drop from list, got: {rows}"
    );
}

#[test]
fn stop_rejects_oversize_instance_id_at_command_layer() {
    let app = make_app();
    install_registry(&app, "sess-stop-oversize");
    let window = make_window(&app, "session-sess-stop-oversize");

    let err = invoke(
        &window,
        "stop_background_agent",
        serde_json::json!({
            "sessionId": "sess-stop-oversize",
            "instanceId": "A".repeat(1024),
        }),
    )
    .expect_err("oversize instance_id must be rejected by the size gate");
    assert!(
        err.contains("payload too large") && err.contains("instance_id"),
        "expected size-cap error mentioning instance_id, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// F-365: explicit DoD coverage for `stop_background_agent`.
//
// The sibling tests above (`stop_completes_instance_and_emits_completion_event`
// + `cross_session_stop_is_rejected_with_label_mismatch`) cover the event
// fan-out and window-label authz. These two add the remaining gaps the
// F-365 review called out:
//
// 1. `stop_background_agent_transitions_instance_to_terminal_and_stops_sampling`
//    asserts that a successful stop, driven through the IPC command, (a) flips
//    the instance to terminal and (b) causes the registry's forwarder to
//    untrack the instance from the resource monitor. Without this, a regression
//    that decoupled the forwarder from `monitor.untrack` would leak sampler
//    tasks and produce misleading `list` rows.
// 2. `stop_background_agent_from_wrong_session_is_rejected` mirrors the
//    existing `delete_branch` / promote authz shape — the window label is a
//    session window, but the payload's `sessionId` targets a different session.
//    The label check must reject this before the bridge resolves a registry.
// ---------------------------------------------------------------------------

/// DoD: flip the registry to `Completed` through `stop_background_agent`,
/// assert the monitor untracks so future samples stop.
#[tokio::test(flavor = "multi_thread")]
async fn stop_background_agent_transitions_instance_to_terminal_and_stops_sampling() {
    use forge_agents::Orchestrator as AgentOrchestrator;
    use forge_session::{fake_sample, FakeSampler, ResourceMonitor};

    let app = make_app();

    // Fast-tick monitor with a scripted FakeSampler so we can observe the
    // `tracked_count` drop without waiting for the 1 s production cadence.
    let orchestrator = Arc::new(AgentOrchestrator::new());
    let fake = Arc::new(FakeSampler::new(fake_sample(0.0, Some(0), Some(0))));
    let monitor = Arc::new(ResourceMonitor::new(
        fake as Arc<dyn forge_session::Sampler>,
        Duration::from_millis(20),
    ));
    let registry = Arc::new(BackgroundAgentRegistry::with_monitor(
        Arc::clone(&orchestrator),
        Arc::new(vec![def("writer"), def("reviewer")]),
        Arc::clone(&monitor),
    ));
    install_bg_session_for_test(&app.handle().clone(), "sess-term", Arc::clone(&registry));
    let window = make_window(&app, "session-sess-term");

    let start_res = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-term",
            "agentName": "writer",
            "prompt": "work",
        }),
    )
    .expect("start must succeed");
    let instance_id = start_res
        .deserialize::<serde_json::Value>()
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // F-370: `start` itself does not register a sampler task — the
    // daemon-PID guard in `ResourceMonitor::track` prevents misleading
    // per-instance pills while no real child PID exists. Stand in for
    // the future step executor by handing the monitor a non-daemon PID
    // directly so the `untrack(id) on terminal` invariant still has a
    // live task to abort.
    let instance_id_parsed = forge_core::AgentInstanceId::from_string(instance_id.clone());
    monitor
        .track(instance_id_parsed, std::process::id().wrapping_add(1))
        .await;
    let deadline = Instant::now() + Duration::from_secs(2);
    while monitor.tracked_count().await == 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(
        monitor.tracked_count().await,
        1,
        "monitor must track the instance once a real PID is supplied"
    );

    invoke(
        &window,
        "stop_background_agent",
        serde_json::json!({
            "sessionId": "sess-term",
            "instanceId": instance_id.clone(),
        }),
    )
    .expect("stop must succeed under matching label");

    // (a) Terminal transition: the registry's forwarder drops the id from
    // `list` once it observes the orchestrator's terminal event.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let rows: serde_json::Value = invoke(
            &window,
            "list_background_agents",
            serde_json::json!({ "sessionId": "sess-term" }),
        )
        .expect("list must succeed")
        .deserialize()
        .unwrap();
        if rows.as_array().unwrap().is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            panic!("terminal transition did not drop id from list: {rows}");
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    // (b) Sampler is released: `monitor.untrack(id)` ran in the forwarder and
    // the sampler task is gone. `tracked_count` is the observable proxy for
    // "sampler not leaking".
    let deadline = Instant::now() + Duration::from_secs(2);
    while monitor.tracked_count().await != 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        monitor.tracked_count().await,
        0,
        "monitor must untrack the instance on terminal transition — otherwise \
         the sampler task leaks and `list` rows are misleading"
    );
}

/// DoD: `stop_background_agent` from a session-A window with a payload
/// claiming session-B must be rejected by the window-label gate before any
/// registry lookup happens.
#[test]
fn stop_background_agent_from_wrong_session_is_rejected() {
    let app = make_app();
    install_registry(&app, "session-owner");
    let window = make_window(&app, "session-attacker");

    let err = invoke(
        &window,
        "stop_background_agent",
        serde_json::json!({
            "sessionId": "session-owner",
            "instanceId": "deadbeefcafebabe",
        }),
    )
    .expect_err("a session-attacker window must not stop session-owner's agents");
    assert!(
        err.contains(LABEL_MISMATCH),
        "expected label-mismatch error, got: {err}"
    );
}

/// Size-cap regression: oversize prompts must be rejected at the command
/// layer (`require_size`) before the registry's `start` allocates anything.
#[test]
fn start_rejects_oversize_prompt_at_command_layer() {
    let app = make_app();
    install_registry(&app, "sess-oversize");
    let window = make_window(&app, "session-sess-oversize");

    let err = invoke(
        &window,
        "start_background_agent",
        serde_json::json!({
            "sessionId": "sess-oversize",
            "agentName": "writer",
            "prompt": "A".repeat(1024 * 1024),
        }),
    )
    .expect_err("oversize prompt must be rejected");
    assert!(
        err.contains("payload too large") && err.contains("prompt"),
        "expected size-cap error mentioning prompt, got: {err}"
    );
    // `unknown agent` would prove the cap fired after the registry — we
    // must see it fire before.
    assert!(
        !err.contains("unknown agent"),
        "size check must fire before the registry, got: {err}"
    );
}
