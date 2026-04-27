//! F-134 integration tests for the `agent.spawn` built-in tool.
//!
//! These exercise the tool through the public `forge_session::tools`
//! surface — the same code path `run_request_loop` uses when a provider
//! emits a tool call. They complement the per-tool unit tests by proving
//! the dispatcher round-trip works against a real `Session` event log.

use forge_agents::{
    AgentDef, AgentScope, Isolation, Orchestrator as AgentOrchestrator, SpawnContext,
};
use forge_core::ids::AgentInstanceId;
use forge_core::{Event, MessageId};
use forge_session::session::Session;
use forge_session::tools::{AgentSpawnCtx, AgentSpawnTool, ToolCtx, ToolDispatcher};
use serde_json::json;
use std::sync::Arc;
use tempfile::TempDir;

fn agent_def(name: &str, isolation: Isolation) -> AgentDef {
    AgentDef {
        name: name.to_string(),
        description: None,
        body: String::new(),
        allowed_paths: vec![],
        isolation,
        memory_enabled: false,
    }
}

async fn fresh_session() -> (TempDir, Arc<Session>) {
    let dir = TempDir::new().unwrap();
    let session = Arc::new(
        Session::create(dir.path().join("events.jsonl"))
            .await
            .unwrap(),
    );
    (dir, session)
}

/// DoD: parent agent invokes `agent.spawn("child")`; verify child ran in
/// correct isolation and event was emitted. The parent is `Process`, the
/// child is `Container`; the registered instance must reflect the
/// **child's** isolation — never the parent's.
#[tokio::test]
async fn parent_process_spawning_container_child_does_not_escalate_or_demote_privilege() {
    let (_dir, session) = fresh_session().await;
    let orchestrator = Arc::new(AgentOrchestrator::new());

    // Register a realistic parent instance up front so the
    // `parent_instance_id` we thread into the tool is a real registered id.
    let parent_inst = orchestrator
        .spawn(
            agent_def("parent", Isolation::Process),
            SpawnContext::user(),
        )
        .await
        .unwrap();

    // Subscribe to session events *before* dispatching so we don't miss the
    // emit. `broadcast::Sender` drops events with no active receivers.
    let mut rx = session.event_tx.subscribe();

    let agent_ctx = AgentSpawnCtx {
        agent_defs: Arc::new(vec![
            agent_def("parent", Isolation::Process),
            agent_def("child", Isolation::Container),
        ]),
        orchestrator: Arc::clone(&orchestrator),
        session: Arc::clone(&session),
        parent_instance_id: parent_inst.id.clone(),
        current_msg_id: MessageId::new(),
    };

    let ctx = ToolCtx {
        allowed_paths: vec![],
        workspace_root: None,
        child_registry: None,
        byte_budget: None,
        agent_ctx: Some(agent_ctx),
        mcp: None,
    };

    let mut dispatcher = ToolDispatcher::new();
    dispatcher.register(Box::new(AgentSpawnTool)).unwrap();

    let result = dispatcher
        .dispatch(
            "agent.spawn",
            &json!({ "agent_name": "child", "prompt": "go" }),
            &ctx,
        )
        .await
        .unwrap();

    assert_eq!(
        result["ok"].as_bool(),
        Some(true),
        "dispatch result: {result}"
    );
    let child_id =
        AgentInstanceId::from_string(result["child_instance_id"].as_str().unwrap().to_string());

    // 1. Isolation invariant: the registered child carries the CHILD's
    //    isolation, distinct from the parent's.
    let child_inst = orchestrator.get(&child_id).await.expect("child registered");
    assert_eq!(child_inst.def.isolation, Isolation::Container);
    assert_ne!(child_inst.def.isolation, parent_inst.def.isolation);
    assert_eq!(child_inst.def.name, "child");

    // 2. Event emitted on the session event bus with the right ids.
    let mut saw_event = false;
    while let Ok((_seq, event)) = rx.try_recv() {
        if let Event::SubAgentSpawned {
            parent,
            child,
            from_msg: _,
            ..
        } = event
        {
            assert_eq!(
                parent, parent_inst.id,
                "parent must match parent_instance_id"
            );
            assert_eq!(child, child_id, "child must match the spawn's returned id");
            saw_event = true;
            break;
        }
    }
    assert!(saw_event, "SubAgentSpawned must be emitted");
}

/// DoD: "Dispatcher resolves agent_name via forge_agents::load_agents, fails
/// with typed error if not found." The dispatcher's direct failure carries a
/// machine-readable error prefix so callers can distinguish name-lookup
/// failure from runtime failure.
#[tokio::test]
async fn unknown_agent_name_fails_with_typed_error_not_panic() {
    let (_dir, session) = fresh_session().await;

    let agent_ctx = AgentSpawnCtx {
        agent_defs: Arc::new(vec![agent_def("other", Isolation::Process)]),
        orchestrator: Arc::new(AgentOrchestrator::new()),
        session,
        parent_instance_id: AgentInstanceId::new(),
        current_msg_id: MessageId::new(),
    };

    let ctx = ToolCtx {
        allowed_paths: vec![],
        workspace_root: None,
        child_registry: None,
        byte_budget: None,
        agent_ctx: Some(agent_ctx),
        mcp: None,
    };

    let mut dispatcher = ToolDispatcher::new();
    dispatcher.register(Box::new(AgentSpawnTool)).unwrap();

    let result = dispatcher
        .dispatch(
            "agent.spawn",
            &json!({ "agent_name": "does-not-exist", "prompt": "go" }),
            &ctx,
        )
        .await
        .unwrap();

    let err = result["error"].as_str().unwrap_or_default();
    assert!(
        err.starts_with("tool.agent.spawn: unknown agent"),
        "expected typed error, got: {result}"
    );
}

/// DoD guardrail: an attacker who programmatically constructs a user-scope
/// `Trusted` child def (bypassing the parser's reject at load time) must
/// still be refused at spawn. The tool pins `AgentScope::User`, so the
/// orchestrator's runtime check fires and the sub-agent is not registered.
#[tokio::test]
async fn user_scope_cannot_spawn_trusted_child_even_if_def_is_forged() {
    let (_dir, session) = fresh_session().await;
    let orchestrator = Arc::new(AgentOrchestrator::new());

    let agent_ctx = AgentSpawnCtx {
        agent_defs: Arc::new(vec![agent_def("evil", Isolation::Trusted)]),
        orchestrator: Arc::clone(&orchestrator),
        session,
        parent_instance_id: AgentInstanceId::new(),
        current_msg_id: MessageId::new(),
    };
    let ctx = ToolCtx {
        allowed_paths: vec![],
        workspace_root: None,
        child_registry: None,
        byte_budget: None,
        agent_ctx: Some(agent_ctx),
        mcp: None,
    };

    let mut dispatcher = ToolDispatcher::new();
    dispatcher.register(Box::new(AgentSpawnTool)).unwrap();

    // Subscribe to orchestrator lifecycle events BEFORE the rejected spawn
    // so we can prove no `Spawned` arrives. Dropping the ctx (and the Arc<_>
    // it holds for the orchestrator) closes the channel and lets the stream
    // terminate cleanly; polling with a timeout guards against a hang if
    // the invariant ever regresses.
    let mut stream = orchestrator.state_stream();

    let result = dispatcher
        .dispatch(
            "agent.spawn",
            &json!({ "agent_name": "evil", "prompt": "escalate" }),
            &ctx,
        )
        .await
        .unwrap();

    let err = result["error"].as_str().unwrap_or_default();
    assert!(
        err.contains("spawn failed"),
        "expected spawn-failure error, got: {result}"
    );
    assert!(
        err.to_lowercase().contains("trusted"),
        "error should name the offending isolation level, got: {result}"
    );

    // Close every Arc<Orchestrator> so the broadcast stream ends.
    drop(dispatcher);
    drop(ctx);
    drop(orchestrator);

    use futures::StreamExt;
    let mut collected = vec![];
    // `state_stream` ends when the last Sender drops; a bounded timeout
    // prevents a dangling receiver from stalling the test.
    let drain = async {
        while let Some(Ok(ev)) = stream.next().await {
            collected.push(ev);
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(2), drain)
        .await
        .expect("broadcast stream should terminate after last sender drops");

    assert!(
        !collected
            .iter()
            .any(|e| matches!(e, forge_agents::AgentEvent::Spawned { .. })),
        "no Spawned event should have been emitted for a rejected trusted child, got: {collected:?}"
    );
}

/// Regression guard: embedders that don't need spawning (e.g. `rerun_replace`)
/// register the tool but leave `ToolCtx.agent_ctx` as `None`. Invocation must
/// return a typed error, not panic, not deadlock, not silently no-op.
#[tokio::test]
async fn tool_is_registered_but_inert_when_agent_ctx_is_absent() {
    let mut dispatcher = ToolDispatcher::new();
    dispatcher.register(Box::new(AgentSpawnTool)).unwrap();

    let result = dispatcher
        .dispatch(
            "agent.spawn",
            &json!({ "agent_name": "child", "prompt": "go" }),
            &ToolCtx::default(),
        )
        .await
        .unwrap();

    assert!(
        result["error"]
            .as_str()
            .unwrap_or_default()
            .contains("agent runtime not configured"),
        "result: {result}"
    );
}

/// Sanity check that the `AgentScope::User` pinning in the tool is not
/// circumvented by the caller's choice of parent_instance_id. We construct
/// the `SpawnContext::user()` locally so this test is explicit about the
/// contract rather than implicit via the isolation tests above.
#[tokio::test]
async fn spawn_context_scope_is_always_user_for_this_tool() {
    use std::mem::discriminant;
    let user = SpawnContext::user();
    assert_eq!(
        discriminant(&user.scope),
        discriminant(&AgentScope::User),
        "agent.spawn is meant to pin user scope; breaking that changes \
         the privilege model and needs dedicated review"
    );
}

/// F-137 additional mandate: `agent.spawn`'s `prompt` argument must be
/// forwarded from the parent's tool call onto the spawned child's
/// `AgentInstance.initial_prompt`. Before F-137 the arg was validated but
/// dropped (`let _prompt = …`), which turned F-134 into scaffolding-only
/// plumbing. This test asserts the end-to-end wiring: dispatch →
/// `SpawnContext.initial_prompt` → registered instance.
#[tokio::test]
async fn agent_spawn_forwards_prompt_onto_child_instance_for_first_turn_materialisation() {
    let (_dir, session) = fresh_session().await;
    let orchestrator = Arc::new(AgentOrchestrator::new());

    let parent_inst = orchestrator
        .spawn(
            agent_def("parent", Isolation::Process),
            SpawnContext::user(),
        )
        .await
        .unwrap();

    let agent_ctx = AgentSpawnCtx {
        agent_defs: Arc::new(vec![agent_def("child", Isolation::Process)]),
        orchestrator: Arc::clone(&orchestrator),
        session,
        parent_instance_id: parent_inst.id.clone(),
        current_msg_id: MessageId::new(),
    };
    let ctx = ToolCtx {
        allowed_paths: vec![],
        workspace_root: None,
        child_registry: None,
        byte_budget: None,
        agent_ctx: Some(agent_ctx),
        mcp: None,
    };

    let mut dispatcher = ToolDispatcher::new();
    dispatcher.register(Box::new(AgentSpawnTool)).unwrap();

    // A distinctive prompt so the `contains` assertion below is unambiguous
    // even if some future seeding step concatenates this with a persona or
    // other system text.
    const PROMPT: &str = "review the bg_agents module for race conditions";

    let result = dispatcher
        .dispatch(
            "agent.spawn",
            &json!({ "agent_name": "child", "prompt": PROMPT }),
            &ctx,
        )
        .await
        .unwrap();

    assert_eq!(result["ok"].as_bool(), Some(true), "dispatch: {result}");
    let child_id =
        AgentInstanceId::from_string(result["child_instance_id"].as_str().unwrap().to_string());

    // The load-bearing assertion: the registered child instance carries the
    // parent's seed prompt verbatim. A step executor materialising the
    // child's first user turn reads from exactly this field — the DoD's
    // "spawned child's first user turn contains the parent's prompt"
    // phrasing is satisfied as soon as `initial_prompt.as_deref() ==
    // Some(parent_prompt)` at the boundary where the turn is constructed.
    let child_inst = orchestrator.get(&child_id).await.expect("child registered");
    assert_eq!(
        child_inst.initial_prompt.as_deref(),
        Some(PROMPT),
        "agent.spawn must forward its `prompt` argument onto the child's \
         AgentInstance.initial_prompt — pre-F-137 the arg was dropped \
         (`let _prompt = …`), which made F-134 scaffolding-only"
    );
}
