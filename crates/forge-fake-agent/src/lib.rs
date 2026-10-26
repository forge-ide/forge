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

pub use script::{
    ExecStep, Expected, McpCallStep, PermissionAction, Script, ScriptError, Step, WriteStep,
    glob_match,
};

/// Script position and per-connection state.
struct State {
    steps: Vec<Step>,
    next: usize,
    cwd: PathBuf,
    last_permission: Option<Expected>,
    turns: u64,
    /// The URL of the MCP server the client named `forge`.
    mcp: Option<String>,
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
            mcp: None,
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
                    InitializeResponse::new(init.protocol_version).agent_capabilities(
                        AgentCapabilities::new()
                            .mcp_capabilities(McpCapabilities::new().http(true)),
                    ),
                )
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: NewSessionRequest, responder, _cx| {
                let mut state = on_session.state();
                state.cwd = req.cwd;
                state.mcp = req.mcp_servers.iter().find_map(|s| match s {
                    McpServer::Http(http) if http.name == "forge" => Some(http.url.clone()),
                    _ => None,
                });
                drop(state);
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
                // `{forge_mcp_url}` stands for the MCP URL the client gave, as an agent that
                // logs its config would print it.
                let url = shared.state().mcp.clone().unwrap_or_default();
                let text = text.replace("{forge_mcp_url}", &url);
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
                expect(request(cx, req).await.map(drop), w.expect, "write", &w.path)?;
            }
            Step::Exec(e) => {
                let what = e.argv.join(" ");
                expect(
                    exec(cx, &session, &cwd, e.argv).await,
                    e.expect,
                    "exec",
                    &what,
                )?;
            }
            Step::RequestPermission(p) => {
                let id = format!("perm-{}", shared.state().next);
                let mut request_meta = None;
                match p.action {
                    PermissionAction::ParsedRead => {
                        let mut call = ToolCall::new(id.clone(), "Read file").kind(ToolKind::Read);
                        if let Some(path) = &p.path {
                            call = call.locations(vec![ToolCallLocation::new(resolve(&cwd, path))]);
                        }
                        update(cx, &session, SessionUpdate::ToolCall(call))?;
                    }
                    PermissionAction::McpTool => {
                        let tool = p.tool.clone().ok_or("mcp_tool needs a tool")?;
                        let mut meta = serde_json::Map::new();
                        meta.insert("is_mcp_tool_call".into(), true.into());
                        let call = ToolCall::new(id.clone(), format!("mcp.forge.{tool}"))
                            .kind(ToolKind::Execute)
                            .raw_input(json!({ "server": "forge", "tool": tool, "arguments": {} }))
                            .meta(meta);
                        update(cx, &session, SessionUpdate::ToolCall(call))?;
                        let mut meta = serde_json::Map::new();
                        meta.insert("is_mcp_tool_approval".into(), true.into());
                        request_meta = Some(meta);
                    }
                    _ => {}
                }
                let (kind, raw) = match p.action {
                    PermissionAction::ParsedRead => (
                        ToolKind::Execute,
                        json!({ "command": p.command.clone().ok_or("parsed_read needs a command")? }),
                    ),
                    PermissionAction::McpTool => (ToolKind::Execute, serde_json::Value::Null),
                    PermissionAction::Exec => (ToolKind::Execute, json!({ "argv": p.argv })),
                    PermissionAction::Edit if p.path.is_some() => {
                        (ToolKind::Edit, json!({ "path": p.path }))
                    }
                    PermissionAction::Edit => (ToolKind::Edit, json!({})),
                    PermissionAction::Read => (ToolKind::Read, json!({})),
                    PermissionAction::Fetch => {
                        (ToolKind::Fetch, json!({ "url": "https://example.com/" }))
                    }
                };
                let mut fields = ToolCallUpdateFields::new().kind(kind);
                if p.action != PermissionAction::McpTool {
                    fields = fields.title(format!("{:?}", p.action)).raw_input(raw);
                }
                if p.action == PermissionAction::Read
                    && let Some(path) = &p.path
                {
                    fields = fields.locations(vec![ToolCallLocation::new(resolve(&cwd, path))]);
                }
                if !p.paths.is_empty() {
                    fields = fields.locations(
                        p.paths
                            .iter()
                            .map(|path| ToolCallLocation::new(resolve(&cwd, path)))
                            .collect::<Vec<_>>(),
                    );
                }
                let options = vec![
                    PermissionOption::new("allow", "Allow", PermissionOptionKind::AllowOnce),
                    PermissionOption::new("reject", "Reject", PermissionOptionKind::RejectOnce),
                ];
                let tool_call = ToolCallUpdate::new(id, fields);
                let resp = request(
                    cx,
                    RequestPermissionRequest::new(session.clone(), tool_call, options)
                        .meta(request_meta),
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
            Step::McpCall(call) => {
                let url = shared
                    .state()
                    .mcp
                    .clone()
                    .ok_or("the client offered no forge MCP server")?;
                let refused = mcp_call(&url, &call.tool, call.arguments).await?;
                match (refused, call.expect) {
                    (Some(why), Some(Expected::Deny)) => {
                        eprintln!("forge-fake-agent: refused as expected: {why}")
                    }
                    (Some(why), _) => return Err(format!("{} refused: {why}", call.tool)),
                    (None, Some(Expected::Deny)) => {
                        return Err(format!("{}: expected a refusal", call.tool));
                    }
                    (None, _) => {}
                }
            }
            Step::SleepMs(ms) => {
                shared.sleep(Duration::from_millis(ms)).await;
            }
        }
    }
}

/// Initialize the MCP server at `url` and call `tool`: `Some(reason)` if the tool refused.
async fn mcp_call(
    url: &str,
    tool: &str,
    arguments: serde_json::Value,
) -> Result<Option<String>, String> {
    mcp_post(
        url,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {},
                           "clientInfo": {"name": "forge-fake-agent", "version": "0"}}}),
    )
    .await?;
    let reply = mcp_post(
        url,
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                "params": {"name": tool, "arguments": arguments}}),
    )
    .await?;
    let result = reply
        .get("result")
        .ok_or_else(|| format!("{tool}: {reply}"))?;
    if result["isError"] == true {
        Ok(Some(
            result["content"][0]["text"]
                .as_str()
                .unwrap_or("")
                .to_owned(),
        ))
    } else {
        Ok(None)
    }
}

/// POST one JSON-RPC message to a plain-HTTP URL and read the JSON answer.
async fn mcp_post(url: &str, message: &serde_json::Value) -> Result<serde_json::Value, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url
        .strip_prefix("http://")
        .ok_or("the MCP URL isn't http://")?;
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    let body = message.to_string();
    let mut stream = tokio::net::TcpStream::connect(authority)
        .await
        .map_err(|e| format!("connecting to {authority}: {e}"))?;
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .map_err(|e| e.to_string())?;
    let response = String::from_utf8_lossy(&response);
    let (head, body) = response
        .split_once("\r\n\r\n")
        .ok_or("malformed MCP response")?;
    if !head.starts_with("HTTP/1.1 200") {
        return Err(format!(
            "MCP server answered {}",
            head.lines().next().unwrap_or("")
        ));
    }
    serde_json::from_str(body).map_err(|e| format!("MCP response: {e}"))
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

/// Check a request's result against what the script expects: by default it must succeed;
/// with `expect: deny` it must be refused, and the turn goes on.
fn expect(
    result: Result<(), String>,
    want: Option<Expected>,
    step: &str,
    what: &str,
) -> Result<(), String> {
    match (result, want) {
        (Ok(()), Some(Expected::Deny)) => Err(format!("{step} {what}: expected a refusal")),
        (Err(_), Some(Expected::Deny)) | (Ok(()), _) => Ok(()),
        (Err(e), _) => Err(e),
    }
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
