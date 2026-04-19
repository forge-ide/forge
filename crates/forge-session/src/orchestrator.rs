use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use chrono::Utc;
use forge_core::{
    ids::{MessageId, ProviderId, ToolCallId},
    ApprovalScope, ApprovalSource, Event,
};
use forge_providers::{ChatBlock, ChatChunk, ChatMessage, ChatRequest, ChatRole, Provider};
use futures::StreamExt;
use tokio::sync::{oneshot, Mutex};

use crate::sandbox::ChildRegistry;
use crate::session::Session;
use crate::tools::{
    FsEditTool, FsReadTool, FsWriteTool, ShellExecTool, ToolCtx, ToolDispatcher, ToolError,
};

/// Pending tool call approvals: maps ToolCallId → sender for the approval result.
pub type PendingApprovals = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

/// Run a complete turn for the given user text. Emits all session events for:
/// UserMessage → AssistantMessage(open) → AssistantDelta* →
/// [ToolCallStarted → ToolCallApprovalRequested → ToolCallApproved → ToolCallCompleted]* →
/// AssistantMessage(finalised)
///
/// Tool calls block until the client sends `ToolCallApproved` / `ToolCallRejected`
/// through `pending_approvals`. `allowed_paths` is the set of glob patterns the
/// agent is permitted to access via `fs.read`.
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
) -> Result<()> {
    let msg_id = MessageId::new();

    session
        .emit(Event::UserMessage {
            id: msg_id.clone(),
            at: Utc::now(),
            text: text.clone(),
            context: vec![],
            branch_parent: None,
        })
        .await?;

    let initial_req = ChatRequest {
        system: None,
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
#[allow(clippy::too_many_arguments)]
async fn run_request_loop<P: Provider>(
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
                text: String::new(),
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
                            delta,
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
                                let (tx, rx) = oneshot::channel::<bool>();
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

                                let approved = rx.await.unwrap_or(false);

                                if !approved {
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
                                            text: assistant_text.clone(),
                                            branch_parent: None,
                                            branch_variant_index: 0,
                                        })
                                        .await?;
                                    return Ok(());
                                }

                                session
                                    .emit(Event::ToolCallApproved {
                                        id: call_id.clone(),
                                        by: ApprovalSource::User,
                                        scope: ApprovalScope::Once,
                                        at: Utc::now(),
                                    })
                                    .await?;
                            }

                            tool.invoke(&args, ctx)
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
                            text: assistant_text.clone(),
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
                            text: assistant_text.clone(),
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
                    text: assistant_text.clone(),
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
