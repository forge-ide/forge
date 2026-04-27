use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use chrono::Utc;
use forge_core::{
    apply_superseded,
    credentials::Credentials,
    ids::{MessageId, ProviderId, StepId, ToolCallId},
    read_since, ApprovalScope, ApprovalSource, Event, RerunVariant, StepKind, StepOutcome,
};
use forge_providers::{ChatBlock, ChatChunk, ChatMessage, ChatRequest, ChatRole, Provider};
use futures::StreamExt;
use tokio::sync::{oneshot, Mutex};

/// F-587: per-turn credential pull binding.
///
/// `run_turn` consults `store.get(provider_id)` exactly once at turn start
/// (before the request loop opens the model step). The result is **not**
/// passed into the request loop — the [`Provider`] trait doesn't yet take
/// per-turn auth (Phase 1 ships a keyless `OllamaProvider`; Anthropic /
/// OpenAI providers are wiring this as their auth-injection seam).
///
/// What lands today:
///
/// * The orchestrator pulls the credential on every turn (DoD #5).
/// * A `tracing::trace!` records hit / miss with `provider_id` only —
///   never the value, never even at `debug` level.
/// * Errors from the store propagate as `Err(_)` so a hostile or broken
///   keyring backend fails the turn instead of silently downgrading to
///   keyless.
///
/// When provider-level auth lands, the call site in `run_turn` will hand
/// `credential` into the provider's per-turn auth shape; nothing else
/// about this struct should need to change.
#[derive(Clone)]
pub struct CredentialContext {
    pub store: Arc<dyn Credentials>,
    pub provider_id: String,
}

impl std::fmt::Debug for CredentialContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately omits `store` (Box<dyn _> doesn't `Debug` well) and
        // never prints the credential value. `provider_id` is non-secret
        // by contract.
        f.debug_struct("CredentialContext")
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

use crate::byte_budget::ByteBudget;
use crate::dispatcher_cache::DispatcherCache;
use crate::sandbox::ChildRegistry;
use crate::session::Session;
use crate::tools::{
    AgentRuntime, AgentSpawnCtx, AgentSpawnTool, FsEditTool, FsReadTool, FsWriteTool, McpTool,
    ShellExecTool, ToolCtx, ToolDispatcher, ToolError,
};
use forge_mcp::McpManager;

/// Client decision for a pending tool call approval. Carries the
/// client-supplied `ApprovalScope` on approval so the event log records
/// the scope faithfully (F-053); `Rejected` collapses scope since there's
/// nothing to carry forward.
#[derive(Debug, Clone)]
pub enum ApprovalDecision {
    Approved(ApprovalScope),
    Rejected,
}

/// Pending tool call approvals: maps ToolCallId → sender for the approval result.
pub type PendingApprovals = Arc<Mutex<HashMap<String, oneshot::Sender<ApprovalDecision>>>>;

/// F-139: short SHA-256 prefix of a JSON-serialized args payload.
///
/// Used on `ToolInvoked` so downstream consumers can correlate a tool
/// invocation with the matching `ToolCallStarted.args` without shipping
/// the payload twice. 8 hex chars (32 bits) is ample for UI correlation
/// within a single turn — collisions are not a security boundary here.
fn args_digest(args: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(args).unwrap_or_default();
    let full = Sha256::digest(&bytes);
    let mut s = String::with_capacity(8);
    for b in full.iter().take(4) {
        use std::fmt::Write as _;
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// Run a complete turn for the given user text. Emits all session events for:
/// UserMessage → StepStarted(Model) → AssistantMessage(open) → AssistantDelta* →
/// [StepStarted(Tool) → ToolCallStarted → ToolCallApprovalRequested →
///  ToolCallApproved → ToolInvoked → ToolReturned → ToolCallCompleted →
///  StepFinished(Tool)]* → AssistantMessage(finalised) → StepFinished(Model)
///
/// F-139 ordering invariant (pinned by `tests/step_events.rs`):
///  * every `StepStarted` is followed by exactly one `StepFinished` with
///    the same `step_id`, in LIFO order relative to other open steps;
///  * `ToolInvoked` / `ToolReturned` for a given `step_id` fall strictly
///    between that step's `StepStarted` and `StepFinished`;
///  * `AssistantMessage` / `AssistantDelta` for a turn fall inside the
///    enclosing `Model` step's window;
///  * on abnormal exits (stream error, tool rejection) the inner step is
///    closed with `StepOutcome::Error` and the outer Model step is
///    closed as `Ok` before the function returns — so replay never sees
///    an unterminated step window.
///
/// Tool calls block until the client sends `ToolCallApproved` / `ToolCallRejected`
/// through `pending_approvals`. `allowed_paths` is the set of glob patterns the
/// agent is permitted to access via `fs.read`.
///
/// `agents_md` is the cached workspace `AGENTS.md` contents (loaded once at
/// session start in `serve_with_session`). When `Some`, its contents are
/// injected into `ChatRequest.system` as a labeled section — see F-135.
#[allow(clippy::too_many_arguments)]
pub async fn run_turn<P: Provider>(
    session: Arc<Session>,
    provider: Arc<P>,
    text: String,
    pending_approvals: PendingApprovals,
    allowed_paths: Vec<String>,
    auto_approve: bool,
    workspace_root: Option<std::path::PathBuf>,
    child_registry: Option<ChildRegistry>,
    byte_budget: Option<Arc<ByteBudget>>,
    agents_md: Option<Arc<str>>,
    agent_runtime: Option<AgentRuntime>,
    mcp: Option<Arc<McpManager>>,
    // F-567: optional shared dispatcher cache. When `Some`, the cache hands
    // out a single `Arc<ToolDispatcher>` across turns and only rebuilds when
    // the MCP tools-list epoch advances — eliminating the per-turn HashMap
    // alloc, the M·T `McpTool` adapter allocations, and the `McpManager`
    // lock acquisition that previously sat on the time-to-first-token path.
    // `None` preserves the legacy "build a fresh dispatcher per turn"
    // behavior for tests and embedders without the cache wired up.
    dispatcher_cache: Option<Arc<DispatcherCache>>,
    // F-587: optional per-turn credential binding. When `Some`, the
    // orchestrator pulls the credential for `provider_id` from `store`
    // exactly once before the request loop opens. The pulled value is
    // never logged (trace-level only records hit/miss + provider_id), and
    // backend errors fail the turn rather than silently downgrading to
    // keyless. See [`CredentialContext`] for the seam contract.
    credentials: Option<CredentialContext>,
) -> Result<()> {
    // F-587: pull the credential for the active provider before any
    // model-side work begins. Today the value is held briefly and then
    // dropped — the Phase-3 Anthropic / OpenAI providers will plumb it
    // into their per-request auth headers; for the keyless `OllamaProvider`
    // shipping in Phase 1, the pull is a no-op-with-trace by design.
    //
    // Backend errors propagate. A misconfigured Secret Service daemon or a
    // locked Keychain is more useful as a turn-level failure than a silent
    // fall-through to "no auth" that the provider would later 401 on.
    if let Some(ctx) = credentials.as_ref() {
        let pulled = ctx.store.get(&ctx.provider_id).await?;
        tracing::trace!(
            target: "forge_session::orchestrator::credentials",
            provider_id = %ctx.provider_id,
            hit = pulled.is_some(),
            "credential pull",
        );
        // Drop `pulled` here — the value is intentionally not held longer
        // than necessary. When provider-level auth wiring lands, the value
        // is handed directly to the provider's request-builder via
        // `secrecy::ExposeSecret::expose_secret` at the network boundary.
        drop(pulled);
    }

    let msg_id = MessageId::new();

    session
        .emit(Event::UserMessage {
            id: msg_id.clone(),
            at: Utc::now(),
            // F-112: wrap at the forge-core boundary. When F-108 lands, the
            // upstream producer will hand us an `Arc<str>` and this becomes a
            // move; for now `Arc::from` is a single allocation (same count as
            // the previous `clone`).
            text: Arc::from(text.as_str()),
            context: vec![],
            branch_parent: None,
        })
        .await?;

    // F-135: Inject workspace `AGENTS.md` into the system prompt. Placement
    // follows the DoD: a leading `\n\n---\n` separator so a future base-persona
    // prepend slots cleanly before the labeled section.
    //
    // F-566: `agents_md` is now the **already-wrapped** labeled prefix, built
    // once at session start in `serve_with_session::build_system_prompt`. The
    // orchestrator clones the `Arc<str>` into `req.system` (refcount bump),
    // never the underlying bytes. Eliminates the per-turn `format!()` against
    // a potentially 256 KiB AGENTS.md.
    let system: Option<Arc<str>> = agents_md.clone();

    let initial_req = ChatRequest {
        system,
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ChatBlock::Text(text)],
        }],
        parallel_tool_calls_allowed: false,
    };

    // F-567: pull the dispatcher from the session-scoped cache when one is
    // wired (steady state: a single `Arc::clone`). When the cache is absent
    // — tests / embedders that pass `None` — fall back to the legacy
    // build-per-turn shape so the runtime contract on those paths stays
    // identical to pre-F-567 behavior.
    //
    // F-134 / F-132: builtins (`fs.read` / `fs.write` / `fs.edit` /
    // `shell.exec` / `agent.spawn`) plus one `McpTool` adapter per
    // currently-advertised tool. The cache snapshots at turn start (not
    // per-chunk) just like the original inline build, so a mid-turn
    // `tools/list` refresh can never mutate the dispatch table under the
    // running loop. Un-connected servers contribute zero tools — fail-open,
    // matches the prior behavior.
    let dispatcher: Arc<ToolDispatcher> = match dispatcher_cache.as_ref() {
        Some(cache) => cache.get(mcp.as_ref()).await,
        None => Arc::new(build_legacy_dispatcher(mcp.as_ref()).await),
    };

    let agent_ctx = agent_runtime.as_ref().map(|rt| AgentSpawnCtx {
        agent_defs: Arc::clone(&rt.agent_defs),
        orchestrator: Arc::clone(&rt.orchestrator),
        session: Arc::clone(&session),
        parent_instance_id: rt.parent_instance_id.clone(),
        current_msg_id: msg_id.clone(),
    });
    let instance_id = agent_runtime
        .as_ref()
        .map(|rt| rt.parent_instance_id.clone());
    let ctx = ToolCtx {
        allowed_paths,
        workspace_root,
        child_registry,
        byte_budget,
        agent_ctx,
        mcp,
    };

    run_request_loop(
        session,
        provider,
        initial_req,
        msg_id,
        None, // branch_parent — top-level turns are never a branch variant
        0,    // branch_variant_index — root position
        pending_approvals,
        dispatcher.as_ref(),
        &ctx,
        auto_approve,
        instance_id,
    )
    .await
}

/// F-567 fallback: build a fresh dispatcher when no [`DispatcherCache`] is
/// wired (tests, embedders that don't pass one). Mirrors the historical
/// inline body so behavior on the uncached path stays identical.
async fn build_legacy_dispatcher(mcp: Option<&Arc<McpManager>>) -> ToolDispatcher {
    let mut dispatcher = ToolDispatcher::new();
    dispatcher
        .register(Box::new(FsReadTool))
        .expect("fs.read must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(FsWriteTool))
        .expect("fs.write must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(FsEditTool))
        .expect("fs.edit must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(ShellExecTool))
        .expect("shell.exec must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(AgentSpawnTool))
        .expect("agent.spawn must register on a fresh dispatcher");

    if let Some(mgr) = mcp {
        for server in mgr.list().await {
            for tool in server.tools {
                if let Some(adapter) = McpTool::new(
                    tool.name.clone(),
                    tool.description,
                    tool.read_only,
                    mgr.clone(),
                ) {
                    let _ = dispatcher.register(Box::new(adapter));
                }
            }
        }
    }

    dispatcher
}

/// Drives the provider request loop for one logical turn.
/// On tool calls: waits for approval, executes stub, appends result to the
/// next request, and continues until the provider returns `Done` with no
/// pending tool calls.
///
/// `pub(crate)` so rerun paths (F-143+) can reuse the loop with a pre-built
/// `ChatRequest` and a pre-chosen `msg_id`, instead of going through
/// [`run_turn`] which synthesizes a fresh `UserMessage` event.
///
/// `branch_parent` / `branch_variant_index` (F-144) are threaded onto every
/// `AssistantMessage` event this loop emits for `msg_id`:
///   * `None` / `0` — top-level turn or the root variant of a branch point.
///   * `Some(root_id)` / `N >= 1` — a Branch-rerun generation; both the
///     original and this new message co-exist in the transcript. Consumers
///     choose which to display via `BranchSelected`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_request_loop<P: Provider>(
    session: Arc<Session>,
    provider: Arc<P>,
    mut req: ChatRequest,
    msg_id: MessageId,
    branch_parent: Option<MessageId>,
    branch_variant_index: u32,
    pending_approvals: PendingApprovals,
    dispatcher: &ToolDispatcher,
    ctx: &ToolCtx,
    auto_approve: bool,
    // F-140: populated with the session's root `AgentInstanceId` when the
    // caller has a wired `AgentRuntime`. Threaded onto every `StepStarted`
    // so the Agent Monitor can group a session's trace under a stable
    // parent id. `None` preserves the pre-F-140 behaviour for embedders
    // that don't have an agent runtime wired up.
    instance_id: Option<forge_core::ids::AgentInstanceId>,
) -> Result<()> {
    // Fixed provider/model identifiers for the mock provider.
    let provider_id = ProviderId::new();
    let model = "mock".to_string();

    loop {
        // F-139: open a `Model` step around each provider pass. The step
        // envelopes every event this iteration emits (AssistantMessage*,
        // AssistantDelta, Tool*) — downstream consumers (Agent Monitor,
        // replay readers) rely on StepStarted preceding any per-turn
        // event with the same step_id.
        //
        // F-140: `instance_id` carries the session's root `AgentInstanceId`
        // when a caller wired an `AgentRuntime`; callers without one (legacy
        // tests, embedders with no agent wiring) still emit `None`. The
        // Agent Monitor groups a session's trace under the stable id here.
        let model_step_id = StepId::new();
        let model_step_started = Instant::now();
        session
            .emit(Event::StepStarted {
                step_id: model_step_id.clone(),
                instance_id: instance_id.clone(),
                kind: StepKind::Model,
                started_at: Utc::now(),
            })
            .await?;

        // Emit AssistantMessage(open) before any chunks arrive — ensures the
        // event is present even when the first chunk is a tool call (not text).
        session
            .emit(Event::AssistantMessage {
                id: msg_id.clone(),
                provider: provider_id.clone(),
                model: model.clone(),
                at: Utc::now(),
                stream_finalised: false,
                // F-112: empty Arc<str> — no allocation (matches `Arc::<str>::from("")`).
                text: Arc::from(""),
                branch_parent: branch_parent.clone(),
                branch_variant_index,
            })
            .await?;

        let mut stream = provider.chat(req.clone()).await?;
        let mut assistant_text = String::new();
        // Separate accumulators for the assistant's tool-call blocks (added to the
        // assistant message) and tool-result blocks (added to the next user message).
        let mut tc_blocks: Vec<ChatBlock> = vec![];
        let mut tr_blocks: Vec<ChatBlock> = vec![];
        let mut had_tool_calls = false;

        while let Some(chunk) = stream.next().await {
            match chunk {
                ChatChunk::TextDelta(delta) => {
                    assistant_text.push_str(&delta);
                    session
                        .emit(Event::AssistantDelta {
                            id: msg_id.clone(),
                            at: Utc::now(),
                            // F-112: wrap the per-token String in Arc<str> at this
                            // boundary. Once F-108 lands, `parse_line` will hand
                            // us `Arc<str>` directly and this becomes a move.
                            delta: Arc::from(delta),
                        })
                        .await?;
                }

                ChatChunk::ToolCall { name, args } => {
                    had_tool_calls = true;
                    let call_id = ToolCallId::new();

                    // F-139: open a nested `Tool` step around this tool
                    // invocation. Nests inside the enclosing Model step
                    // and closes before we loop back to the next stream
                    // chunk. `StepFinished` carries the same `step_id`;
                    // `ToolInvoked` / `ToolReturned` reference it.
                    let tool_step_id = StepId::new();
                    let tool_step_started = Instant::now();
                    session
                        .emit(Event::StepStarted {
                            step_id: tool_step_id.clone(),
                            // F-140: same instance id as the enclosing Model
                            // step, so a tool step nests cleanly inside its
                            // parent step in the Agent Monitor timeline.
                            instance_id: instance_id.clone(),
                            kind: StepKind::Tool,
                            started_at: Utc::now(),
                        })
                        .await?;

                    session
                        .emit(Event::ToolCallStarted {
                            id: call_id.clone(),
                            msg: msg_id.clone(),
                            tool: name.clone(),
                            args: args.clone(),
                            at: Utc::now(),
                            parallel_group: None,
                        })
                        .await?;

                    let started = Instant::now();
                    let tool = dispatcher.get(&name);

                    let result = match tool {
                        Ok(tool) => {
                            // F-132: read-only tools (MCP tools whose
                            // `readOnlyHint` is `true`) bypass the
                            // user-approval prompt. We still emit a
                            // `ToolCallApproved` with `ApprovalSource::Auto`
                            // + `ApprovalScope::Once` so replay sees a
                            // terminated approval event — the invariant
                            // every `ToolCallStarted` eventually resolves
                            // via either Approved or Rejected holds.
                            if auto_approve || tool.read_only() {
                                session
                                    .emit(Event::ToolCallApproved {
                                        id: call_id.clone(),
                                        by: ApprovalSource::Auto,
                                        scope: ApprovalScope::Once,
                                        at: Utc::now(),
                                    })
                                    .await?;
                            } else {
                                let (tx, rx) = oneshot::channel::<ApprovalDecision>();
                                pending_approvals
                                    .lock()
                                    .await
                                    .insert(call_id.to_string(), tx);

                                session
                                    .emit(Event::ToolCallApprovalRequested {
                                        id: call_id.clone(),
                                        preview: tool.approval_preview(&args),
                                    })
                                    .await?;

                                // If the client drops the channel we treat it as a
                                // rejection — matches the pre-F-053 default.
                                let decision = rx.await.unwrap_or(ApprovalDecision::Rejected);

                                let scope = match decision {
                                    ApprovalDecision::Approved(scope) => scope,
                                    ApprovalDecision::Rejected => {
                                        session
                                            .emit(Event::ToolCallRejected {
                                                id: call_id,
                                                reason: Some("rejected by client".to_string()),
                                            })
                                            .await?;
                                        // F-139: close the Tool step with
                                        // an error outcome before unwinding.
                                        // The enclosing Model step is
                                        // closed just below, in the
                                        // AssistantMessage(final) path.
                                        session
                                            .emit(Event::StepFinished {
                                                step_id: tool_step_id.clone(),
                                                outcome: StepOutcome::Error {
                                                    reason: "rejected by client".to_string(),
                                                },
                                                duration_ms: tool_step_started.elapsed().as_millis()
                                                    as u64,
                                                token_usage: None,
                                            })
                                            .await?;
                                        session
                                            .emit(Event::AssistantMessage {
                                                id: msg_id.clone(),
                                                provider: provider_id.clone(),
                                                model: model.clone(),
                                                at: Utc::now(),
                                                stream_finalised: true,
                                                // F-112: wrap at boundary.
                                                text: Arc::from(assistant_text.as_str()),
                                                branch_parent: branch_parent.clone(),
                                                branch_variant_index,
                                            })
                                            .await?;
                                        // Close the Model step too so the
                                        // LIFO invariant holds even on
                                        // early return.
                                        session
                                            .emit(Event::StepFinished {
                                                step_id: model_step_id.clone(),
                                                outcome: StepOutcome::Ok,
                                                duration_ms: model_step_started
                                                    .elapsed()
                                                    .as_millis()
                                                    as u64,
                                                token_usage: None,
                                            })
                                            .await?;
                                        return Ok(());
                                    }
                                };

                                session
                                    .emit(Event::ToolCallApproved {
                                        id: call_id.clone(),
                                        by: ApprovalSource::User,
                                        scope,
                                        at: Utc::now(),
                                    })
                                    .await?;
                            }

                            // F-139: emit ToolInvoked at the approval→
                            // execution boundary — after approval logged,
                            // before the tool runs. `args_digest` is a
                            // short SHA-256 prefix; downstream consumers
                            // correlate with `ToolCallStarted.args`.
                            session
                                .emit(Event::ToolInvoked {
                                    step_id: tool_step_id.clone(),
                                    tool_call_id: call_id.clone(),
                                    tool_id: name.clone(),
                                    args_digest: args_digest(&args),
                                })
                                .await?;

                            tool.invoke(&args, ctx).await
                        }
                        Err(ToolError::UnknownTool(n)) => {
                            // Unknown / errored dispatcher lookups still
                            // emit ToolInvoked so the step window is
                            // bracketed even when no tool actually ran.
                            // `ok` on the subsequent ToolReturned will
                            // reflect the synthetic error payload.
                            session
                                .emit(Event::ToolInvoked {
                                    step_id: tool_step_id.clone(),
                                    tool_call_id: call_id.clone(),
                                    tool_id: name.clone(),
                                    args_digest: args_digest(&args),
                                })
                                .await?;
                            serde_json::json!({ "error": format!("unknown tool '{n}'") })
                        }
                        Err(e) => {
                            session
                                .emit(Event::ToolInvoked {
                                    step_id: tool_step_id.clone(),
                                    tool_call_id: call_id.clone(),
                                    tool_id: name.clone(),
                                    args_digest: args_digest(&args),
                                })
                                .await?;
                            serde_json::json!({ "error": e.to_string() })
                        }
                    };

                    let duration_ms = started.elapsed().as_millis() as u64;

                    // F-139: ToolReturned right after the invocation
                    // settled. `ok` = absence of a top-level `error` key
                    // on the result payload; `bytes_out` = byte length of
                    // the serialized result.
                    let result_bytes = serde_json::to_string(&result)
                        .map(|s| s.len() as u64)
                        .unwrap_or(0);
                    let result_ok = result.get("error").is_none();
                    session
                        .emit(Event::ToolReturned {
                            step_id: tool_step_id.clone(),
                            tool_call_id: call_id.clone(),
                            ok: result_ok,
                            bytes_out: result_bytes,
                        })
                        .await?;

                    session
                        .emit(Event::ToolCallCompleted {
                            id: call_id.clone(),
                            result: result.clone(),
                            duration_ms,
                            at: Utc::now(),
                        })
                        .await?;

                    // F-139: close the Tool step. `outcome` mirrors the
                    // result's top-level `error` field so consumers can
                    // filter failures without parsing the payload.
                    session
                        .emit(Event::StepFinished {
                            step_id: tool_step_id.clone(),
                            outcome: if result_ok {
                                StepOutcome::Ok
                            } else {
                                StepOutcome::Error {
                                    reason: result
                                        .get("error")
                                        .and_then(|v| v.as_str())
                                        .unwrap_or("tool returned error")
                                        .to_string(),
                                }
                            },
                            duration_ms: tool_step_started.elapsed().as_millis() as u64,
                            token_usage: None,
                        })
                        .await?;

                    tc_blocks.push(ChatBlock::ToolCall {
                        id: call_id.to_string(),
                        name: name.clone(),
                        args,
                    });
                    tr_blocks.push(ChatBlock::ToolResult {
                        id: call_id.to_string(),
                        result,
                    });
                }

                ChatChunk::Done(_) => {
                    // Always finalise the assistant message on Done, whether or not
                    // tool calls occurred in this turn.
                    session
                        .emit(Event::AssistantMessage {
                            id: msg_id.clone(),
                            provider: provider_id.clone(),
                            model: model.clone(),
                            at: Utc::now(),
                            stream_finalised: true,
                            // F-112: wrap at boundary.
                            text: Arc::from(assistant_text.as_str()),
                            branch_parent: branch_parent.clone(),
                            branch_variant_index,
                        })
                        .await?;

                    // F-139: close the Model step before returning or
                    // looping. We emit StepFinished *after* the final
                    // AssistantMessage so the step window contains every
                    // event it logically owns.
                    session
                        .emit(Event::StepFinished {
                            step_id: model_step_id.clone(),
                            outcome: StepOutcome::Ok,
                            duration_ms: model_step_started.elapsed().as_millis() as u64,
                            token_usage: None,
                        })
                        .await?;

                    if !had_tool_calls {
                        return Ok(());
                    }
                    // Tool calls happened — build continuation request and loop.
                    break;
                }

                ChatChunk::Error { kind, message } => {
                    // Stream aborted under a provider-layer bound (line cap,
                    // idle timeout, wall-clock timeout, transport error).
                    // Finalise whatever assistant text we streamed so the UI
                    // composer unwedges, then surface the failure.
                    session
                        .emit(Event::AssistantMessage {
                            id: msg_id.clone(),
                            provider: provider_id.clone(),
                            model: model.clone(),
                            at: Utc::now(),
                            stream_finalised: true,
                            // F-112: wrap at boundary.
                            text: Arc::from(assistant_text.as_str()),
                            branch_parent: branch_parent.clone(),
                            branch_variant_index,
                        })
                        .await?;
                    // F-139: close the Model step with an Error outcome
                    // so late-joining subscribers see a well-formed
                    // step window even on provider abort.
                    session
                        .emit(Event::StepFinished {
                            step_id: model_step_id.clone(),
                            outcome: StepOutcome::Error {
                                reason: format!("provider stream aborted ({kind:?}): {message}"),
                            },
                            duration_ms: model_step_started.elapsed().as_millis() as u64,
                            token_usage: None,
                        })
                        .await?;
                    return Err(anyhow::anyhow!(
                        "provider stream aborted ({kind:?}): {message}"
                    ));
                }
            }
        }

        if !had_tool_calls {
            // Stream ended without a Done chunk — finalise and complete.
            session
                .emit(Event::AssistantMessage {
                    id: msg_id.clone(),
                    provider: provider_id.clone(),
                    model: model.clone(),
                    at: Utc::now(),
                    stream_finalised: true,
                    // F-112: wrap at boundary.
                    text: Arc::from(assistant_text.as_str()),
                    branch_parent: branch_parent.clone(),
                    branch_variant_index,
                })
                .await?;
            // F-139: close Model step on the no-Done exit path too.
            session
                .emit(Event::StepFinished {
                    step_id: model_step_id.clone(),
                    outcome: StepOutcome::Ok,
                    duration_ms: model_step_started.elapsed().as_millis() as u64,
                    token_usage: None,
                })
                .await?;
            return Ok(());
        }

        // Build continuation: the assistant message includes the text + tool calls;
        // the tool results are a new user message.
        let mut assistant_content = vec![ChatBlock::Text(assistant_text)];
        assistant_content.extend(tc_blocks);

        req.messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content: assistant_content,
        });
        req.messages.push(ChatMessage {
            role: ChatRole::User,
            content: tr_blocks,
        });
    }
}

// ── F-143: Orchestrator + rerun_message ────────────────────────────────────

/// Top-level entry point for session-level operations that span beyond a
/// single user turn — today only `rerun_message`; F-144 (Branch) and
/// F-145 (Fresh) will extend it.
///
/// The type is zero-sized on purpose: it is a namespace / trait-like façade
/// for the operations documented in `docs/architecture/ipc-contracts.md §4.1`
/// and keeps `run_turn` (the per-turn free function used by `server.rs`)
/// untouched. When later features accumulate shared state, the struct can
/// carry fields without breaking the call-site shape.
#[derive(Debug, Default, Clone, Copy)]
pub struct Orchestrator;

impl Orchestrator {
    pub fn new() -> Self {
        Self
    }

    /// Re-run an existing assistant message.
    ///
    /// Three variants (see [`RerunVariant`]):
    ///
    /// * [`RerunVariant::Replace`] (F-143) — truncate logically at `msg_id`'s
    ///   assistant turn, regenerate, and emit `MessageSuperseded` so replay
    ///   hides the original.
    /// * [`RerunVariant::Branch`] (F-144) — keep both versions. Spawns a
    ///   new `AssistantMessage` with `branch_parent` threaded to the target's
    ///   branch root and `branch_variant_index = prev_max + 1`. Both the
    ///   original and the new message remain visible in replay; consumers
    ///   pick which to display via `BranchSelected`.
    /// * [`RerunVariant::Fresh`] (F-144) — truncate back to the originating
    ///   user message (discarding intermediate turns / tool calls) and
    ///   regenerate from that user message alone. The new AssistantMessage
    ///   is a new root (`branch_parent = None`); the original turn is
    ///   superseded via `MessageSuperseded`.
    ///
    /// Ordering matters for Replace / Fresh: the `MessageSuperseded` marker
    /// is emitted *after* the new assistant message is finalised. If
    /// regeneration errors mid-stream the marker is never written and the
    /// original message stays authoritative — we don't point the UI at a
    /// half-written new_id. For Branch the original is never hidden, so
    /// no supersede marker is emitted.
    #[allow(clippy::too_many_arguments)]
    pub async fn rerun_message<P: Provider>(
        &self,
        session: Arc<crate::session::Session>,
        provider: Arc<P>,
        msg_id: MessageId,
        variant: RerunVariant,
        pending_approvals: PendingApprovals,
        allowed_paths: Vec<String>,
        auto_approve: bool,
        workspace_root: Option<std::path::PathBuf>,
        child_registry: Option<crate::sandbox::ChildRegistry>,
        byte_budget: Option<Arc<crate::byte_budget::ByteBudget>>,
        agent_runtime: Option<AgentRuntime>,
    ) -> Result<MessageId> {
        match variant {
            RerunVariant::Replace => {
                self.rerun_replace(
                    session,
                    provider,
                    msg_id,
                    pending_approvals,
                    allowed_paths,
                    auto_approve,
                    workspace_root,
                    child_registry,
                    byte_budget,
                    agent_runtime,
                )
                .await
            }
            RerunVariant::Branch => {
                self.rerun_branch(
                    session,
                    provider,
                    msg_id,
                    pending_approvals,
                    allowed_paths,
                    auto_approve,
                    workspace_root,
                    child_registry,
                    byte_budget,
                    agent_runtime,
                )
                .await
            }
            RerunVariant::Fresh => {
                self.rerun_fresh(
                    session,
                    provider,
                    msg_id,
                    pending_approvals,
                    allowed_paths,
                    auto_approve,
                    workspace_root,
                    child_registry,
                    byte_budget,
                    agent_runtime,
                )
                .await
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn rerun_replace<P: Provider>(
        &self,
        session: Arc<crate::session::Session>,
        provider: Arc<P>,
        target: MessageId,
        pending_approvals: PendingApprovals,
        allowed_paths: Vec<String>,
        auto_approve: bool,
        workspace_root: Option<std::path::PathBuf>,
        child_registry: Option<crate::sandbox::ChildRegistry>,
        byte_budget: Option<Arc<crate::byte_budget::ByteBudget>>,
        agent_runtime: Option<AgentRuntime>,
    ) -> Result<MessageId> {
        // Read the log up to the current tip; filter prior supersede markers
        // so a second rerun doesn't rebuild context from already-hidden
        // messages.
        let history = read_since(&session.log_path, 0)
            .await
            .map_err(|e| anyhow!("rerun_replace: read event log: {e}"))?;
        let history = apply_superseded(history);

        let req = build_request_up_to(&history, &target)?;

        // Register the same tool dispatcher `run_turn` uses — rerun must be
        // able to re-execute tool calls if the regenerated stream emits
        // them.
        let mut dispatcher = crate::tools::ToolDispatcher::new();
        dispatcher
            .register(Box::new(crate::tools::FsReadTool))
            .expect("fs.read must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::FsWriteTool))
            .expect("fs.write must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::FsEditTool))
            .expect("fs.edit must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::ShellExecTool))
            .expect("shell.exec must register on a fresh dispatcher");
        // F-134: register `agent.spawn` on the rerun dispatcher too. F-140
        // additional mandate — when the caller supplies an `AgentRuntime`,
        // thread it so a regenerated turn that emits `agent.spawn`
        // actually spawns the child against the session's shared
        // orchestrator. `None` preserves the pre-F-140 "not configured"
        // shape for embedders with no runtime wired up.
        dispatcher
            .register(Box::new(crate::tools::AgentSpawnTool))
            .expect("agent.spawn must register on a fresh dispatcher");
        let new_id = MessageId::new();
        let agent_ctx = agent_runtime.as_ref().map(|rt| AgentSpawnCtx {
            agent_defs: Arc::clone(&rt.agent_defs),
            orchestrator: Arc::clone(&rt.orchestrator),
            session: Arc::clone(&session),
            parent_instance_id: rt.parent_instance_id.clone(),
            current_msg_id: new_id.clone(),
        });
        let instance_id = agent_runtime
            .as_ref()
            .map(|rt| rt.parent_instance_id.clone());
        let ctx = crate::tools::ToolCtx {
            allowed_paths,
            workspace_root,
            child_registry,
            byte_budget,
            agent_ctx,
            // F-132: rerun paths do not re-enter the MCP path. Rerun
            // regenerates a prior assistant turn against already-recorded
            // context — the running session's MCP manager already
            // populated any MCP tool calls in the original transcript,
            // and replaying those same tool calls through a fresh manager
            // would be wrong (the external state may have moved on).
            mcp: None,
        };

        run_request_loop(
            Arc::clone(&session),
            provider,
            req,
            new_id.clone(),
            // Replace does not create a branch — the regenerated message
            // takes the original's place rather than sitting alongside it.
            None,
            0,
            pending_approvals,
            &dispatcher,
            &ctx,
            auto_approve,
            instance_id,
        )
        .await?;

        // Emit the supersede marker only after regeneration succeeded. If
        // run_request_loop returned Err, we bailed above — the original
        // assistant message remains authoritative in the transcript.
        session
            .emit(Event::MessageSuperseded {
                old_id: target,
                new_id: new_id.clone(),
            })
            .await?;

        Ok(new_id)
    }

    /// F-144: re-run producing a new Branch sibling of `target`.
    ///
    /// Semantics (per `docs/ui-specs/branching.md §15.1` and CONCEPT.md
    /// §10.3 "Branch"):
    ///   1. Read + filter history. Compute the branch root:
    ///      `root = target.branch_parent.unwrap_or(target)`. This coalesces
    ///      "branch of a branch" so every variant of the same original
    ///      response threads to the same root id (otherwise chained branches
    ///      would form a tree rather than a flat list of siblings).
    ///   2. Walk the filtered history to find `prev_max`, the highest
    ///      `branch_variant_index` among any `AssistantMessage` with
    ///      `branch_parent.unwrap_or(id) == root`. The root itself sits at
    ///      variant 0; the first Branch re-run produces variant 1.
    ///   3. Rebuild the provider request up to `target` via the same
    ///      `build_request_up_to` helper Replace uses — a branch is a
    ///      sibling generation from the same prompt state.
    ///   4. Drive `run_request_loop` with `branch_parent = Some(root)` and
    ///      `branch_variant_index = prev_max + 1`. No supersede marker is
    ///      emitted — both the original and the new sibling stay visible
    ///      in replay; consumers choose which to display via
    ///      `BranchSelected` (emitted separately by `select_branch`).
    #[allow(clippy::too_many_arguments)]
    async fn rerun_branch<P: Provider>(
        &self,
        session: Arc<crate::session::Session>,
        provider: Arc<P>,
        target: MessageId,
        pending_approvals: PendingApprovals,
        allowed_paths: Vec<String>,
        auto_approve: bool,
        workspace_root: Option<std::path::PathBuf>,
        child_registry: Option<crate::sandbox::ChildRegistry>,
        byte_budget: Option<Arc<crate::byte_budget::ByteBudget>>,
        agent_runtime: Option<AgentRuntime>,
    ) -> Result<MessageId> {
        let history = read_since(&session.log_path, 0)
            .await
            .map_err(|e| anyhow!("rerun_branch: read event log: {e}"))?;
        let history = apply_superseded(history);

        // Resolve the branch root. `target.branch_parent ?? target.id` —
        // spec §15.1. Also capture `prev_max`, the highest variant index
        // already at this branch point (root sits at 0 implicitly).
        let (root, prev_max) = find_branch_root_and_max(&history, &target)?;
        let next_index = prev_max
            .checked_add(1)
            .ok_or_else(|| anyhow!("rerun_branch: branch_variant_index overflow"))?;

        let req = build_request_up_to(&history, &target)?;

        let mut dispatcher = crate::tools::ToolDispatcher::new();
        dispatcher
            .register(Box::new(crate::tools::FsReadTool))
            .expect("fs.read must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::FsWriteTool))
            .expect("fs.write must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::FsEditTool))
            .expect("fs.edit must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::ShellExecTool))
            .expect("shell.exec must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::AgentSpawnTool))
            .expect("agent.spawn must register on a fresh dispatcher");
        let new_id = MessageId::new();
        let agent_ctx = agent_runtime.as_ref().map(|rt| AgentSpawnCtx {
            agent_defs: Arc::clone(&rt.agent_defs),
            orchestrator: Arc::clone(&rt.orchestrator),
            session: Arc::clone(&session),
            parent_instance_id: rt.parent_instance_id.clone(),
            current_msg_id: new_id.clone(),
        });
        let instance_id = agent_runtime
            .as_ref()
            .map(|rt| rt.parent_instance_id.clone());
        let ctx = crate::tools::ToolCtx {
            allowed_paths,
            workspace_root,
            child_registry,
            byte_budget,
            agent_ctx,
            // F-132: rerun paths do not re-enter the MCP path. Rerun
            // regenerates a prior assistant turn against already-recorded
            // context — the running session's MCP manager already
            // populated any MCP tool calls in the original transcript,
            // and replaying those same tool calls through a fresh manager
            // would be wrong (the external state may have moved on).
            mcp: None,
        };

        run_request_loop(
            Arc::clone(&session),
            provider,
            req,
            new_id.clone(),
            Some(root),
            next_index,
            pending_approvals,
            &dispatcher,
            &ctx,
            auto_approve,
            instance_id,
        )
        .await?;

        // Branch does not emit MessageSuperseded: both versions co-exist.
        Ok(new_id)
    }

    /// F-144: re-run discarding intermediate turns — the "Fresh" variant.
    ///
    /// Semantics (per CONCEPT.md §10.3 "Fresh"): regenerate from the
    /// originating user message alone, losing all intermediate tool calls
    /// and sub-agent context. The new AssistantMessage is a new root
    /// (`branch_parent = None`); the original target assistant turn is
    /// logically superseded via `MessageSuperseded` so replay consumers
    /// hide it.
    ///
    /// Steps:
    ///   1. Read + filter history.
    ///   2. Locate the `UserMessage` that immediately precedes `target`'s
    ///      assistant turn; build a one-message `ChatRequest` from it.
    ///      This is the key behavioural difference from Replace (which
    ///      carries *all* prior turns) — Fresh discards everything between
    ///      the user message and the target.
    ///   3. Drive `run_request_loop` with root branch metadata (None / 0).
    ///   4. Emit `MessageSuperseded { old_id: target, new_id }` on success
    ///      so a fresh subscriber sees only the regenerated message.
    ///
    /// Ordering invariant matches Replace: if the regenerated stream errors
    /// mid-flight, the supersede marker is never emitted and the original
    /// message stays authoritative.
    #[allow(clippy::too_many_arguments)]
    async fn rerun_fresh<P: Provider>(
        &self,
        session: Arc<crate::session::Session>,
        provider: Arc<P>,
        target: MessageId,
        pending_approvals: PendingApprovals,
        allowed_paths: Vec<String>,
        auto_approve: bool,
        workspace_root: Option<std::path::PathBuf>,
        child_registry: Option<crate::sandbox::ChildRegistry>,
        byte_budget: Option<Arc<crate::byte_budget::ByteBudget>>,
        agent_runtime: Option<AgentRuntime>,
    ) -> Result<MessageId> {
        let history = read_since(&session.log_path, 0)
            .await
            .map_err(|e| anyhow!("rerun_fresh: read event log: {e}"))?;
        let history = apply_superseded(history);

        let req = build_fresh_request_for(&history, &target)?;

        let mut dispatcher = crate::tools::ToolDispatcher::new();
        dispatcher
            .register(Box::new(crate::tools::FsReadTool))
            .expect("fs.read must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::FsWriteTool))
            .expect("fs.write must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::FsEditTool))
            .expect("fs.edit must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::ShellExecTool))
            .expect("shell.exec must register on a fresh dispatcher");
        dispatcher
            .register(Box::new(crate::tools::AgentSpawnTool))
            .expect("agent.spawn must register on a fresh dispatcher");
        let new_id = MessageId::new();
        let agent_ctx = agent_runtime.as_ref().map(|rt| AgentSpawnCtx {
            agent_defs: Arc::clone(&rt.agent_defs),
            orchestrator: Arc::clone(&rt.orchestrator),
            session: Arc::clone(&session),
            parent_instance_id: rt.parent_instance_id.clone(),
            current_msg_id: new_id.clone(),
        });
        let instance_id = agent_runtime
            .as_ref()
            .map(|rt| rt.parent_instance_id.clone());
        let ctx = crate::tools::ToolCtx {
            allowed_paths,
            workspace_root,
            child_registry,
            byte_budget,
            agent_ctx,
            // F-132: rerun paths do not re-enter the MCP path. Rerun
            // regenerates a prior assistant turn against already-recorded
            // context — the running session's MCP manager already
            // populated any MCP tool calls in the original transcript,
            // and replaying those same tool calls through a fresh manager
            // would be wrong (the external state may have moved on).
            mcp: None,
        };

        run_request_loop(
            Arc::clone(&session),
            provider,
            req,
            new_id.clone(),
            None,
            0,
            pending_approvals,
            &dispatcher,
            &ctx,
            auto_approve,
            instance_id,
        )
        .await?;

        session
            .emit(Event::MessageSuperseded {
                old_id: target,
                new_id: new_id.clone(),
            })
            .await?;

        Ok(new_id)
    }

    /// F-144: activate a specific branch variant for replay / UI.
    ///
    /// Resolves `variant_index` against the filtered event log and emits
    /// `Event::BranchSelected { parent, selected }`.
    ///
    /// Resolution rules:
    ///   * `variant_index == 0` — `selected` is `parent` itself (the root's
    ///     own id).
    ///   * `variant_index >= 1` — `selected` is the `AssistantMessage` with
    ///     `branch_parent == Some(parent)` and matching
    ///     `branch_variant_index`.
    ///
    /// An unknown variant index returns `Err` and does **not** emit
    /// `BranchSelected`. Emitting for a nonexistent variant would corrupt
    /// the event log — downstream consumers reasonably assume every
    /// `BranchSelected.selected` points at a real message.
    pub async fn select_branch(
        &self,
        session: Arc<crate::session::Session>,
        parent: MessageId,
        variant_index: u32,
    ) -> Result<()> {
        let history = read_since(&session.log_path, 0)
            .await
            .map_err(|e| anyhow!("select_branch: read event log: {e}"))?;
        let history = apply_superseded(history);

        let selected = resolve_branch_variant(&history, &parent, variant_index)?;
        session
            .emit(Event::BranchSelected { parent, selected })
            .await?;
        Ok(())
    }

    /// F-145: tombstone a branch variant. Resolves `(parent, variant_index)`
    /// against the filtered event log (reusing `resolve_branch_variant` so
    /// unknown variants surface the same diagnostic), then emits
    /// `Event::BranchDeleted { parent, variant_index }`.
    ///
    /// Refuses to delete `variant_index == 0` when sibling variants remain:
    /// the root is the original message and removing it would orphan the
    /// siblings whose `branch_parent` points at it. Deleting the root when
    /// it is the *only* variant would leave the turn empty on screen; the
    /// UI gates against that path too (the strip only renders when there
    /// are two or more variants), but the orchestrator enforces the
    /// invariant server-side so a compromised webview cannot bypass it.
    pub async fn delete_branch(
        &self,
        session: Arc<crate::session::Session>,
        parent: MessageId,
        variant_index: u32,
    ) -> Result<()> {
        let history = read_since(&session.log_path, 0)
            .await
            .map_err(|e| anyhow!("delete_branch: read event log: {e}"))?;
        let history = apply_superseded(history);

        // Resolve the target — shares its error message with select_branch so
        // clients see a consistent "variant not found" diagnostic regardless
        // of which action triggered the lookup.
        let _ = resolve_branch_variant(&history, &parent, variant_index)
            .map_err(|e| anyhow!("delete_branch: {e}"))?;

        // Refuse root deletion when siblings exist. Count live siblings
        // (branch_parent == Some(parent)) — if any remain, deleting the root
        // would leave them parent-less on replay.
        if variant_index == 0 {
            let sibling_count = history
                .iter()
                .filter(|(_, ev)| {
                    matches!(
                        ev,
                        Event::AssistantMessage { branch_parent: Some(bp), .. } if bp == &parent
                    )
                })
                .count();
            if sibling_count > 0 {
                return Err(anyhow!(
                    "delete_branch: refusing to delete root variant while {sibling_count} sibling(s) remain"
                ));
            }
        }

        session
            .emit(Event::BranchDeleted {
                parent,
                variant_index,
            })
            .await?;
        Ok(())
    }
}

/// Walk `history` (a superseded-filtered `(seq, Event)` replay) and
/// rebuild the [`ChatRequest`] that was in front of the provider when
/// `target` was produced.
///
/// Rules:
/// - Events up to and including the `UserMessage` immediately preceding
///   `target`'s assistant turn are translated to `ChatMessage`s.
/// - `target`'s own assistant turn (including its deltas, tool calls,
///   tool results, and the terminal `AssistantMessage { stream_finalised:
///   true }`) is dropped.
/// - Unknown/non-conversation events (SessionStarted, UsageTick, etc.)
///   are skipped.
fn build_request_up_to(history: &[(u64, Event)], target: &MessageId) -> Result<ChatRequest> {
    let mut messages: Vec<ChatMessage> = Vec::new();
    let mut finalised_assistant_text: HashMap<MessageId, String> = HashMap::new();
    let mut current_assistant: Option<MessageId> = None;

    for (_, ev) in history {
        match ev {
            Event::UserMessage { text, .. } => {
                // A UserMessage implicitly closes any open assistant turn.
                if let Some(id) = current_assistant.take() {
                    flush_assistant(&mut messages, &id, &finalised_assistant_text);
                }
                messages.push(ChatMessage {
                    role: ChatRole::User,
                    content: vec![ChatBlock::Text(text.to_string())],
                });
            }
            Event::AssistantMessage {
                id,
                stream_finalised,
                text,
                ..
            } => {
                if id == target {
                    // We've reached the target's turn — stop **before** it.
                    // Anything already flushed is the reconstructed context.
                    return finalise_request(messages);
                }
                current_assistant = Some(id.clone());
                if *stream_finalised {
                    finalised_assistant_text.insert(id.clone(), text.to_string());
                }
            }
            _ => {
                // Skip deltas, tool call events, etc. The finalised
                // AssistantMessage text is authoritative for context
                // reconstruction; per-delta replay is unnecessary here.
            }
        }
    }

    // Target not found in history — this is a client/server state drift.
    Err(anyhow!(
        "rerun_message: target message {target:?} not found in session log"
    ))
}

fn flush_assistant(
    messages: &mut Vec<ChatMessage>,
    id: &MessageId,
    finalised: &HashMap<MessageId, String>,
) {
    if let Some(text) = finalised.get(id) {
        messages.push(ChatMessage {
            role: ChatRole::Assistant,
            content: vec![ChatBlock::Text(text.clone())],
        });
    }
}

fn finalise_request(messages: Vec<ChatMessage>) -> Result<ChatRequest> {
    // The request must contain at least the preceding UserMessage. If the
    // target was the very first event (shouldn't happen — an assistant
    // message always follows a user turn) we return an informative error.
    if messages.is_empty() {
        return Err(anyhow!(
            "rerun_message: no conversation context before target message"
        ));
    }
    Ok(ChatRequest {
        system: None,
        messages,
        parallel_tool_calls_allowed: false,
    })
}

/// F-144: given a filtered history and a rerun target, return the branch
/// root id and the highest `branch_variant_index` already present at that
/// root. Used by `rerun_branch` to thread the new variant as
/// `(root, prev_max + 1)`.
///
/// Root resolution follows spec §15.1:
///   * If `target.branch_parent == Some(root)` — target is already a
///     branch variant; the new sibling shares the same root.
///   * If `target.branch_parent == None` — target is a root message; the
///     new sibling's root is `target.id` itself.
///
/// `prev_max` starts at 0 (the root's implicit variant). Any sibling with
/// `branch_parent == Some(root)` bumps it to at least its own
/// `branch_variant_index`.
///
/// Walks the filtered `(seq, Event)` list twice: once to find `target`'s
/// own AssistantMessage (to read its `branch_parent`), once to scan for
/// siblings. Both passes are O(n) in history size — single-threaded reruns
/// are not a hot path.
fn find_branch_root_and_max(
    history: &[(u64, Event)],
    target: &MessageId,
) -> Result<(MessageId, u32)> {
    // Pass 1: locate target's own `branch_parent`.
    let target_branch_parent = history
        .iter()
        .find_map(|(_, ev)| match ev {
            Event::AssistantMessage {
                id, branch_parent, ..
            } if id == target => Some(branch_parent.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            anyhow!("rerun_branch: target message {target:?} not found in session log")
        })?;

    let root = target_branch_parent.unwrap_or_else(|| target.clone());

    // Pass 2: scan siblings. `prev_max` is the highest `branch_variant_index`
    // seen for any AssistantMessage whose branch_parent coalesces to `root`.
    // `id == root` covers the root itself (which is the implicit variant 0
    // but we tolerate any value its own variant field may carry).
    let mut prev_max: u32 = 0;
    for (_, ev) in history {
        if let Event::AssistantMessage {
            id,
            branch_parent,
            branch_variant_index,
            ..
        } = ev
        {
            let belongs_to_root = match branch_parent {
                Some(p) => p == &root,
                None => id == &root,
            };
            if belongs_to_root && *branch_variant_index > prev_max {
                prev_max = *branch_variant_index;
            }
        }
    }

    Ok((root, prev_max))
}

/// F-144: build the single-message `ChatRequest` for the Fresh re-run
/// variant.
///
/// Behaviourally distinct from `build_request_up_to`: where Replace /
/// Branch carry *all* prior turns into the request, Fresh discards
/// everything between the originating user message and `target`. The
/// returned request contains exactly one message — the last `UserMessage`
/// before `target`'s assistant turn.
///
/// If `target` is not found, or no `UserMessage` precedes it, returns an
/// informative error.
fn build_fresh_request_for(history: &[(u64, Event)], target: &MessageId) -> Result<ChatRequest> {
    let mut last_user: Option<String> = None;

    for (_, ev) in history {
        match ev {
            Event::UserMessage { text, .. } => {
                last_user = Some(text.to_string());
            }
            Event::AssistantMessage { id, .. } if id == target => {
                let text = last_user.ok_or_else(|| {
                    anyhow!("rerun_fresh: no user message precedes target {target:?}")
                })?;
                return Ok(ChatRequest {
                    system: None,
                    messages: vec![ChatMessage {
                        role: ChatRole::User,
                        content: vec![ChatBlock::Text(text)],
                    }],
                    parallel_tool_calls_allowed: false,
                });
            }
            _ => {}
        }
    }

    Err(anyhow!(
        "rerun_fresh: target message {target:?} not found in session log"
    ))
}

/// F-144: resolve `(parent, variant_index)` to the MessageId to report in
/// `BranchSelected.selected`.
///
/// * `variant_index == 0` returns `parent` directly — the root variant is
///   represented by the original message id itself.
/// * `variant_index >= 1` scans `history` for an `AssistantMessage` with
///   `branch_parent == Some(parent)` and `branch_variant_index` equal to
///   the requested index.
///
/// Unknown variants return `Err`. Callers **must** propagate rather than
/// emit `BranchSelected { parent, selected: parent }` as a fallback —
/// replay consumers assume every selected id is live.
fn resolve_branch_variant(
    history: &[(u64, Event)],
    parent: &MessageId,
    variant_index: u32,
) -> Result<MessageId> {
    if variant_index == 0 {
        // Sanity-check that a root with this id actually exists. A random
        // parent id should not be silently accepted — the UI would gate
        // display of a nonexistent message.
        let exists = history
            .iter()
            .any(|(_, ev)| matches!(ev, Event::AssistantMessage { id, .. } if id == parent));
        if !exists {
            return Err(anyhow!(
                "select_branch: parent message {parent:?} not found in session log"
            ));
        }
        return Ok(parent.clone());
    }

    history
        .iter()
        .find_map(|(_, ev)| match ev {
            Event::AssistantMessage {
                id,
                branch_parent: Some(bp),
                branch_variant_index,
                ..
            } if bp == parent && *branch_variant_index == variant_index => Some(id.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            anyhow!("select_branch: no variant with index {variant_index} under parent {parent:?}")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_providers::MockProvider;
    use std::collections::HashMap;
    use tempfile::TempDir;
    use tokio::sync::Mutex;

    // F-135: verify AGENTS.md content is injected into `ChatRequest.system`
    // exactly once, at the correct position, and that the cached value is
    // reused across multiple turns (no re-read of the file on disk).
    #[tokio::test]
    async fn agents_md_injected_into_system_prompt_and_cached_across_turns() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("events.jsonl");
        let session = Arc::new(Session::create(log_path).await.unwrap());

        // Script an end-of-turn response for each of two turns.
        let script = "{\"done\":\"end_turn\"}\n".to_string();
        let provider = Arc::new(
            MockProvider::from_responses(vec![script.clone(), script]).expect("construct mock"),
        );

        // F-566: simulate the labeled-prefix cache that `serve_with_session`
        // would build once via `build_system_prompt`. `run_turn` no longer
        // reformats per turn — it clones the prebuilt `Arc<str>`.
        let original = "be helpful";
        let expected = format!("\n\n---\nAGENTS.md (workspace):\n{original}");
        let agents_md: Option<Arc<str>> = Some(Arc::from(expected.as_str()));

        let pending: PendingApprovals = Arc::new(Mutex::new(HashMap::new()));

        run_turn(
            Arc::clone(&session),
            Arc::clone(&provider),
            "first turn".to_string(),
            Arc::clone(&pending),
            vec![],
            true,
            None,
            None,
            None,
            agents_md.clone(),
            None,
            None,
            None,
            None, // F-587: no credentials wired in this AGENTS.md test.
        )
        .await
        .unwrap();

        // Between turns, overwrite the hypothetical on-disk AGENTS.md. The
        // cache is an `Arc<str>` captured at session start, so the second
        // turn must still observe the original content — proving no re-read.
        // (We don't have a workspace wired in this unit test; the assertion
        // below on request #2 is what proves cache reuse.)

        run_turn(
            Arc::clone(&session),
            Arc::clone(&provider),
            "second turn".to_string(),
            pending,
            vec![],
            true,
            None,
            None,
            None,
            agents_md,
            None,
            None,
            None,
            None, // F-587
        )
        .await
        .unwrap();

        let reqs = provider.recorded_requests();
        assert_eq!(reqs.len(), 2, "exactly two turns dispatched");

        assert_eq!(
            reqs[0].system.as_deref(),
            Some(expected.as_str()),
            "first turn: AGENTS.md injected at the correct position with exact delimiter"
        );
        assert_eq!(
            reqs[1].system.as_deref(),
            Some(expected.as_str()),
            "second turn: cached value reused, no re-read"
        );

        // "Injection appears once" — the labeled header must not be
        // duplicated inside the system string (e.g. by accidental double
        // prepend on continuation requests within a turn).
        assert_eq!(
            reqs[0]
                .system
                .as_deref()
                .unwrap()
                .matches("AGENTS.md (workspace):")
                .count(),
            1,
            "label must appear exactly once in the system prompt"
        );
    }

    // F-135: when no AGENTS.md is cached (file absent or workspace unset),
    // `ChatRequest.system` stays `None` — no session failure, no empty
    // labeled block.
    #[tokio::test]
    async fn system_prompt_is_none_when_agents_md_absent() {
        let dir = TempDir::new().unwrap();
        let log_path = dir.path().join("events.jsonl");
        let session = Arc::new(Session::create(log_path).await.unwrap());

        let provider = Arc::new(
            MockProvider::from_responses(vec!["{\"done\":\"end_turn\"}\n".into()])
                .expect("construct mock"),
        );

        run_turn(
            session,
            Arc::clone(&provider),
            "hello".to_string(),
            Arc::new(Mutex::new(HashMap::new())),
            vec![],
            true,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None, // F-587
        )
        .await
        .unwrap();

        let reqs = provider.recorded_requests();
        assert_eq!(reqs.len(), 1);
        assert!(
            reqs[0].system.is_none(),
            "no cache → no injection, system stays None"
        );
    }

    // F-144: branch-of-a-branch re-runs must thread to the same root. Spec
    // §15.1: `branch_parent_id = M.branch_parent_id ?? M.id`. If the target
    // is already a branch variant, the new sibling shares target's parent
    // (not target itself). This is what keeps variants flat — without
    // coalescing, chained Branch reruns would form a tree of parents and
    // `prev_max + 1` couldn't resolve sibling order.
    #[test]
    fn find_branch_root_coalesces_when_target_is_itself_a_variant() {
        use chrono::Utc;

        let root = MessageId::new();
        let variant1 = MessageId::new();
        let variant2 = MessageId::new();

        fn assistant_with_branch(id: &MessageId, parent: Option<&MessageId>, idx: u32) -> Event {
            Event::AssistantMessage {
                id: id.clone(),
                provider: ProviderId::new(),
                model: "mock".into(),
                at: Utc::now(),
                stream_finalised: true,
                text: Arc::from(""),
                branch_parent: parent.cloned(),
                branch_variant_index: idx,
            }
        }

        let history = vec![
            (1, assistant_with_branch(&root, None, 0)),
            (2, assistant_with_branch(&variant1, Some(&root), 1)),
            (3, assistant_with_branch(&variant2, Some(&root), 2)),
        ];

        // Branch-ing `variant1` must coalesce to `root`, and prev_max must
        // reflect variant2 (index 2) — not just variant1's own index.
        let (resolved_root, prev_max) =
            find_branch_root_and_max(&history, &variant1).expect("resolve");
        assert_eq!(
            resolved_root, root,
            "branch-of-a-branch must resolve to root"
        );
        assert_eq!(
            prev_max, 2,
            "prev_max must scan all siblings, not just target's own variant_index"
        );

        // Branch-ing the root directly lands at the same root and the same
        // prev_max — the family of variants does not grow by targeting the
        // root.
        let (root_again, same_max) =
            find_branch_root_and_max(&history, &root).expect("resolve root");
        assert_eq!(root_again, root);
        assert_eq!(same_max, 2);
    }

    // F-144: resolve_branch_variant must refuse unknown variant indices.
    // A well-meaning bug that emits `BranchSelected { parent, selected:
    // parent }` as a fallback on unknown index would corrupt replay —
    // downstream UIs assume every selected id is a live message.
    #[test]
    fn resolve_branch_variant_rejects_unknown_index() {
        use chrono::Utc;

        let root = MessageId::new();
        let variant1 = MessageId::new();
        let history = vec![
            (
                1,
                Event::AssistantMessage {
                    id: root.clone(),
                    provider: ProviderId::new(),
                    model: "mock".into(),
                    at: Utc::now(),
                    stream_finalised: true,
                    text: Arc::from(""),
                    branch_parent: None,
                    branch_variant_index: 0,
                },
            ),
            (
                2,
                Event::AssistantMessage {
                    id: variant1.clone(),
                    provider: ProviderId::new(),
                    model: "mock".into(),
                    at: Utc::now(),
                    stream_finalised: true,
                    text: Arc::from(""),
                    branch_parent: Some(root.clone()),
                    branch_variant_index: 1,
                },
            ),
        ];

        // Known variants resolve.
        assert_eq!(
            resolve_branch_variant(&history, &root, 0).expect("root resolves"),
            root
        );
        assert_eq!(
            resolve_branch_variant(&history, &root, 1).expect("variant 1 resolves"),
            variant1
        );

        // Unknown indexes return Err without silently falling back.
        assert!(resolve_branch_variant(&history, &root, 99).is_err());
        // Parent id that doesn't exist in the log is also rejected —
        // even at variant_index 0, where the naïve implementation would
        // return the parent unchanged.
        let orphan = MessageId::new();
        assert!(resolve_branch_variant(&history, &orphan, 0).is_err());
    }
}
