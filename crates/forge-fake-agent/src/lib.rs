//! Scripted ACP agent that replays YAML steps for tests.
//!
//! [`serve`] runs the agent side of an ACP connection: each prompt replays steps from the
//! [`Script`] until `end_turn`, making the same client requests a real agent would (file reads
//! and writes, terminals, permission requests, session updates).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::schema::v1::*;
use agent_client_protocol::{Agent, Client, ConnectTo, ConnectionTo};
use serde_json::json;
use tokio::sync::Notify;

pub mod script;

pub use script::{Expected, PermissionAction, Script, ScriptError, Step, glob_match};

/// Script position and per-connection state.
struct State {
    steps: Vec<Step>,
    next: usize,
    cwd: PathBuf,
    last_permission: Option<Expected>,
    turns: u64,
}

struct Shared {
    state: Mutex<State>,
    cancelled: AtomicBool,
    cancel: Notify,
}

impl Shared {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Sleep for `d`, returning early (with `false`) if the turn is cancelled.
    async fn sleep(&self, d: Duration) -> bool {
        let notified = self.cancel.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.is_cancelled() {
            return false;
        }
        tokio::select! {
            () = tokio::time::sleep(d) => true,
            () = notified => false,
        }
    }
}

/// Serve the agent side of one ACP connection until the client disconnects.
pub async fn serve(
    script: Script,
    transport: impl ConnectTo<Agent> + 'static,
) -> Result<(), agent_client_protocol::Error> {
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            steps: script.steps,
            next: 0,
            cwd: PathBuf::from("/"),
            last_permission: None,
            turns: 0,
        }),
        cancelled: AtomicBool::new(false),
        cancel: Notify::new(),
    });
    let (on_session, on_prompt, on_cancel) = (shared.clone(), shared.clone(), shared);
    Agent
        .builder()
        .name("forge-fake-agent")
        .on_receive_request(
            async move |init: InitializeRequest, responder, _cx| {
                responder.respond(
                    InitializeResponse::new(init.protocol_version)
                        .agent_capabilities(AgentCapabilities::new()),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                on_session.state().cwd = req.cwd;
                responder.respond(NewSessionResponse::new("fake-session"))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: PromptRequest, responder, cx: ConnectionTo<Client>| {
                let shared = on_prompt.clone();
                let conn = cx.clone();
                // Turns make client requests and wait for answers, so they run off the
                // dispatch loop.
                cx.spawn(async move {
                    match run_turn(&conn, &shared, req).await {
                        Ok(stop) => responder.respond(PromptResponse::new(stop)),
                        Err(message) => {
                            eprintln!("forge-fake-agent: {message}");
                            responder.respond_with_error(agent_client_protocol::Error::new(
                                -32603, message,
                            ))
                        }
                    }
                })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_: CancelNotification, _cx| {
                on_cancel.cancelled.store(true, Ordering::SeqCst);
                on_cancel.cancel.notify_waiters();
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(transport)
        .await
}

async fn run_turn(
    cx: &ConnectionTo<Client>,
    shared: &Shared,
    req: PromptRequest,
) -> Result<StopReason, String> {
    shared.cancelled.store(false, Ordering::SeqCst);
    let session = req.session_id;
    let prompt: String = req
        .prompt
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect();
    let (cwd, message_id) = {
        let mut state = shared.state();
        state.turns += 1;
        (state.cwd.clone(), format!("msg-{}", state.turns))
    };
    loop {
        if shared.is_cancelled() {
            skip_to_next_turn(shared);
            return Ok(StopReason::Cancelled);
        }
        let step = {
            let mut state = shared.state();
            let step = state.steps.get(state.next).cloned();
            state.next += 1;
            step
        };
        let Some(step) = step else {
            return Ok(StopReason::EndTurn);
        };
        match step {
            Step::EndTurn => return Ok(StopReason::EndTurn),
            Step::ExpectPrompt(pattern) => {
                if !glob_match(&pattern, &prompt) {
                    return Err(format!("prompt {prompt:?} does not match {pattern:?}"));
                }
            }
            Step::Plan(entries) => {
                let entries = entries
                    .into_iter()
                    .map(|e| PlanEntry::new(e, PlanEntryPriority::Medium, PlanEntryStatus::Pending))
                    .collect();
                update(cx, &session, SessionUpdate::Plan(Plan::new(entries)))?;
            }
            Step::Message(text) => {
                let chunk = ContentChunk::new(ContentBlock::Text(TextContent::new(text)))
                    .message_id(MessageId::new(message_id.as_str()));
                update(cx, &session, SessionUpdate::AgentMessageChunk(chunk))?;
            }
            Step::Read(path) => {
                request(
                    cx,
                    ReadTextFileRequest::new(session.clone(), resolve(&cwd, &path)),
                )
                .await?;
            }
            Step::Write(w) => {
                let req =
                    WriteTextFileRequest::new(session.clone(), resolve(&cwd, &w.path), w.content);
                request(cx, req).await?;
            }
            Step::Exec(argv) => exec(cx, &session, &cwd, argv).await?,
            Step::RequestPermission(p) => {
                let (kind, raw) = match p.action {
                    PermissionAction::Exec => (ToolKind::Execute, json!({ "argv": p.argv })),
                    PermissionAction::Edit => (ToolKind::Edit, json!({ "path": p.path })),
                };
                let fields = ToolCallUpdateFields::new()
                    .kind(kind)
                    .title(format!("{:?}", p.action))
                    .raw_input(raw);
                let options = vec![
                    PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
                    PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
                ];
                let tool_call =
                    ToolCallUpdate::new(format!("perm-{}", shared.state().next), fields);
                let resp = request(
                    cx,
                    RequestPermissionRequest::new(session.clone(), tool_call, options),
                )
                .await?;
                let answer = match resp.outcome {
                    RequestPermissionOutcome::Selected(s) if &*s.option_id.0 == "allow" => {
                        Expected::Allow
                    }
                    _ => Expected::Deny,
                };
                shared.state().last_permission = Some(answer);
            }
            Step::ExpectPermission(want) => {
                let got = shared.state().last_permission;
                if got != Some(want) {
                    return Err(format!("expected permission {want:?}, got {got:?}"));
                }
            }
            Step::SleepMs(ms) => {
                shared.sleep(Duration::from_millis(ms)).await;
            }
        }
    }
}

/// After a cancel, continue the script from the step after the interrupted turn's `end_turn`.
fn skip_to_next_turn(shared: &Shared) {
    let mut state = shared.state();
    while let Some(end) = state.steps.get(state.next).map(|s| *s == Step::EndTurn) {
        state.next += 1;
        if end {
            break;
        }
    }
}

async fn exec(
    cx: &ConnectionTo<Client>,
    session: &SessionId,
    cwd: &Path,
    argv: Vec<String>,
) -> Result<(), String> {
    let (command, args) = argv.split_first().ok_or("exec needs a command")?;
    let created = request(
        cx,
        CreateTerminalRequest::new(session.clone(), command.clone())
            .args(args.to_vec())
            .cwd(cwd.to_path_buf()),
    )
    .await?;
    let id = created.terminal_id;
    request(
        cx,
        WaitForTerminalExitRequest::new(session.clone(), id.clone()),
    )
    .await?;
    request(cx, TerminalOutputRequest::new(session.clone(), id.clone())).await?;
    request(cx, ReleaseTerminalRequest::new(session.clone(), id)).await?;
    Ok(())
}

fn resolve(cwd: &Path, path: &str) -> PathBuf {
    cwd.join(path)
}

fn update(
    cx: &ConnectionTo<Client>,
    session: &SessionId,
    update: SessionUpdate,
) -> Result<(), String> {
    cx.send_notification(SessionNotification::new(session.clone(), update))
        .map_err(|e| format!("sending update: {e}"))
}

async fn request<R>(cx: &ConnectionTo<Client>, req: R) -> Result<R::Response, String>
where
    R: agent_client_protocol::JsonRpcRequest,
{
    let method = req.method().to_owned();
    cx.send_request(req)
        .block_task()
        .await
        .map_err(|e| format!("{method}: {e}"))
}
