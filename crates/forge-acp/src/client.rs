//! The ACP connection to one agent process.

use std::collections::VecDeque;
use std::path::Path;
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
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
use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo, UntypedMessage};
use forge_api::TurnId;
use forge_api::events::{
    AgentMessageBody, AgentPlanBody, EditBody, EventPayload, PlanEntry, TerminalOutputBody,
    ToolCallBody, ToolCallSource, ToolResultBody,
};
use forge_api::model::{Action, EditKind};
use forge_sandbox::SessionSandbox;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::{oneshot, watch};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::adapters::CommandSource;
use crate::permission::{self, Target, ToolCommands};
use crate::terminal::{DEFAULT_OUTPUT_LIMIT, ExitInfo, Pipe, Terminals};
use crate::workspace::{PathError, Workspace};

/// Lines of the agent's stderr kept for crash reports.
const STDERR_TAIL: usize = 20;
/// Tool calls remembered while they run without asking Forge.
const REMEMBERED_CALLS: usize = 256;
/// Longest tool output kept in a `tool.result` event.
const RESULT_OUTPUT_LIMIT: usize = 4096;
/// The error code for a write or terminal the gate refused.
const REFUSED: i32 = -32003;

/// Where session events go. The session layer appends them to the event log.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, payload: EventPayload) -> BoxFuture<'_, ()>;

    /// A chunk of a terminal's output, from one of its pipes.
    fn terminal_output(&self, _pipe: Pipe, body: TerminalOutputBody) -> BoxFuture<'_, ()> {
        self.emit(EventPayload::TerminalOutput(body))
    }
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
    /// A call to one of Forge's own tools (`git_commit`), through its MCP endpoint.
    Tool,
}

/// An action the agent wants to take, for the [`Gate`] to decide. `action` is `None` when a
/// permission request describes nothing Forge can map to an action; `unmapped` then says what
/// it asked for, if Forge can tell.
#[derive(Clone, Debug, PartialEq)]
pub struct GateRequest {
    pub kind: GateKind,
    pub tool_call_id: String,
    pub title: String,
    pub action: Option<Action>,
    /// The content of a write, for the secret scan.
    pub content: Option<String>,
    pub unmapped: Unmapped,
    /// What the human deciding should also see, such as the files a commit changes.
    pub detail: Option<String>,
}

/// What a permission request without an action asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Unmapped {
    /// Nothing Forge can check.
    #[default]
    Nothing,
    /// Reading or searching files: inside the session's worktrees, or not.
    Read { inside_worktree: bool },
    /// Fetching a URL with the agent's own tool.
    Fetch,
}

/// How to open the agent's ACP session.
#[derive(Clone, Debug, Default)]
pub struct SessionSetup {
    /// Where the agent's execute permission requests carry their command.
    pub command_source: CommandSource,
    /// MCP servers offered to the agent, over HTTP.
    pub mcp_servers: Vec<McpEndpoint>,
    /// `_meta` for `session/new`.
    pub meta: Option<serde_json::Map<String, serde_json::Value>>,
    /// The ACP mode the agent must be in before the first prompt. The session fails to start
    /// if the agent doesn't offer it.
    pub mode: Option<String>,
    /// Where to copy the agent's stderr as it comes, for a sign-in: the agent may print the
    /// URL to finish in a browser there.
    pub stderr_log: Option<std::path::PathBuf>,
    /// Whether the agent's `_meta` names MCP tools in its permission requests (see
    /// [`crate::adapters::Permissions::mcp_meta`]).
    pub mcp_meta: bool,
}

/// An MCP server the agent reaches over Streamable HTTP.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpEndpoint {
    pub name: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateVerdict {
    Allow,
    /// Allow a terminal, running this argv instead of the one requested (a hook rewrote it).
    /// The extra environment, if kept, is part of the argv as an `env` prefix.
    Rewrite(Vec<String>),
    Deny(String),
    /// A human refused it, with their note for the agent if they wrote one.
    HumanDeny(Option<String>),
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
    /// The agent needs signing in first; these are the ways it offers.
    #[error("the agent isn't signed in")]
    AuthRequired { methods: Vec<SignInMethod> },
}

/// A way an agent offers to sign in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignInMethod {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// For a terminal method: how to run the agent, interactively, to sign in.
    pub terminal: Option<TerminalSignIn>,
}

/// A terminal sign-in: the agent's own launch with these args appended and this env added.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSignIn {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

fn sign_in_methods(methods: &[acp::AuthMethod]) -> Vec<SignInMethod> {
    methods
        .iter()
        .map(|m| SignInMethod {
            id: m.id().0.to_string(),
            name: m.name().to_owned(),
            description: m.description().map(str::to_owned),
            terminal: match m {
                acp::AuthMethod::Terminal(t) => Some(TerminalSignIn {
                    args: t.args.clone(),
                    env: t.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                }),
                _ => None,
            },
        })
        .collect()
}

/// Events from a sign-in connection, which has none worth keeping.
struct NoEvents;

impl EventSink for NoEvents {
    fn emit(&self, _: EventPayload) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// A connected agent with one ACP session.
pub struct AcpClient {
    cwd: std::path::PathBuf,
    conn: ConnectionTo<Agent>,
    session: acp::SessionId,
    turn: Arc<Mutex<Option<TurnId>>>,
    terminals: Arc<Terminals>,
    exit: watch::Receiver<Option<AgentExit>>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
    /// Whether the agent takes steering messages.
    steering: Arc<AtomicBool>,
}

impl AcpClient {
    /// Start the agent in the workspace inside `sandbox`, initialize ACP, and open a session.
    /// The agent's terminals run in the same sandbox.
    pub async fn start(
        command: AgentCommand,
        setup: SessionSetup,
        workspace: Workspace,
        sandbox: Arc<dyn SessionSandbox>,
        sink: Arc<dyn EventSink>,
        gate: Arc<dyn Gate>,
    ) -> Result<Self, AcpError> {
        let mcp_servers = setup.mcp_servers.clone();
        let meta = setup.meta.clone();
        let mode = setup.mode.clone();
        let mut client = Self::spawn(command, setup, workspace, sandbox, sink, gate).await?;
        let cwd = client.cwd.clone();
        let opened = async {
            let agent = client.initialize().await?;
            let steering = agent
                .meta
                .as_ref()
                .and_then(|m| m.get("steering")?.get("supported")?.as_bool())
                .unwrap_or(false);
            client
                .steering
                .store(steering, std::sync::atomic::Ordering::SeqCst);
            // Forge's tools are reached over HTTP; an agent that can't do that goes without.
            let http = agent.agent_capabilities.mcp_capabilities.http;
            if !http && !mcp_servers.is_empty() {
                tracing::warn!(
                    "the agent can't reach MCP servers over HTTP; it gets none of Forge's tools"
                );
            }
            let servers = mcp_servers
                .iter()
                .filter(|_| http)
                .map(|s| acp::McpServer::Http(acp::McpServerHttp::new(&s.name, &s.url)))
                .collect();
            let session = client
                .request(NewSessionRequest::new(cwd).mcp_servers(servers).meta(meta))
                .await
                .map_err(|e| match e {
                    AcpError::AuthRequired { .. } => AcpError::AuthRequired {
                        methods: sign_in_methods(&agent.auth_methods),
                    },
                    other => other,
                })?;
            if let Some(mode) = &mode {
                client.enter_mode(&session, mode).await?;
            }
            Ok(session)
        };
        match opened.await {
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

    /// The sign-in methods the agent advertises, from a connection with no session.
    pub async fn sign_in_methods(
        command: AgentCommand,
        cwd: &Path,
        sandbox: Arc<dyn SessionSandbox>,
    ) -> Result<Vec<SignInMethod>, AcpError> {
        let client = Self::sign_in_connection(command, cwd, sandbox, None).await?;
        let agent = client.initialize().await;
        client.shutdown().await;
        Ok(sign_in_methods(&agent?.auth_methods))
    }

    /// Sign in with the agent's own (non-terminal) method `method`: the agent does the whole
    /// flow (a browser, a stored key) and keeps what it gets, as it would outside Forge. Its
    /// stderr is copied to `stderr_log` as it comes, for the URL it may print.
    pub async fn authenticate(
        command: AgentCommand,
        cwd: &Path,
        sandbox: Arc<dyn SessionSandbox>,
        method: &str,
        stderr_log: Option<std::path::PathBuf>,
    ) -> Result<(), AcpError> {
        let client = Self::sign_in_connection(command, cwd, sandbox, stderr_log).await?;
        let result = async {
            let agent = client.initialize().await?;
            let offered = sign_in_methods(&agent.auth_methods);
            match offered.iter().find(|m| m.id == method) {
                None => Err(AcpError::Protocol(format!(
                    "the agent offers no sign-in method {method:?}"
                ))),
                // A client must never send `authenticate` for a terminal method.
                Some(m) if m.terminal.is_some() => Err(AcpError::Protocol(format!(
                    "{method:?} signs in in a terminal, by running the agent with its args"
                ))),
                Some(_) => {
                    client
                        .request(acp::AuthenticateRequest::new(method.to_owned()))
                        .await?;
                    Ok(())
                }
            }
        }
        .await;
        client.shutdown().await;
        result
    }

    /// A connection for signing in: nothing to read, write, or run, and every request refused.
    async fn sign_in_connection(
        command: AgentCommand,
        cwd: &Path,
        sandbox: Arc<dyn SessionSandbox>,
        stderr_log: Option<std::path::PathBuf>,
    ) -> Result<Self, AcpError> {
        let workspace = Workspace::new(vec![crate::Root {
            name: "sign-in".into(),
            path: cwd.to_path_buf(),
        }])
        .map_err(|e| AcpError::Protocol(e.to_string()))?;
        Self::spawn(
            command,
            SessionSetup {
                stderr_log,
                ..SessionSetup::default()
            },
            workspace,
            sandbox,
            Arc::new(NoEvents),
            Arc::new(DenyAll {
                reason: "signing in runs nothing".into(),
            }),
        )
        .await
    }

    /// `initialize`, offering terminal sign-ins: Forge's CLI runs them.
    async fn initialize(&self) -> Result<acp::InitializeResponse, AcpError> {
        let capabilities = acp::ClientCapabilities::new()
            .fs(acp::FileSystemCapabilities::new()
                .read_text_file(true)
                .write_text_file(true))
            .terminal(true)
            .auth(acp::AuthCapabilities::new().terminal(true));
        self.request(InitializeRequest::new(ProtocolVersion::V1).client_capabilities(capabilities))
            .await
    }

    /// Start the agent process and connect to it.
    async fn spawn(
        command: AgentCommand,
        setup: SessionSetup,
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
        let stderr_done = child.stderr.take().map(|stderr| {
            tokio::spawn(read_stderr(
                stderr,
                stderr_tail.clone(),
                setup.stderr_log.clone(),
            ))
        });

        let terminals = Arc::new(Terminals::new(sandbox));
        let turn = Arc::new(Mutex::new(None));
        let steering = Arc::new(AtomicBool::new(false));
        let cwd = workspace.cwd().to_path_buf();
        let handlers = Arc::new(Handlers {
            workspace,
            sink,
            gate,
            terminals: terminals.clone(),
            turn: turn.clone(),
            command_source: setup.command_source,
            mcp_meta: setup.mcp_meta,
            own_servers: setup.mcp_servers.iter().map(|s| s.name.clone()).collect(),
            commands: Mutex::new(ToolCommands::default()),
            unasked: Mutex::new(VecDeque::new()),
            asked: Mutex::new(VecDeque::new()),
            steering: steering.clone(),
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

        let client = Self {
            cwd,
            conn: conn_rx
                .await
                .map_err(|_| AcpError::Protocol("connection closed during setup".into()))?,
            session: acp::SessionId::new(""),
            turn,
            terminals,
            exit: exit_rx,
            kill: Mutex::new(Some(kill_tx)),
            steering,
        };
        Ok(client)
    }

    /// Put a new session in ACP mode `mode`, which the agent must offer. An agent may pick its
    /// starting mode from settings a repo controls, so the mode is set whatever it reports.
    async fn enter_mode(
        &self,
        session: &acp::NewSessionResponse,
        mode: &str,
    ) -> Result<(), AcpError> {
        let offered = session
            .modes
            .as_ref()
            .is_some_and(|m| m.available_modes.iter().any(|a| *a.id.0 == *mode));
        if !offered {
            return Err(AcpError::Protocol(format!(
                "the agent doesn't offer the {mode:?} mode Forge runs it in"
            )));
        }
        self.request(acp::SetSessionModeRequest::new(
            session.session_id.clone(),
            mode.to_owned(),
        ))
        .await?;
        Ok(())
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
                    _ if e.code == agent_client_protocol::ErrorCode::AuthRequired => {
                        Err(AcpError::AuthRequired { methods: vec![] })
                    }
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

async fn read_stderr(
    stderr: tokio::process::ChildStderr,
    tail: Arc<Mutex<VecDeque<String>>>,
    log: Option<std::path::PathBuf>,
) {
    use tokio::io::AsyncWriteExt;
    let mut log = match log {
        Some(path) => tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .await
            .ok(),
        None => None,
    };
    let mut lines = BufReader::new(stderr).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if let Some(file) = &mut log {
            let _ = file.write_all(format!("{line}\n").as_bytes()).await;
        }
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
    command_source: CommandSource,
    mcp_meta: bool,
    /// The MCP servers Forge offers the agent, whose tools ask Forge themselves.
    own_servers: Vec<String>,
    /// Commands the agent reported in its tool calls.
    commands: Mutex<ToolCommands>,
    /// Tool calls the agent reported that haven't asked Forge, as reported so far: one that
    /// finishes without asking is recorded then.
    unasked: Mutex<VecDeque<acp::ToolCall>>,
    /// Tool calls that asked Forge, which are recorded by their requests.
    asked: Mutex<VecDeque<acp::ToolCallId>>,
    /// Whether the agent takes steering messages, as its `initialize` said.
    steering: Arc<AtomicBool>,
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
                let conn = cx.clone();
                cx.spawn(async move { reply(responder, h.permission(req, &conn).await) })
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

/// The extension request that adds a message to the agent's running turn (Claude's adapter
/// offers it, saying so in `initialize`'s `_meta.steering.supported`).
const STEER_METHOD: &str = "_session/steering";
/// The longest reason or note a steering message carries.
const STEER_LIMIT: usize = 1024;

/// What to tell an agent whose request for tool call `id` got `verdict`, if anything: Forge's
/// reason, or a human's note. It goes to the agent as a user message, so nothing in it is the
/// agent's own text: a policy reason's quoted parts (the paths and commands it names) are
/// left out, and the call is named only by an ID of plain characters.
fn steering_text(id: &str, verdict: &GateVerdict) -> Option<String> {
    let plain = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    let call = if plain {
        format!("tool call {id}")
    } else {
        "a tool call".to_owned()
    };
    match verdict {
        GateVerdict::Deny(reason) => Some(format!(
            "[forge] Forge refused {call}: {}",
            one_line(&unquoted(reason))
        )),
        GateVerdict::HumanDeny(Some(note)) => Some(format!(
            "[forge] The user refused {call} and says: {}",
            one_line(note)
        )),
        _ => None,
    }
}

/// `text` with each double-quoted part (escapes included) replaced by `"…"`.
fn unquoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '"' {
            out.push(c);
            continue;
        }
        out.push_str("\"…\"");
        while let Some(c) = chars.next() {
            match c {
                '\\' => {
                    chars.next();
                }
                '"' => break,
                _ => {}
            }
        }
    }
    out
}

/// `text` on one line, without control characters, at most [`STEER_LIMIT`] bytes.
fn one_line(text: &str) -> String {
    let mut out = text
        .split(|c: char| c.is_control())
        .filter(|part| !part.trim().is_empty())
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" ");
    if out.len() > STEER_LIMIT {
        let mut end = STEER_LIMIT;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

/// Whether a tool call with `status` has finished.
fn finished(status: Option<acp::ToolCallStatus>) -> bool {
    matches!(
        status,
        Some(acp::ToolCallStatus::Completed | acp::ToolCallStatus::Failed)
    )
}

fn tool_call_id() -> String {
    format!("tc_{}", ulid::Ulid::new())
}

/// At most the last `RESULT_OUTPUT_LIMIT` bytes of `text`. A cut never starts inside a word:
/// the end of a secret cut short would no longer look like one, and redaction would miss it.
fn tail(text: &str) -> String {
    let mut start = text.len().saturating_sub(RESULT_OUTPUT_LIMIT);
    if start == 0 {
        return text.to_owned();
    }
    while !text.is_char_boundary(start) {
        start += 1;
    }
    let after_space = text[start..]
        .find(char::is_whitespace)
        .map_or(text.len(), |i| start + i);
    text[after_space..].trim_start().to_owned()
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
            unmapped: Unmapped::Nothing,
            detail: None,
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
            unmapped: Unmapped::Nothing,
            detail: None,
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
            Arc::new(move |pipe, chunk| {
                let (h, terminal_id) = (out.clone(), terminal_id.clone());
                Box::pin(async move {
                    h.sink
                        .terminal_output(pipe, TerminalOutputBody { terminal_id, chunk })
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
        conn: &ConnectionTo<Agent>,
    ) -> AcpResult<RequestPermissionResponse> {
        let fields = &req.tool_call.fields;
        let id = &req.tool_call.tool_call_id;
        lock(&self.unasked).retain(|c| c.tool_call_id != *id);
        {
            let mut asked = lock(&self.asked);
            if !asked.contains(id) {
                if asked.len() == REMEMBERED_CALLS {
                    asked.pop_front();
                }
                asked.push_back(id.clone());
            }
        }
        let target = {
            let commands = lock(&self.commands);
            permission::target(
                &req,
                &self.workspace,
                self.command_source,
                self.mcp_meta,
                &commands,
            )
        };
        let asks: Vec<(Option<Action>, Unmapped)> = match target {
            // A tool of Forge's own server asks Forge itself when it runs, as it does for
            // agents that call it without asking.
            Target::Action(Action::Mcp { server, .. }) if self.own_servers.contains(&server) => {
                vec![]
            }
            Target::Action(action) => vec![(Some(action), Unmapped::Nothing)],
            Target::Edits(edits) => edits
                .into_iter()
                .map(|e| (Some(e), Unmapped::Nothing))
                .collect(),
            Target::Read { inside_worktree } => vec![(None, Unmapped::Read { inside_worktree })],
            Target::Fetch => vec![(None, Unmapped::Fetch)],
            Target::Unknown => vec![(None, Unmapped::Nothing)],
        };
        // Each path a patch changes is decided on its own, and the first that isn't allowed
        // decides the request.
        let mut verdict = GateVerdict::Allow;
        for (action, unmapped) in asks {
            let request = GateRequest {
                kind: GateKind::Permission,
                tool_call_id: req.tool_call.tool_call_id.0.to_string(),
                title: fields.title.clone().unwrap_or_default(),
                action,
                content: None,
                unmapped,
                detail: None,
            };
            if let Some(action) = &request.action {
                self.call(&request.tool_call_id, action, &request.title)
                    .await;
            }
            verdict = self.gate.decide(request).await;
            if verdict != GateVerdict::Allow {
                break;
            }
        }
        use acp::PermissionOptionKind as Kind;
        let wanted: &[Kind] = match verdict {
            GateVerdict::Allow => &[Kind::AllowOnce, Kind::AllowAlways],
            // The agent's own tool can't run a different command.
            GateVerdict::Deny(_) | GateVerdict::HumanDeny(_) | GateVerdict::Rewrite(_) => {
                &[Kind::RejectOnce, Kind::RejectAlways]
            }
            GateVerdict::Cancelled => &[],
        };
        // A reject tells the agent nothing more, so an agent that takes steering hears why in
        // its running turn, before the reject lands.
        if self.steering.load(std::sync::atomic::Ordering::SeqCst)
            && let Some(text) = steering_text(&req.tool_call.tool_call_id.0, &verdict)
        {
            let message = UntypedMessage::new(
                STEER_METHOD,
                serde_json::json!({
                    "sessionId": req.session_id,
                    "prompt": [{ "type": "text", "text": text }],
                }),
            );
            let sent = match message {
                Ok(message) => conn.send_request(message).block_task().await.map(drop),
                Err(e) => Err(e),
            };
            match sent {
                Ok(()) => {
                    let id = req.tool_call.tool_call_id.0.to_string();
                    self.result(&id, "failed", Some(text)).await;
                }
                Err(e) => tracing::debug!(error = %e, "steering the agent"),
            }
        }
        let outcome = match choose(&req.options, wanted) {
            Some(option) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(
                option.option_id.clone(),
            )),
            None => RequestPermissionOutcome::Cancelled,
        };
        Ok(RequestPermissionResponse::new(outcome))
    }

    /// Record that the agent wants to take `action`, which Forge decides.
    async fn call(&self, id: &str, action: &Action, title: &str) {
        self.call_from(id, action, title, ToolCallSource::Forge)
            .await;
    }

    async fn call_from(&self, id: &str, action: &Action, title: &str, source: ToolCallSource) {
        self.sink
            .emit(EventPayload::ToolCall(ToolCallBody {
                tool_call_id: id.to_owned(),
                action: action.clone(),
                title: title.to_owned(),
                source,
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
            GateVerdict::HumanDeny(note) => match note {
                Some(note) => format!("refused by the user: {note}"),
                None => "refused by the user".to_owned(),
            },
            GateVerdict::Cancelled => "cancelled".to_owned(),
        };
        Err(self.refuse(&id, reason).await)
    }

    /// Record a refused write or terminal as failed, and the error for the agent.
    async fn refuse(&self, id: &str, reason: String) -> agent_client_protocol::Error {
        self.result(id, "failed", Some(reason.clone())).await;
        agent_client_protocol::Error::new(REFUSED, reason)
    }

    /// Remember what a tool call reports, for its permission request.
    fn record_call(&self, call: &acp::ToolCall) {
        let mut commands = lock(&self.commands);
        commands.record(&call.tool_call_id.0, call.raw_input.as_ref());
        commands.record_changes(&call.tool_call_id.0, &call.content);
        commands.record_mcp(
            &call.tool_call_id.0,
            call.meta.as_ref(),
            call.raw_input.as_ref(),
        );
        let names_command = call
            .raw_input
            .as_ref()
            .is_some_and(|r| r.get("command").is_some());
        if matches!(call.kind, acp::ToolKind::Read | acp::ToolKind::Search) && !names_command {
            commands.record_read(&call.tool_call_id.0);
        }
    }

    async fn update(&self, notification: SessionNotification) {
        let payload = match notification.update {
            SessionUpdate::ToolCall(call) => {
                self.record_call(&call);
                if lock(&self.asked).contains(&call.tool_call_id) {
                    return;
                }
                if finished(Some(call.status)) {
                    self.ran_unasked(call).await;
                } else {
                    let mut unasked = lock(&self.unasked);
                    if unasked.len() == REMEMBERED_CALLS {
                        unasked.pop_front();
                    }
                    unasked.push_back(call);
                }
                return;
            }
            SessionUpdate::ToolCallUpdate(update) => {
                {
                    let mut commands = lock(&self.commands);
                    commands.record(&update.tool_call_id.0, update.fields.raw_input.as_ref());
                    if let Some(content) = &update.fields.content {
                        commands.record_changes(&update.tool_call_id.0, content);
                    }
                }
                let done = {
                    let mut unasked = lock(&self.unasked);
                    let Some(i) = unasked
                        .iter()
                        .position(|c| c.tool_call_id == update.tool_call_id)
                    else {
                        return;
                    };
                    unasked[i].update(update.fields.clone());
                    finished(update.fields.status)
                        .then(|| unasked.remove(i))
                        .flatten()
                };
                if let Some(call) = done {
                    self.ran_unasked(call).await;
                }
                return;
            }
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

    /// Record `call`, which finished without asking Forge (an agent runs commands it deems
    /// read-only by itself), when it names an action: as a tool call and its result, with no
    /// policy decision.
    async fn ran_unasked(&self, call: acp::ToolCall) {
        let fields = acp::ToolCallUpdateFields::new()
            .kind(call.kind)
            .title(call.title.clone())
            .raw_input(call.raw_input.clone())
            .locations(call.locations.clone());
        let request = RequestPermissionRequest::new(
            "unasked",
            acp::ToolCallUpdate::new(call.tool_call_id.clone(), fields).meta(call.meta.clone()),
            vec![],
        );
        let target = {
            let commands = lock(&self.commands);
            permission::target(
                &request,
                &self.workspace,
                self.command_source,
                self.mcp_meta,
                &commands,
            )
        };
        let action = match target {
            // Forge's own tools record themselves when they run.
            Target::Action(Action::Mcp { server, .. }) if self.own_servers.contains(&server) => {
                return;
            }
            Target::Action(action) => action,
            _ => return,
        };
        let id = call.tool_call_id.0.to_string();
        self.call_from(&id, &action, &call.title, ToolCallSource::Agent)
            .await;
        let status = match call.status {
            acp::ToolCallStatus::Completed => "completed",
            _ => "failed",
        };
        let text: Vec<&str> = call
            .content
            .iter()
            .filter_map(|c| match c {
                acp::ToolCallContent::Content(acp::Content {
                    content: ContentBlock::Text(t),
                    ..
                }) => Some(t.text.as_str()),
                _ => None,
            })
            .collect();
        let output = (!text.is_empty()).then(|| tail(&text.join("\n")));
        self.result(&id, status, output).await;
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

/// The first option of the `wanted` kinds, in order, that doesn't save a rule of the agent's.
fn choose<'a>(
    options: &'a [acp::PermissionOption],
    wanted: &[acp::PermissionOptionKind],
) -> Option<&'a acp::PermissionOption> {
    wanted.iter().find_map(|k| {
        options
            .iter()
            .find(|o| o.kind == *k && !saves_a_rule(&o.option_id.0))
    })
}

/// Whether picking permission option `id` makes the agent save a rule of its own that would
/// let later commands run without asking Forge: Codex offers to amend its exec and network
/// policies alongside a plain approval.
fn saves_a_rule(id: &str) -> bool {
    id.starts_with("accept_execpolicy_amendment")
        || id.starts_with("apply_network_policy_amendment")
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

    #[test]
    fn options_that_save_agent_rules_are_never_picked() {
        use acp::{PermissionOption as Opt, PermissionOptionKind as Kind};
        // What Codex offers for a command: a plain approval first, then a session-wide one,
        // then rules of its own.
        let codex = [
            Opt::new("allow_once", "Yes, proceed", Kind::AllowOnce),
            Opt::new("allow_for_session", "This session", Kind::AllowAlways),
            Opt::new("accept_execpolicy_amendment", "Always", Kind::AllowAlways),
            Opt::new("decline", "No", Kind::RejectOnce),
        ];
        let allow = [Kind::AllowOnce, Kind::AllowAlways];
        assert_eq!(
            choose(&codex, &allow).map(|o| &*o.option_id.0),
            Some("allow_once")
        );
        // Without a plain approval, the session-wide one; never a rule.
        let no_once = [
            Opt::new("accept_execpolicy_amendment", "Always", Kind::AllowAlways),
            Opt::new("allow_for_session", "This session", Kind::AllowAlways),
        ];
        assert_eq!(
            choose(&no_once, &allow).map(|o| &*o.option_id.0),
            Some("allow_for_session")
        );
        let only_rules = [
            Opt::new("accept_execpolicy_amendment", "Always", Kind::AllowAlways),
            Opt::new(
                "apply_network_policy_amendment:0",
                "Host",
                Kind::AllowAlways,
            ),
        ];
        assert_eq!(choose(&only_rules, &allow), None);
        let plain = [
            Opt::new("always", "Always", Kind::AllowAlways),
            Opt::new("once", "Once", Kind::AllowOnce),
        ];
        assert_eq!(
            choose(&plain, &allow).map(|o| &*o.option_id.0),
            Some("once")
        );
        assert_eq!(
            choose(&codex, &[Kind::RejectOnce, Kind::RejectAlways]).map(|o| &*o.option_id.0),
            Some("decline")
        );
    }

    #[test]
    fn output_tails_never_start_inside_a_word() {
        assert_eq!(tail("short output"), "short output");
        let token = format!("ghp_{}", "a".repeat(36));
        let text = format!(
            "{}{token} end\nlast line\n",
            "x".repeat(RESULT_OUTPUT_LIMIT)
        );
        let cut = tail(&text);
        assert!(cut.len() <= RESULT_OUTPUT_LIMIT);
        assert_eq!(cut, "end\nlast line\n");
        // One unbroken word longer than the limit leaves nothing rather than a fragment.
        assert_eq!(tail(&"y".repeat(RESULT_OUTPUT_LIMIT + 10)), "");
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
