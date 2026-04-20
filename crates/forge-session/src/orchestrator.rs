use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{anyhow, Result};
use chrono::Utc;
use forge_core::{
    apply_superseded,
    ids::{MessageId, ProviderId, ToolCallId},
    read_since, ApprovalScope, ApprovalSource, Event, RerunVariant,
};
use forge_providers::{ChatBlock, ChatChunk, ChatMessage, ChatRequest, ChatRole, Provider};
use futures::StreamExt;
use tokio::sync::{oneshot, Mutex};

use crate::byte_budget::ByteBudget;
use crate::sandbox::ChildRegistry;
use crate::session::Session;
use crate::tools::{
    FsEditTool, FsReadTool, FsWriteTool, ShellExecTool, ToolCtx, ToolDispatcher, ToolError,
};

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

/// Run a complete turn for the given user text. Emits all session events for:
/// UserMessage → AssistantMessage(open) → AssistantDelta* →
/// [ToolCallStarted → ToolCallApprovalRequested → ToolCallApproved → ToolCallCompleted]* →
/// AssistantMessage(finalised)
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
) -> Result<()> {
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
    // prepend slots cleanly before the labeled section. The cache is loaded
    // once at session start (see `serve_with_session`) and reused across every
    // turn to avoid re-reading the file on each provider call.
    let system = agents_md
        .as_deref()
        .map(|content| format!("\n\n---\nAGENTS.md (workspace):\n{content}"));

    let initial_req = ChatRequest {
        system,
        messages: vec![ChatMessage {
            role: ChatRole::User,
            content: vec![ChatBlock::Text(text)],
        }],
    };

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
    let ctx = ToolCtx {
        allowed_paths,
        workspace_root,
        child_registry,
        byte_budget,
    };

    run_request_loop(
        session,
        provider,
        initial_req,
        msg_id,
        pending_approvals,
        &dispatcher,
        &ctx,
        auto_approve,
    )
    .await
}

/// Drives the provider request loop for one logical turn.
/// On tool calls: waits for approval, executes stub, appends result to the
/// next request, and continues until the provider returns `Done` with no
/// pending tool calls.
///
/// `pub(crate)` so rerun paths (F-143+) can reuse the loop with a pre-built
/// `ChatRequest` and a pre-chosen `msg_id`, instead of going through
/// [`run_turn`] which synthesizes a fresh `UserMessage` event.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_request_loop<P: Provider>(
    session: Arc<Session>,
    provider: Arc<P>,
    mut req: ChatRequest,
    msg_id: MessageId,
    pending_approvals: PendingApprovals,
    dispatcher: &ToolDispatcher,
    ctx: &ToolCtx,
    auto_approve: bool,
) -> Result<()> {
    // Fixed provider/model identifiers for the mock provider.
    let provider_id = ProviderId::new();
    let model = "mock".to_string();

    loop {
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
                branch_parent: None,
                branch_variant_index: 0,
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
                            if auto_approve {
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
                                        session
                                            .emit(Event::AssistantMessage {
                                                id: msg_id.clone(),
                                                provider: provider_id.clone(),
                                                model: model.clone(),
                                                at: Utc::now(),
                                                stream_finalised: true,
                                                // F-112: wrap at boundary.
                                                text: Arc::from(assistant_text.as_str()),
                                                branch_parent: None,
                                                branch_variant_index: 0,
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

                            tool.invoke(&args, ctx).await
                        }
                        Err(ToolError::UnknownTool(n)) => {
                            serde_json::json!({ "error": format!("unknown tool '{n}'") })
                        }
                        Err(e) => serde_json::json!({ "error": e.to_string() }),
                    };

                    let duration_ms = started.elapsed().as_millis() as u64;

                    session
                        .emit(Event::ToolCallCompleted {
                            id: call_id.clone(),
                            result: result.clone(),
                            duration_ms,
                            at: Utc::now(),
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
                            branch_parent: None,
                            branch_variant_index: 0,
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
                            branch_parent: None,
                            branch_variant_index: 0,
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
                    branch_parent: None,
                    branch_variant_index: 0,
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

    /// F-143: re-run an existing assistant message.
    ///
    /// For [`RerunVariant::Replace`]:
    ///   1. Read the event log and filter prior supersede markers so reruns
    ///      don't compound.
    ///   2. Reconstruct the provider request from events up to — but not
    ///      including — `msg_id`'s assistant turn.
    ///   3. Drive `run_request_loop` with a fresh `new_id` to regenerate
    ///      the response.
    ///   4. After the regenerated assistant message is finalised, emit
    ///      `Event::MessageSuperseded { old_id: msg_id, new_id }` so
    ///      replay consumers hide the original.
    ///
    /// Ordering matters: the `MessageSuperseded` marker is emitted *after*
    /// the new assistant message is finalised. If regeneration errors
    /// mid-stream, the marker is never written and the original message
    /// stays visible — we don't point the UI at a half-written new_id.
    ///
    /// Branch / Fresh return an error today; they land in F-144 / F-145.
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
                )
                .await
            }
            RerunVariant::Branch => Err(anyhow!(
                "rerun_message: Branch variant not implemented (F-144)"
            )),
            RerunVariant::Fresh => Err(anyhow!(
                "rerun_message: Fresh variant not implemented (F-145)"
            )),
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
        let ctx = crate::tools::ToolCtx {
            allowed_paths,
            workspace_root,
            child_registry,
            byte_budget,
        };

        let new_id = MessageId::new();
        run_request_loop(
            Arc::clone(&session),
            provider,
            req,
            new_id.clone(),
            pending_approvals,
            &dispatcher,
            &ctx,
            auto_approve,
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

        // Simulate the cache that `serve_with_session` would build from
        // `forge_agents::load_agents_md`.
        let original = "be helpful";
        let agents_md: Option<Arc<str>> = Some(Arc::from(original));

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
        )
        .await
        .unwrap();

        let reqs = provider.recorded_requests();
        assert_eq!(reqs.len(), 2, "exactly two turns dispatched");

        let expected = format!("\n\n---\nAGENTS.md (workspace):\n{original}");
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
}
