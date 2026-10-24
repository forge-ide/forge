//! The ACP connection to one agent process.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    self as acp, ContentBlock, CreateTerminalRequest, CreateTerminalResponse, InitializeRequest,
    KillTerminalRequest, KillTerminalResponse, NewSessionRequest, PromptRequest,
    ReadTextFileRequest, ReadTextFileResponse, ReleaseTerminalRequest, ReleaseTerminalResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, TerminalExitStatus,
    TerminalOutputRequest, TerminalOutputResponse, TextContent, WaitForTerminalExitRequest,
    WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use forge_api::TurnId;
use forge_api::events::{
    AgentMessageBody, AgentPlanBody, EditBody, EventPayload, PlanEntry, TerminalOutputBody,
    ToolCallBody, ToolResultBody,
};
use forge_api::model::{Action, EditKind};
use forge_sandbox::SessionSandbox;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{oneshot, watch};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::terminal::{DEFAULT_OUTPUT_LIMIT, ExitInfo, Terminals};
use crate::workspace::{PathError, Workspace};

/// Lines of the agent's stderr kept for crash reports.
const STDERR_TAIL: usize = 20;
/// Longest tool output kept in a `tool.result` event.
const RESULT_OUTPUT_LIMIT: usize = 4096;
/// The error code for a write or terminal the gate refused.
const REFUSED: i32 = -32003;

/// Where session events go. The session layer appends them to the event log.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, payload: EventPayload) -> BoxFuture<'_, ()>;
}

/// What the agent is asking to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateKind {
    /// A permission request: the agent asks before using a tool of its own.
    Permission,
    /// A file write through the client.
    Write,
    /// A command in a client terminal.
    Terminal,
}

/// An action the agent wants to take, for the [`Gate`] to decide. `action` is `None` when a
/// permission request describes nothing Forge can map to an action.
#[derive(Clone, Debug, PartialEq)]
pub struct GateRequest {
    pub kind: GateKind,
    pub tool_call_id: String,
    pub title: String,
    pub action: Option<Action>,
    /// The content of a write, for the secret scan.
    pub content: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateVerdict {
    Allow,
    /// Allow a terminal, running this argv instead of the one requested (a hook rewrote it).
    /// The extra environment, if kept, is part of the argv as an `env` prefix.
    Rewrite(Vec<String>),
    Deny(String),
    /// The turn was cancelled while the request waited.
    Cancelled,
}

/// Decides every permission request, write, and terminal before it happens, recording the
/// decision. A decision may wait on a human.
pub trait Gate: Send + Sync + 'static {
    fn decide(&self, request: GateRequest) -> BoxFuture<'_, GateVerdict>;

    /// After an allowed write or terminal finished, successfully or not: text for the end of
    /// a terminal's output, such as notes from post hooks.
    fn finished<'a>(
        &'a self,
        _kind: GateKind,
        _action: &'a Action,
        _success: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async { None })
    }
}

/// Allows everything; for tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAll;

impl Gate for AllowAll {
    fn decide(&self, _: GateRequest) -> BoxFuture<'_, GateVerdict> {
        Box::pin(async { GateVerdict::Allow })
    }
}

/// Denies everything with a fixed reason; for tests.
#[derive(Clone, Debug)]
pub struct DenyAll {
    pub reason: String,
}

impl Gate for DenyAll {
    fn decide(&self, _: GateRequest) -> BoxFuture<'_, GateVerdict> {
        let verdict = GateVerdict::Deny(self.reason.clone());
        Box::pin(async move { verdict })
    }
}

/// How to start an agent.
#[derive(Clone, Debug)]
pub struct AgentCommand {
    /// Program and arguments.
    pub argv: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
}

/// How an agent process ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentExit {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    /// The last lines it wrote to stderr.
    pub stderr: Vec<String>,
}

impl std::fmt::Display for AgentExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.code, self.signal) {
            (Some(code), _) => write!(f, "agent exited with code {code}")?,
            (None, Some(signal)) => write!(f, "agent killed by signal {signal}")?,
            (None, None) => write!(f, "agent exited")?,
        }
        if let Some(last) = self.stderr.last() {
            write!(f, ": {last}")?;
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("agent command is empty")]
    NoCommand,
    #[error("starting {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Exited(AgentExit),
    #[error("agent: {0}")]
    Protocol(String),
}

/// A connected agent with one ACP session.
pub struct AcpClient {
    conn: ConnectionTo<Agent>,
    session: acp::SessionId,
    turn: Arc<Mutex<Option<TurnId>>>,
    terminals: Arc<Terminals>,
    exit: watch::Receiver<Option<AgentExit>>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
}

impl AcpClient {
    /// Start the agent in the workspace inside `sandbox`, initialize ACP, and open a session.
    /// The agent's terminals run in the same sandbox.
    pub async fn start(
        command: AgentCommand,
        workspace: Workspace,
        sandbox: Arc<dyn SessionSandbox>,
        sink: Arc<dyn EventSink>,
        gate: Arc<dyn Gate>,
    ) -> Result<Self, AcpError> {
        let (program, args) = command.argv.split_first().ok_or(AcpError::NoCommand)?;
        let mut child = sandbox
            .command(program, args, workspace.cwd(), &command.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0)
            .spawn()
            .map_err(|source| AcpError::Spawn {
                program: program.clone(),
                source,
            })?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr_tail = Arc::new(Mutex::new(VecDeque::new()));
        let stderr_done = child
            .stderr
            .take()
            .map(|stderr| tokio::spawn(read_stderr(stderr, stderr_tail.clone())));

        let terminals = Arc::new(Terminals::new(sandbox));
        let turn = Arc::new(Mutex::new(None));
        let cwd = workspace.cwd().to_path_buf();
        let handlers = Arc::new(Handlers {
            workspace,
            sink,
            gate,
            terminals: terminals.clone(),
            turn: turn.clone(),
        });

        let (exit_tx, exit_rx) = watch::channel(None);
        let (kill_tx, kill_rx) = oneshot::channel();
        let (conn_tx, conn_rx) = oneshot::channel();
        let (stop_tx, stop_rx) = oneshot::channel::<()>();
        let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());
        tokio::spawn(async move {
            let result = connect(handlers, transport, async move |cx: ConnectionTo<Agent>| {
                let _ = conn_tx.send(cx);
                let _ = stop_rx.await;
                Ok(())
            })
            .await;
            if let Err(e) = result {
                tracing::debug!(error = %e, "agent connection ended");
            }
        });
        let pid = child.id();
        tokio::spawn(supervise(
            child,
            pid,
            kill_rx,
            stop_tx,
            stderr_done,
            stderr_tail,
            exit_tx,
        ));

        let mut client = Self {
            conn: conn_rx
                .await
                .map_err(|_| AcpError::Protocol("connection closed during setup".into()))?,
            session: acp::SessionId::new(""),
            turn,
            terminals,
            exit: exit_rx,
            kill: Mutex::new(Some(kill_tx)),
        };
        let capabilities = acp::ClientCapabilities::new()
            .fs(acp::FileSystemCapabilities::new()
                .read_text_file(true)
                .write_text_file(true))
            .terminal(true);
        let init = InitializeRequest::new(ProtocolVersion::V1).client_capabilities(capabilities);
        let setup = async {
            client.request(init).await?;
            client.request(NewSessionRequest::new(cwd)).await
        };
        match setup.await {
            Ok(session) => {
                client.session = session.session_id;
                Ok(client)
            }
            Err(e) => {
                client.shutdown().await;
                Err(e)
            }
        }
    }

    /// Run one prompt turn; returns the stop reason (`end_turn`, `cancelled`, ...).
    pub async fn prompt(&self, turn: TurnId, text: String) -> Result<String, AcpError> {
        *lock(&self.turn) = Some(turn);
        let request = PromptRequest::new(
            self.session.clone(),
            vec![ContentBlock::Text(TextContent::new(text))],
        );
        let result = self.request(request).await;
        *lock(&self.turn) = None;
        let stop = result?.stop_reason;
        Ok(match serde_json::to_value(stop) {
            Ok(serde_json::Value::String(s)) => s,
            _ => format!("{stop:?}"),
        })
    }

    /// Ask the agent to stop the current turn; the turn ends with stop reason `cancelled`.
    pub fn cancel(&self) -> Result<(), AcpError> {
        self.conn
            .send_notification(acp::CancelNotification::new(self.session.clone()))
            .map_err(|e| AcpError::Protocol(e.to_string()))
    }

    /// Resolves when the agent process exits.
    pub fn exit_watch(&self) -> watch::Receiver<Option<AgentExit>> {
        self.exit.clone()
    }

    /// Kill the agent and every terminal it started, and wait for it to exit.
    pub async fn shutdown(&self) {
        self.terminals.kill_all();
        if let Some(kill) = lock(&self.kill).take() {
            let _ = kill.send(());
        }
        let mut exit = self.exit.clone();
        let _ = exit.wait_for(Option::is_some).await;
    }

    async fn request<R>(&self, request: R) -> Result<R::Response, AcpError>
    where
        R: agent_client_protocol::JsonRpcRequest,
    {
        let result = self.conn.send_request(request).block_task().await;
        match result {
            Ok(response) => Ok(response),
            Err(e) => {
                // A request that failed because the agent died reports the exit instead.
                let mut exit = self.exit.clone();
                let exited = tokio::time::timeout(
                    std::time::Duration::from_millis(500),
                    exit.wait_for(Option::is_some),
                )
                .await;
                match exited {
                    Ok(Ok(info)) => Err(AcpError::Exited(info.clone().expect("checked"))),
                    _ => Err(AcpError::Protocol(e.to_string())),
                }
            }
        }
    }
}

/// Wait for the agent to exit (or kill it on request), then report how it ended.
async fn supervise(
    mut child: Child,
    pid: Option<u32>,
    kill: oneshot::Receiver<()>,
    stop_connection: oneshot::Sender<()>,
    stderr_done: Option<tokio::task::JoinHandle<()>>,
    stderr_tail: Arc<Mutex<VecDeque<String>>>,
    exit: watch::Sender<Option<AgentExit>>,
) {
    let status = tokio::select! {
        status = child.wait() => status,
        _ = kill => {
            kill_group(pid);
            child.wait().await
        }
    };
    if let Some(done) = stderr_done {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), done).await;
    }
    let _ = stop_connection.send(());
    use std::os::unix::process::ExitStatusExt;
    let info = AgentExit {
        code: status.as_ref().ok().and_then(|s| s.code()),
        signal: status.as_ref().ok().and_then(|s| s.signal()),
        stderr: lock(&stderr_tail).iter().cloned().collect(),
    };
    let _ = exit.send(Some(info));
}

async fn read_stderr(stderr: tokio::process::ChildStderr, tail: Arc<Mutex<VecDeque<String>>>) {
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let mut tail = lock(&tail);
        if tail.len() == STDERR_TAIL {
            tail.pop_front();
        }
        tail.push_back(line);
    }
}

fn kill_group(pid: Option<u32>) {
    let Some(pid) = pid
        .and_then(|p| i32::try_from(p).ok())
        .and_then(rustix::process::Pid::from_raw)
    else {
        return;
    };
    if let Err(e) = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL) {
        tracing::debug!(error = %e, "killing agent process group");
    }
}

/// Serves the agent's requests against the workspace.
struct Handlers {
    workspace: Workspace,
    sink: Arc<dyn EventSink>,
    gate: Arc<dyn Gate>,
    terminals: Arc<Terminals>,
    turn: Arc<Mutex<Option<TurnId>>>,
}

type AcpResult<T> = Result<T, agent_client_protocol::Error>;

fn reply<T: agent_client_protocol::JsonRpcResponse>(
    responder: agent_client_protocol::Responder<T>,
    result: AcpResult<T>,
) -> AcpResult<()> {
    match result {
        Ok(response) => responder.respond(response),
        Err(e) => responder.respond_with_error(e),
    }
}

/// Run the client side of the connection, serving the agent's requests with `h`.
async fn connect(
    h: Arc<Handlers>,
    transport: impl agent_client_protocol::ConnectTo<Client> + 'static,
    main: impl AsyncFnOnce(ConnectionTo<Agent>) -> AcpResult<()>,
) -> AcpResult<()> {
    let (read, write, create, output, wait, kill, release, permission, update) = (
        h.clone(),
        h.clone(),
        h.clone(),
        h.clone(),
        h.clone(),
        h.clone(),
        h.clone(),
        h.clone(),
        h,
    );
    Client
        .builder()
        .name("forge")
        .on_receive_request(
            async move |req: ReadTextFileRequest, responder, _cx| {
                reply(responder, read.read(req).await)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: WriteTextFileRequest, responder, cx: ConnectionTo<Agent>| {
                // The gate may wait on a human; don't hold the dispatch loop.
                let h = write.clone();
                cx.spawn(async move { reply(responder, h.write(req).await) })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: CreateTerminalRequest, responder, cx: ConnectionTo<Agent>| {
                let h = create.clone();
                cx.spawn(async move { reply(responder, h.create_terminal(req).await) })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: TerminalOutputRequest, responder, _cx| {
                reply(responder, output.terminal_output(&req))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: WaitForTerminalExitRequest, responder, cx: ConnectionTo<Agent>| {
                // Waiting can take as long as the command runs; don't hold the dispatch loop.
                let h = wait.clone();
                cx.spawn(async move { reply(responder, h.wait_terminal(&req).await) })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: KillTerminalRequest, responder, _cx| {
                let result = kill
                    .terminals
                    .kill(&req.terminal_id.0)
                    .map(|()| KillTerminalResponse::new())
                    .map_err(io_error);
                reply(responder, result)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: ReleaseTerminalRequest, responder, _cx| {
                let result = release
                    .terminals
                    .release(&req.terminal_id.0)
                    .map(|()| ReleaseTerminalResponse::new())
                    .map_err(io_error);
                reply(responder, result)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |req: RequestPermissionRequest, responder, cx: ConnectionTo<Agent>| {
                // A decision may wait on a human; don't hold the dispatch loop.
                let h = permission.clone();
                cx.spawn(async move { reply(responder, h.permission(req).await) })
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |n: SessionNotification, _cx| {
                update.update(n).await;
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_with(transport, main)
        .await
}

fn tool_call_id() -> String {
    format!("tc_{}", ulid::Ulid::new())
}

/// The last `RESULT_OUTPUT_LIMIT` bytes of `text`.
fn tail(text: &str) -> String {
    let mut start = text.len().saturating_sub(RESULT_OUTPUT_LIMIT);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_owned()
}

impl Handlers {
    async fn read(&self, req: ReadTextFileRequest) -> AcpResult<ReadTextFileResponse> {
        let resolved = self.workspace.resolve_read(&req.path).map_err(path_error)?;
        let text = tokio::fs::read_to_string(&resolved.path)
            .await
            .map_err(io_error)?;
        let skip = req.line.map_or(0, |l| l.saturating_sub(1) as usize);
        let content = match (skip, req.limit) {
            (0, None) => text,
            (skip, limit) => {
                let lines = text.split_inclusive('\n').skip(skip);
                match limit {
                    Some(n) => lines.take(n as usize).collect(),
                    None => lines.collect(),
                }
            }
        };
        Ok(ReadTextFileResponse::new(content))
    }

    async fn write(&self, req: WriteTextFileRequest) -> AcpResult<WriteTextFileResponse> {
        let resolved = self
            .workspace
            .resolve_write(&req.path)
            .map_err(path_error)?;
        let path = resolved.path.clone();
        let content = req.content;
        let id = tool_call_id();
        let old = tokio::fs::read_to_string(&path).await.ok();
        let kind = if old.is_some() || path.exists() {
            EditKind::Modify
        } else {
            EditKind::Create
        };
        let action = Action::Edit {
            path: resolved.display.clone(),
            kind,
        };
        let title = format!("Write {}", resolved.display);
        self.call(&id, &action, &title).await;
        let request = GateRequest {
            kind: GateKind::Write,
            tool_call_id: id.clone(),
            title,
            action: Some(action),
            content: Some(content.clone()),
        };
        if self.allowed(request).await?.is_some() {
            return Err(self.refuse(&id, "a write can't be rewritten".into()).await);
        }
        // The decision may have waited on a human while the agent's processes kept running,
        // so resolve again: a parent directory swapped for a symlink in the meantime must not
        // carry the write somewhere else.
        let again = self.workspace.resolve_write(&req.path);
        if again.as_ref().map(|r| &r.path) != Ok(&path) {
            let reason = "the path changed while the write waited for a decision".to_owned();
            return Err(self.refuse(&id, reason).await);
        }
        let old = tokio::fs::read_to_string(&path).await.ok().or(old);
        let new = content.clone();
        let written = tokio::task::spawn_blocking(move || write_file(&path, &content))
            .await
            .map_err(io_error)
            .and_then(|r| r.map_err(io_error));
        let action = Action::Edit {
            path: resolved.display.clone(),
            kind,
        };
        self.gate
            .finished(GateKind::Write, &action, written.is_ok())
            .await;
        match written {
            Ok(()) => {
                let (additions, deletions) = line_changes(old.as_deref().unwrap_or(""), &new);
                self.sink
                    .emit(EventPayload::Edit(EditBody {
                        path: resolved.display,
                        kind,
                        additions,
                        deletions,
                    }))
                    .await;
                self.result(&id, "completed", None).await;
                Ok(WriteTextFileResponse::new())
            }
            Err(e) => {
                self.result(&id, "failed", Some(e.message.clone())).await;
                Err(e)
            }
        }
    }

    async fn create_terminal(
        self: &Arc<Self>,
        req: CreateTerminalRequest,
    ) -> AcpResult<CreateTerminalResponse> {
        let cwd = match &req.cwd {
            Some(dir) => self.workspace.resolve_dir(dir),
            None => self.workspace.resolve_dir(self.workspace.cwd()),
        }
        .map_err(path_error)?;
        let id = tool_call_id();
        let env: Vec<(String, String)> = req
            .env
            .iter()
            .map(|v| (v.name.clone(), v.value.clone()))
            .collect();
        if let Some((bad, _)) = env
            .iter()
            .find(|(k, v)| !forge_sandbox::is_valid_name(k) || v.contains('\0'))
        {
            return Err(agent_client_protocol::Error::new(
                -32602,
                format!("invalid environment variable {bad:?}"),
            ));
        }
        let argv = terminal_argv(&req.command, &req.args, &env);
        let action = Action::Exec {
            argv: argv.clone(),
            cwd: cwd.display.clone(),
        };
        let title = format!("Run {}", argv.join(" "));
        self.call(&id, &action, &title).await;
        let request = GateRequest {
            kind: GateKind::Terminal,
            tool_call_id: id.clone(),
            title,
            action: Some(action),
            content: None,
        };
        let (program, args, env) = match self.allowed(request).await? {
            Some(argv) => {
                let (program, args) = argv
                    .split_first()
                    .map(|(p, a)| (p.clone(), a.to_vec()))
                    .unwrap_or_default();
                (program, args, Vec::new())
            }
            None => (req.command.clone(), req.args.clone(), env),
        };
        let action = Action::Exec {
            argv: terminal_argv(&program, &args, &env),
            cwd: cwd.display.clone(),
        };
        let limit = req
            .output_byte_limit
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(DEFAULT_OUTPUT_LIMIT);
        let (out, done) = (self.clone(), self.clone());
        let terminal_id = id.clone();
        let result_id = id.clone();
        let spawned = self.terminals.spawn(
            id.clone(),
            &program,
            &args,
            &cwd.path,
            &env,
            limit,
            Arc::new(move |chunk| {
                let (h, terminal_id) = (out.clone(), terminal_id.clone());
                Box::pin(async move {
                    h.sink
                        .emit(EventPayload::TerminalOutput(TerminalOutputBody {
                            terminal_id,
                            chunk,
                        }))
                        .await;
                })
            }),
            Box::new(move |info: ExitInfo, output: String| {
                Box::pin(async move {
                    let status = if info.success() {
                        "completed"
                    } else {
                        "failed"
                    };
                    let extra = done
                        .gate
                        .finished(GateKind::Terminal, &action, info.success())
                        .await
                        .map(|note| format!("\n[forge] {note}\n"));
                    let output = output + extra.as_deref().unwrap_or("");
                    done.result(&result_id, status, Some(tail(&output))).await;
                    extra
                })
            }),
        );
        match spawned {
            Ok(id) => Ok(CreateTerminalResponse::new(id)),
            Err(e) => {
                self.result(&id, "failed", Some(e.to_string())).await;
                Err(io_error(e))
            }
        }
    }

    fn terminal_output(&self, req: &TerminalOutputRequest) -> AcpResult<TerminalOutputResponse> {
        let (output, truncated, exit) = self
            .terminals
            .output(&req.terminal_id.0)
            .map_err(io_error)?;
        Ok(TerminalOutputResponse::new(output, truncated).exit_status(exit.map(exit_status)))
    }

    async fn wait_terminal(
        &self,
        req: &WaitForTerminalExitRequest,
    ) -> AcpResult<WaitForTerminalExitResponse> {
        let info = self
            .terminals
            .wait(&req.terminal_id.0)
            .await
            .map_err(io_error)?;
        Ok(WaitForTerminalExitResponse::new(exit_status(info)))
    }

    async fn permission(
        &self,
        req: RequestPermissionRequest,
    ) -> AcpResult<RequestPermissionResponse> {
        let fields = &req.tool_call.fields;
        let request = GateRequest {
            kind: GateKind::Permission,
            tool_call_id: req.tool_call.tool_call_id.0.to_string(),
            title: fields.title.clone().unwrap_or_default(),
            action: permission_action(&req, &self.workspace),
            content: None,
        };
        if let Some(action) = &request.action {
            self.call(&request.tool_call_id, action, &request.title)
                .await;
        }
        use acp::PermissionOptionKind as Kind;
        let wanted: &[Kind] = match self.gate.decide(request).await {
            GateVerdict::Allow => &[Kind::AllowOnce, Kind::AllowAlways],
            // The agent's own tool can't run a different command.
            GateVerdict::Deny(_) | GateVerdict::Rewrite(_) => {
                &[Kind::RejectOnce, Kind::RejectAlways]
            }
            GateVerdict::Cancelled => &[],
        };
        let choice = wanted
            .iter()
            .find_map(|k| req.options.iter().find(|o| o.kind == *k));
        let outcome = match choice {
            Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            )),
            None => RequestPermissionOutcome::Cancelled,
        };
        Ok(RequestPermissionResponse::new(outcome))
    }

    /// Record that the agent wants to take `action`.
    async fn call(&self, id: &str, action: &Action, title: &str) {
        self.sink
            .emit(EventPayload::ToolCall(ToolCallBody {
                tool_call_id: id.to_owned(),
                action: action.clone(),
                title: title.to_owned(),
            }))
            .await;
    }

    /// Ask the gate about a write or terminal: the argv to run instead, if a hook rewrote it.
    /// On a refusal, record the failed result and return the error for the agent.
    async fn allowed(&self, request: GateRequest) -> AcpResult<Option<Vec<String>>> {
        let id = request.tool_call_id.clone();
        let reason = match self.gate.decide(request).await {
            GateVerdict::Allow => return Ok(None),
            GateVerdict::Rewrite(argv) if !argv.is_empty() => return Ok(Some(argv)),
            GateVerdict::Rewrite(_) => "refused: a hook rewrote it to nothing".to_owned(),
            GateVerdict::Deny(reason) => format!("refused by policy: {reason}"),
            GateVerdict::Cancelled => "cancelled".to_owned(),
        };
        Err(self.refuse(&id, reason).await)
    }

    /// Record a refused write or terminal as failed, and the error for the agent.
    async fn refuse(&self, id: &str, reason: String) -> agent_client_protocol::Error {
        self.result(id, "failed", Some(reason.clone())).await;
        agent_client_protocol::Error::new(REFUSED, reason)
    }

    async fn update(&self, notification: SessionNotification) {
        let payload = match notification.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                let ContentBlock::Text(text) = chunk.content else {
                    return;
                };
                let Some(turn_id) = lock(&self.turn).clone() else {
                    return;
                };
                let message_id = chunk
                    .message_id
                    .map(|id| id.0.to_string())
                    .unwrap_or_else(|| format!("msg-{turn_id}"));
                EventPayload::AgentMessage(AgentMessageBody {
                    turn_id,
                    message_id,
                    delta: text.text,
                })
            }
            SessionUpdate::Plan(plan) => EventPayload::AgentPlan(AgentPlanBody {
                entries: plan
                    .entries
                    .into_iter()
                    .map(|e| PlanEntry {
                        content: e.content,
                        status: match serde_json::to_value(e.status) {
                            Ok(serde_json::Value::String(s)) => s,
                            _ => "pending".into(),
                        },
                    })
                    .collect(),
            }),
            _ => return,
        };
        self.sink.emit(payload).await;
    }

    async fn result(&self, id: &str, status: &str, output: Option<String>) {
        self.sink
            .emit(EventPayload::ToolResult(ToolResultBody {
                tool_call_id: id.to_owned(),
                status: status.to_owned(),
                output,
            }))
            .await;
    }
}

/// The argv a terminal request runs, as policy sees it. Extra environment variables come first
/// as `env NAME=value ...`, since they can change what runs (`PATH`, `BASH_ENV`, `LD_PRELOAD`)
/// and an `exec` action has no other place for them. The sandbox keeps its own variables
/// whatever the agent sets.
fn terminal_argv(command: &str, args: &[String], env: &[(String, String)]) -> Vec<String> {
    let mut argv = Vec::with_capacity(env.len() + args.len() + 2);
    if !env.is_empty() {
        argv.push("env".to_owned());
        argv.extend(env.iter().map(|(k, v)| format!("{k}={v}")));
        // So a program named like an assignment or an option still reads as the program.
        argv.push("--".to_owned());
    }
    argv.push(command.to_owned());
    argv.extend(args.iter().cloned());
    argv
}

/// The [`Action`] a permission request describes, when it maps to one.
fn permission_action(req: &RequestPermissionRequest, workspace: &Workspace) -> Option<Action> {
    let fields = &req.tool_call.fields;
    let raw = fields.raw_input.as_ref();
    match fields.kind? {
        acp::ToolKind::Execute => {
            let argv = match raw.and_then(|r| r.get("argv").or_else(|| r.get("command"))) {
                Some(serde_json::Value::Array(items)) => items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect(),
                // A command string is a shell line, which is what the agent's shell runs;
                // splitting it here would keep its quotes inside the words.
                Some(serde_json::Value::String(cmd)) => {
                    vec!["sh".to_owned(), "-c".to_owned(), cmd.clone()]
                }
                _ => return None,
            };
            Some(Action::Exec {
                argv,
                cwd: ".".into(),
            })
        }
        acp::ToolKind::Edit | acp::ToolKind::Delete | acp::ToolKind::Move => {
            let path = fields
                .locations
                .as_ref()
                .and_then(|l| l.first())
                .map(|l| l.path.clone())
                .or_else(|| raw.and_then(|r| r.get("path")?.as_str().map(PathBuf::from)))?;
            // Resolved as a write would be, so the path decided is the file that would really
            // change: a symlink the agent planted can't carry an allowed edit elsewhere.
            let path = workspace
                .resolve_write(&workspace.cwd().join(path))
                .ok()?
                .display;
            let kind = if fields.kind == Some(acp::ToolKind::Delete) {
                EditKind::Delete
            } else {
                EditKind::Modify
            };
            Some(Action::Edit { path, kind })
        }
        _ => None,
    }
}

fn exit_status(info: ExitInfo) -> TerminalExitStatus {
    TerminalExitStatus::new()
        .exit_code(info.code.and_then(|c| u32::try_from(c).ok()))
        .signal(info.signal)
}

/// Lines added and removed going from `old` to `new`.
fn line_changes(old: &str, new: &str) -> (u32, u32) {
    let diff = similar::TextDiff::from_lines(old, new);
    let (mut added, mut removed) = (0u32, 0u32);
    for change in diff.iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Insert => added += 1,
            similar::ChangeTag::Delete => removed += 1,
            similar::ChangeTag::Equal => {}
        }
    }
    (added, removed)
}

/// Write `content` to `path`, creating parent directories, without following a symlink at the
/// final component.
fn write_file(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
        .open(path)?;
    file.write_all(content.as_bytes())
}

fn path_error(e: PathError) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(-32602, e.to_string())
}

fn io_error(e: impl std::fmt::Display) -> agent_client_protocol::Error {
    agent_client_protocol::Error::new(-32603, e.to_string())
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn permission(raw: serde_json::Value) -> RequestPermissionRequest {
        let fields = acp::ToolCallUpdateFields::new()
            .kind(acp::ToolKind::Execute)
            .raw_input(raw);
        RequestPermissionRequest::new(
            acp::SessionId::new("s"),
            acp::ToolCallUpdate::new("t", fields),
            vec![],
        )
    }

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let ws = Workspace::new(vec![crate::Root {
            name: "repo".into(),
            path: root,
        }])
        .unwrap();
        (dir, ws)
    }

    fn argv(action: Option<Action>) -> Vec<String> {
        match action {
            Some(Action::Exec { argv, .. }) => argv,
            other => panic!("not an exec: {other:?}"),
        }
    }

    #[test]
    fn command_strings_become_shell_lines() {
        let (_dir, ws) = workspace();
        let line = r#"git -C "a b" commit -m 'x y'"#;
        let req = permission(serde_json::json!({ "command": line }));
        assert_eq!(argv(permission_action(&req, &ws)), ["sh", "-c", line]);
    }

    #[test]
    fn argv_arrays_are_kept_as_given() {
        let (_dir, ws) = workspace();
        let req = permission(serde_json::json!({ "argv": ["git", "-C", "a b", "status"] }));
        assert_eq!(
            argv(permission_action(&req, &ws)),
            ["git", "-C", "a b", "status"]
        );
        let req = permission(serde_json::json!({ "command": ["ls", "-la"] }));
        assert_eq!(argv(permission_action(&req, &ws)), ["ls", "-la"]);
    }

    #[test]
    fn terminal_env_shows_as_an_env_prefix() {
        let args = vec!["test".to_owned()];
        assert_eq!(terminal_argv("cargo", &args, &[]), ["cargo", "test"]);
        let env = vec![
            ("PATH".to_owned(), "./bin".to_owned()),
            ("RUST_LOG".to_owned(), "debug".to_owned()),
        ];
        assert_eq!(
            terminal_argv("cargo", &args, &env),
            ["env", "PATH=./bin", "RUST_LOG=debug", "--", "cargo", "test"]
        );
        assert_eq!(
            terminal_argv("FOO=bar", &[], &env[..1]),
            ["env", "PATH=./bin", "--", "FOO=bar"]
        );
    }
}
