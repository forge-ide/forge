//! Stdio server lifecycle and byte-transparent transport.
//!
//! [`Server`] owns one language-server subprocess. Once spawned, stdin
//! writes delivered via the [`MessageTransport`] reach the child's stdin,
//! and any bytes the child writes on stdout surface as
//! [`ServerEvent::Message`] on the event channel.
//!
//! ## Framing
//!
//! The wire is **line-delimited JSON** between this crate and the stub
//! test fixture (`forge-lsp-mock-stdio`). Real LSP uses `Content-Length`
//! framing, but this transport is byte-transparent at the Rust edge:
//! messages inbound from the iframe via Tauri IPC arrive already framed
//! (or already parsed as JSON values) and are written as newline-terminated
//! JSON to the child. For a real server the caller is responsible for
//! producing the header-framed bytes; this module provides the shuttling.
//! See [`StdioTransport::send`] for the concrete wire shape.
//!
//! ## Restart policy
//!
//! If the child reaps unexpectedly, [`Server::start`] emits
//! [`ServerEvent::Exited`] and will restart up to
//! [`BackoffPolicy::max_attempts`] times within
//! [`BackoffPolicy::window`]. Exceeding the budget emits
//! [`ServerEvent::GaveUp`] and the caller must issue a fresh `start` after
//! `window` has elapsed. Sleep is driven by the [`Clock`] seam so tests
//! can drive backoff deterministically with `tokio::time::pause()`.

use std::path::PathBuf;
use std::process::Stdio as StdStdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};
use ts_rs::TS;

use crate::bootstrap::{Bootstrap, BootstrapError};
use crate::registry::ServerId;

/// Current lifecycle state of one supervised LSP server, as observed by
/// [`Server::state`] and reported by [`Server::info`].
///
/// Mirrors the vocabulary of `forge_core::ServerState` so the UI can render
/// an LSP-status pill the same way it already renders the MCP one — F-374
/// named the state-surface asymmetry as the problem, so parity is the goal.
/// The two enums stay separate because the LSP axis has no equivalent of
/// MCP's `Healthy` / `Degraded` health-check distinction: a language server
/// is either up (`Running`) or the supervisor is in a restart loop
/// (`Failed { reason }`) or out of budget (`GaveUp`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum LspState {
    /// Supervisor has not yet spawned the child for the current attempt.
    /// This is the initial state before [`Server::start`] runs.
    Starting,
    /// Child process is alive and the transport is installed. Sends via
    /// [`MessageTransport::send`] reach the child's stdin.
    Running,
    /// The most recent spawn attempt failed or the child reaped; the
    /// supervisor will retry after backoff unless the budget is exhausted.
    /// `reason` carries the last-observed failure description.
    Failed {
        /// Human-readable description of the most recent attempt failure.
        reason: String,
    },
    /// Terminal: the supervisor exhausted [`BackoffPolicy::max_attempts`]
    /// within [`BackoffPolicy::window`] and will not restart until the
    /// caller issues a fresh [`Server::start`].
    GaveUp,
}

/// Opaque snapshot returned by [`Server::info`] and by the shell's
/// `lsp_list` IPC command. Mirrors `forge_mcp::McpServerInfo`'s
/// `{ name, state, tools }` shape on the LSP axis — without `tools`,
/// which has no LSP analogue (the webview talks LSP directly).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct LspServerInfo {
    /// Server identifier — matches the `ServerId` the caller used at
    /// [`Server::from_registry`]. On the IPC boundary this is the same
    /// string the webview passed as `LspStartArgs.server`.
    pub id: String,
    /// Current lifecycle state.
    pub state: LspState,
}

/// Variables forwarded from the parent process into every spawned LSP
/// child. Mirrors the F-345 stdio-MCP allow-list.
///
/// Security posture: the child environment is **deny-by-default**. The
/// spawn path calls `env_clear()` on the `Command` and then re-injects
/// only this minimal allow-list, so a language server cannot silently
/// read parent-held credentials (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
/// `GITHUB_TOKEN`, `AWS_*`, arbitrary shell exports). Parent-env vars a
/// real LSP server actually needs to locate its own binaries (e.g.
/// `PATH` for `solargraph` → `ruby`, `HOME` for user config) are
/// included explicitly.
const PARENT_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "LANG",
    "LC_ALL",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "SystemRoot",
    "ComSpec",
    "PATHEXT",
];

/// Errors returned by [`Server`] operations.
#[derive(Debug, thiserror::Error)]
pub enum ServerError {
    /// Could not spawn the child process (binary missing, PATH issue, etc.).
    #[error("spawn failed: {0}")]
    Spawn(String),
    /// Could not capture a stdio pipe (Rust-level API failure).
    #[error("pipe unavailable: {0}")]
    Pipe(String),
    /// Writing to the child's stdin failed.
    #[error("stdin write failed: {0}")]
    StdinWrite(String),
    /// JSON encode of an outbound message failed.
    #[error("serialize outbound frame: {0}")]
    Serialize(#[from] serde_json::Error),
    /// The server is not running — a send via the transport was attempted
    /// before `Server::start` had installed stdin, or after a terminal exit.
    #[error("server not running")]
    NotRunning,
    /// Caller asked [`Server::from_registry`] for a `ServerId` the
    /// [`Bootstrap::registry`] does not know. Keeps the IPC boundary honest:
    /// the webview can only name a server that's already on the allow-list.
    #[error("unknown lsp server: {id}")]
    UnknownServerId {
        /// The offending id.
        id: String,
    },
    /// The resolved binary path escaped the cache-root sandbox. Produced by
    /// [`Server::from_registry`] as defense-in-depth against a compromised
    /// registry entry whose `binary_name` contains `..` or an absolute path.
    #[error("binary path escaped lsp sandbox: {0}")]
    SandboxEscape(PathBuf),
}

impl From<BootstrapError> for ServerError {
    fn from(err: BootstrapError) -> Self {
        match err {
            BootstrapError::SandboxEscape { path, .. } => ServerError::SandboxEscape(path),
            other => ServerError::Spawn(other.to_string()),
        }
    }
}

/// Maximum per-line byte ceiling enforced on both stdout and stderr by the
/// supervised reader loops. A line (newline-delimited sequence of bytes) that
/// exceeds this cap is discarded — up to the newline or EOF — and surfaces
/// as [`ServerEvent::Malformed`]. Closes F-351: `tokio::io::AsyncBufReadExt::lines`
/// reads until `\n` with no length cap, which lets a compromised / buggy /
/// hostile language server DoS the host via a single enormous line. 4 MiB is
/// large enough for realistic LSP diagnostics on big workspaces (~1 MiB is
/// the field-observed high-water mark) and small enough to keep the worst-case
/// resident set of a misbehaving child bounded.
pub const MAX_LSP_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Which stdio stream produced an over-cap line. Carried by
/// [`ServerEvent::Malformed`] so subscribers can distinguish a hostile
/// diagnostic payload (stdout) from a log-flood (stderr).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MalformedStream {
    /// The child's stdout — the frame-carrying stream.
    Stdout,
    /// The child's stderr — the free-form log stream.
    Stderr,
}

/// Server-emitted events surfaced on the receiver returned by
/// [`Server::take_events`].
#[derive(Debug, Clone)]
pub enum ServerEvent {
    /// Parsed JSON value emitted by the child on stdout. One event per
    /// newline-delimited JSON frame.
    Message(serde_json::Value),
    /// The child reaped. `code` is `Some(n)` for a normal exit; `None`
    /// when killed by signal. `restarts_remaining` is how many more
    /// restart attempts the supervisor is allowed within the current
    /// backoff window.
    Exited {
        /// Exit code, if available.
        code: Option<i32>,
        /// Attempts left before [`ServerEvent::GaveUp`].
        restarts_remaining: u32,
    },
    /// The supervisor has exhausted [`BackoffPolicy::max_attempts`] within
    /// [`BackoffPolicy::window`] and will not restart until the window
    /// rolls over.
    GaveUp {
        /// Failure count observed inside the window.
        attempts: u32,
        /// The window that bounds the attempts.
        window: Duration,
    },
    /// A stdout or stderr line exceeded [`MAX_LSP_LINE_BYTES`] and was
    /// discarded in-flight. F-351: closes the DoS surface a compromised /
    /// buggy / hostile language server exposes by emitting a single enormous
    /// line to the host. The reader keeps running after emitting this event,
    /// so a well-formed frame following the over-cap one still reaches the
    /// event channel.
    Malformed {
        /// Which pipe produced the over-cap line.
        stream: MalformedStream,
        /// How many bytes were buffered before the reader hit the ceiling
        /// and started discarding. Always >= [`MAX_LSP_LINE_BYTES`].
        bytes_discarded: usize,
    },
}

/// Byte-transparent transport contract. The `forge-shell` IPC layer holds
/// a `dyn MessageTransport` per active server; `lsp_send` calls
/// [`MessageTransport::send`] on the boxed transport, and a spawned
/// forwarder task pumps [`ServerEvent::Message`] events into a Tauri emit
/// targeting the owning webview.
#[async_trait]
pub trait MessageTransport: Send + Sync {
    /// Send a JSON-RPC frame to the child's stdin. Implementations
    /// serialise the value and append a newline.
    async fn send(&self, message: serde_json::Value) -> Result<(), ServerError>;
}

/// Clock seam so restart backoff can be driven deterministically.
pub trait Clock: Send + Sync {
    /// Current instant.
    fn now(&self) -> Instant;
    /// Sleep for `dur`. Real impl wraps `tokio::time::sleep`.
    fn sleep(&self, dur: Duration) -> futures_like::BoxFut<'static, ()>;
}

// Tiny local boxed-future alias so we don't pull in the `futures` crate
// just for `BoxFuture`. `std::pin::Pin<Box<dyn Future>>` is the
// idiomatic shape.
mod futures_like {
    use std::future::Future;
    use std::pin::Pin;
    /// Pinned boxed future alias.
    pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
}

/// Default [`Clock`] backed by `tokio::time`. Honors `tokio::time::pause()`
/// in tests because `tokio::time::sleep` does.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn sleep(&self, dur: Duration) -> futures_like::BoxFut<'static, ()> {
        Box::pin(tokio::time::sleep(dur))
    }
}

/// Restart-with-backoff parameters. DoD: "max 5 retries / 10 min".
#[derive(Debug, Clone, Copy)]
pub struct BackoffPolicy {
    /// Maximum restart attempts permitted inside a single window.
    pub max_attempts: u32,
    /// Rolling window over which attempts are counted.
    pub window: Duration,
    /// Base delay between attempts. Doubled each retry, up to `max_delay`.
    pub base_delay: Duration,
    /// Cap on the exponential delay.
    pub max_delay: Duration,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            window: Duration::from_secs(10 * 60),
            base_delay: Duration::from_millis(200),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// A supervised stdio language server.
pub struct Server {
    /// Stable identifier for this server. Set from [`ServerId`] when
    /// [`Server::from_registry`] is used; falls back to the program file
    /// name for the raw `new` constructor.
    id: String,
    program: PathBuf,
    args: Vec<String>,
    policy: BackoffPolicy,
    clock: Arc<dyn Clock>,
    transport: Arc<StdioTransport>,
    event_tx: mpsc::Sender<ServerEvent>,
    event_rx: Option<mpsc::Receiver<ServerEvent>>,
    /// Current lifecycle state. Updated by the supervisor loop as spawn
    /// attempts succeed or fail; readable out-of-band via
    /// [`Server::state`] and [`Server::state_handle`].
    state: Arc<Mutex<LspState>>,
}

impl Server {
    /// Build a supervised server from a [`ServerId`] resolved against the
    /// [`Bootstrap`]'s registry. The resolved binary path is enforced to
    /// live inside the cache-root sandbox before it ever reaches
    /// [`Command::new`]. This is the **only** public constructor: callers
    /// at the IPC boundary cannot name an arbitrary binary path, closing
    /// the arbitrary-binary-exec surface the raw-`PathBuf` constructor
    /// left open (F-353).
    ///
    /// `extra_args` are appended to the spec-declared argv (reserved for
    /// future per-install flags); pass `Vec::new()` when no extras are
    /// needed.
    ///
    /// Errors:
    /// - [`ServerError::UnknownServerId`] if the id is absent from the
    ///   registry.
    /// - [`ServerError::SandboxEscape`] if the resolved binary path lands
    ///   outside the cache root (defense-in-depth against a hostile spec).
    pub fn from_registry(
        id: ServerId,
        bootstrap: &Bootstrap,
        extra_args: Vec<String>,
    ) -> Result<Self, ServerError> {
        let spec = bootstrap
            .registry()
            .get(id)
            .ok_or_else(|| ServerError::UnknownServerId { id: id.to_string() })?;
        let server_dir = bootstrap.server_dir(spec)?;
        let program = server_dir.join(spec.binary_name);
        // Defense-in-depth: re-check the leaf path, not just the server dir.
        // A hostile `binary_name` (e.g. `"../../../bin/sh"`) would otherwise
        // escape because `server_dir` only validates the parent.
        bootstrap.enforce_in_sandbox(&program, spec.id.0)?;
        Ok(Self::with_id_policy_and_clock(
            spec.id.0.to_string(),
            program,
            extra_args,
            BackoffPolicy::default(),
            Arc::new(SystemClock),
        ))
    }

    /// Build a supervised server from a binary path + args. The server is
    /// not spawned until [`Server::start`].
    ///
    /// **Hidden from rustdoc.** This is the raw, trust-the-caller
    /// constructor. External callers (most notably the Tauri IPC layer)
    /// must go through [`Server::from_registry`] so the binary path is
    /// bound to the cache-root sandbox instead of accepted verbatim from
    /// the webview — F-353 closed that surface. The constructor remains
    /// reachable under `#[doc(hidden)]` so the in-tree stdio round-trip
    /// integration test can drive the fixture binary directly without the
    /// registry scaffolding.
    #[doc(hidden)]
    pub fn new(program: PathBuf, args: Vec<String>) -> Self {
        Self::with_policy_and_clock(
            program,
            args,
            BackoffPolicy::default(),
            Arc::new(SystemClock),
        )
    }

    /// Construct with explicit backoff + clock (used by tests).
    ///
    /// **Hidden from rustdoc.** Production callers use
    /// [`Server::from_registry`].
    #[doc(hidden)]
    pub fn with_policy_and_clock(
        program: PathBuf,
        args: Vec<String>,
        policy: BackoffPolicy,
        clock: Arc<dyn Clock>,
    ) -> Self {
        // Derive a best-effort id from the binary name. `from_registry`
        // overrides this with the canonical `ServerId`.
        let id = program
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("lsp")
            .to_string();
        Self::with_id_policy_and_clock(id, program, args, policy, clock)
    }

    /// Like [`Server::with_policy_and_clock`] but with an explicit stable id.
    /// Hidden from rustdoc — production callers use [`Server::from_registry`].
    #[doc(hidden)]
    pub fn with_id_policy_and_clock(
        id: String,
        program: PathBuf,
        args: Vec<String>,
        policy: BackoffPolicy,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(128);
        Self {
            id,
            program,
            args,
            policy,
            clock,
            transport: Arc::new(StdioTransport::new_empty()),
            event_tx: tx,
            event_rx: Some(rx),
            state: Arc::new(Mutex::new(LspState::Starting)),
        }
    }

    /// Stable identifier for this server (registry `ServerId` when built
    /// via [`Server::from_registry`], else the program file name).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Current lifecycle state snapshot. Safe to call concurrently with
    /// [`Server::start`].
    pub async fn state(&self) -> LspState {
        self.state.lock().await.clone()
    }

    /// Snapshot the server's current state into an [`LspServerInfo`].
    /// Mirrors `forge_mcp::McpManager::list` at the single-server scope
    /// — the shell's `lsp_list` IPC fans this across every live entry.
    pub async fn info(&self) -> LspServerInfo {
        LspServerInfo {
            id: self.id.clone(),
            state: self.state().await,
        }
    }

    /// Shared state handle. Hidden from rustdoc — the shell uses this to
    /// snapshot `lsp_list` without holding the `Server` itself across the
    /// supervisor task boundary. External callers should go through
    /// [`Server::state`] / [`Server::info`].
    #[doc(hidden)]
    pub fn state_handle(&self) -> Arc<Mutex<LspState>> {
        self.state.clone()
    }

    /// Take ownership of the event receiver. Only the first caller gets it.
    pub fn take_events(&mut self) -> Option<mpsc::Receiver<ServerEvent>> {
        self.event_rx.take()
    }

    /// Clone a [`MessageTransport`] handle. Valid even before
    /// [`Server::start`]; pre-start sends fail with [`ServerError::NotRunning`].
    pub fn transport(&self) -> Arc<dyn MessageTransport> {
        self.transport.clone()
    }

    /// Spawn the child and drive the supervisor loop. Returns once the
    /// backoff budget is exhausted (after emitting
    /// [`ServerEvent::GaveUp`]). Spawn failures *and* clean child exits
    /// both count as attempts against [`BackoffPolicy::max_attempts`]
    /// within [`BackoffPolicy::window`], so a missing binary surfaces as
    /// `GaveUp` (not a single `Err`) — callers observe the retry story
    /// uniformly. Event delivery on [`Self::take_events`] continues across
    /// restarts; the receiver yields `None` after `GaveUp` because the
    /// supervisor drops the sender on return.
    ///
    /// # Examples
    ///
    /// Resolve a bundled spec through [`Bootstrap`], build a supervised
    /// [`Server`] from the registry, and drive it in a background task
    /// so the caller can observe lifecycle events:
    ///
    /// ```no_run
    /// use forge_lsp::{Bootstrap, Server, ServerError, ServerId};
    ///
    /// # async fn example() -> Result<(), ServerError> {
    /// let bootstrap = Bootstrap::new().expect("cache root resolved");
    /// let mut server = Server::from_registry(ServerId("rust-analyzer"), &bootstrap, vec![])?;
    /// let mut events = server.take_events().expect("event rx");
    /// tokio::spawn(async move {
    ///     while let Some(evt) = events.recv().await {
    ///         tracing::info!(?evt, "lsp event");
    ///     }
    /// });
    /// server.start().await
    /// # }
    /// ```
    pub async fn start(&self) -> Result<(), ServerError> {
        let mut attempts = 0u32;
        let mut window_start = self.clock.now();

        // Fresh `start()` resets the lifecycle state. This lets callers
        // re-use a `Server` after a prior `GaveUp` without observing a
        // stale terminal state on the first pre-spawn tick.
        *self.state.lock().await = LspState::Starting;

        loop {
            // Roll window when exceeded: attempts outside a 10-min window
            // don't count against the budget.
            if self.clock.now().duration_since(window_start) > self.policy.window {
                attempts = 0;
                window_start = self.clock.now();
            }

            // Spawn + pump. Both a spawn error and a clean child exit land
            // here as a single "attempt consumed" event — tests can drive
            // either shape against the same budget. `code` is `None` for
            // spawn errors (no child existed) and for signal-killed exits.
            // `spawn_once` itself flips state → `Running` on successful
            // spawn; after it returns we flip to `Failed` (transient) or
            // `GaveUp` (terminal) below.
            let spawn_result = self.spawn_once().await;
            let code = spawn_result.as_ref().ok().copied().flatten();

            attempts = attempts.saturating_add(1);
            let remaining = self.policy.max_attempts.saturating_sub(attempts);

            let failure_reason = match &spawn_result {
                Ok(None) => "child exited (signal)".to_string(),
                Ok(Some(c)) => format!("child exited with code {c}"),
                Err(e) => format!("spawn error: {e}"),
            };

            let _ = self
                .event_tx
                .send(ServerEvent::Exited {
                    code,
                    restarts_remaining: remaining,
                })
                .await;

            if attempts >= self.policy.max_attempts {
                *self.state.lock().await = LspState::GaveUp;
                let _ = self
                    .event_tx
                    .send(ServerEvent::GaveUp {
                        attempts,
                        window: self.policy.window,
                    })
                    .await;
                return Ok(());
            }

            // Park the server in `Failed { reason }` while the backoff
            // sleep runs; the next loop iteration flips it back to
            // `Starting` → `Running` via `spawn_once`.
            *self.state.lock().await = LspState::Failed {
                reason: failure_reason,
            };

            let delay = backoff_delay(&self.policy, attempts);
            self.clock.sleep(delay).await;
            *self.state.lock().await = LspState::Starting;
        }
    }

    /// Spawn the child once, pump stdio, then return when the child reaps.
    /// The next iteration of `start`'s loop decides whether to restart.
    /// Returns `Ok(code)` if the child reaped (possibly signal-killed, in
    /// which case `code` is `None`); any transient I/O while draining is
    /// logged, not propagated, so a single spawn completes the attempt.
    async fn spawn_once(&self) -> Result<Option<i32>, ServerError> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args)
            // Security (F-353, mirrors F-345 on the MCP axis): wipe the
            // inherited environment first, then re-inject only the minimal
            // `PARENT_ENV_ALLOWLIST`. A language server should not see the
            // parent's AI-provider API keys, GitHub tokens, or arbitrary
            // shell exports. LSP servers communicate via stdio; everything
            // they need to locate their own dependencies is in the
            // allow-list.
            .env_clear();
        for key in PARENT_ENV_ALLOWLIST {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }
        cmd.stdin(StdStdio::piped())
            .stdout(StdStdio::piped())
            .stderr(StdStdio::piped())
            .kill_on_drop(true);

        let mut child = cmd.spawn().map_err(|e| ServerError::Spawn(e.to_string()))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| ServerError::Pipe("child has no stdin pipe".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ServerError::Pipe("child has no stdout pipe".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| ServerError::Pipe("child has no stderr pipe".into()))?;

        // Hand the live stdin pipe to the transport so `lsp_send` can write.
        self.transport.install(stdin).await;

        // Child is up and the transport is wired — flip to `Running`.
        // Observed via `Server::state` / `Server::info`; the `LspState`
        // transition is the current-state surface F-374 adds.
        *self.state.lock().await = LspState::Running;

        // Drain stderr at DEBUG — stderr on LSP is free-form logs. Each
        // line carries `server_id` (the program path) so a field engineer
        // can disambiguate output when multiple servers run concurrently
        // (F-386). Reads are capped at `MAX_LSP_LINE_BYTES`; over-cap lines
        // are discarded (F-351) and surface as `ServerEvent::Malformed`
        // with `MalformedStream::Stderr` so the host can observe the
        // misbehavior without buffering the payload.
        let stderr_server_id = self.program.clone();
        let stderr_tx = self.event_tx.clone();
        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            loop {
                match read_line_bounded(&mut reader, MAX_LSP_LINE_BYTES).await {
                    Ok(BoundedLine::Line(bytes)) => {
                        if bytes.is_empty() {
                            break;
                        }
                        let line = String::from_utf8_lossy(&bytes);
                        let line = line.trim_end_matches('\n').trim_end_matches('\r');
                        tracing::debug!(
                            target: "forge_lsp::server",
                            server_id = %stderr_server_id.display(),
                            stderr = %line,
                        );
                    }
                    Ok(BoundedLine::Overflow { bytes_discarded }) => {
                        tracing::warn!(
                            target: "forge_lsp::server",
                            server_id = %stderr_server_id.display(),
                            stream = "stderr",
                            bytes_discarded = bytes_discarded,
                            cap = MAX_LSP_LINE_BYTES,
                            "dropping over-cap stderr line",
                        );
                        if stderr_tx
                            .send(ServerEvent::Malformed {
                                stream: MalformedStream::Stderr,
                                bytes_discarded,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Stdout → `ServerEvent::Message`. Reads are capped at
        // `MAX_LSP_LINE_BYTES`; over-cap frames are discarded (F-351) and
        // surface as `ServerEvent::Malformed { stream: Stdout, .. }`. The
        // reader keeps running so a well-formed frame following an over-cap
        // line still reaches the event channel.
        let tx = self.event_tx.clone();
        let reader_server_id = self.program.clone();
        let reader_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            loop {
                match read_line_bounded(&mut reader, MAX_LSP_LINE_BYTES).await {
                    Ok(BoundedLine::Line(bytes)) => {
                        if bytes.is_empty() {
                            break;
                        }
                        // `read_line_bounded` yields bytes including any
                        // trailing '\n'. Parse the raw slice so UTF-8 errors
                        // flow through the same `serde_json` sad path as any
                        // other malformed frame, and trim the delimiter
                        // before the empty-line skip check.
                        let trimmed: &[u8] = trim_trailing_newline(&bytes);
                        if trimmed.iter().all(|b| b.is_ascii_whitespace()) {
                            continue;
                        }
                        match serde_json::from_slice::<serde_json::Value>(trimmed) {
                            Ok(v) => {
                                if tx.send(ServerEvent::Message(v)).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                let as_str = String::from_utf8_lossy(trimmed);
                                tracing::warn!(
                                    target: "forge_lsp::server",
                                    server_id = %reader_server_id.display(),
                                    error = %err,
                                    line = %truncate(&as_str, 512),
                                    "dropping malformed stdout frame",
                                );
                            }
                        }
                    }
                    Ok(BoundedLine::Overflow { bytes_discarded }) => {
                        tracing::warn!(
                            target: "forge_lsp::server",
                            server_id = %reader_server_id.display(),
                            stream = "stdout",
                            bytes_discarded = bytes_discarded,
                            cap = MAX_LSP_LINE_BYTES,
                            "dropping over-cap stdout line",
                        );
                        if tx
                            .send(ServerEvent::Malformed {
                                stream: MalformedStream::Stdout,
                                bytes_discarded,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        // Wait for the child to reap.
        let status = child.wait().await.ok();
        // Drain both readers (stdout and stderr pipes close with the child).
        // We await the stderr task as well so any pending `Malformed` event
        // the F-351 cap produced is observed in-order before the
        // supervisor's `Exited` reaches subscribers — without this, a slow
        // stderr drain could race the `Exited` event and surface out of
        // order (or after the receiver has already moved on).
        let _ = reader_task.await;
        let _ = stderr_task.await;
        // Clear the transport: any in-flight `send` past this point returns
        // NotRunning until the next spawn reinstalls a live stdin.
        self.transport.uninstall().await;
        Ok(status.and_then(|s| s.code()))
    }
}

/// Compute the sleep before the `nth` restart (1-based). Exponential,
/// capped at `policy.max_delay`.
fn backoff_delay(policy: &BackoffPolicy, attempt: u32) -> Duration {
    let shift = attempt.saturating_sub(1).min(16);
    let scaled = policy.base_delay.saturating_mul(1u32 << shift);
    if scaled > policy.max_delay {
        policy.max_delay
    } else {
        scaled
    }
}

/// Outcome of a single [`read_line_bounded`] call.
enum BoundedLine {
    /// A full line was read within the byte ceiling. Carries the raw bytes
    /// up to and including any trailing `\n`. An empty `Vec` indicates EOF
    /// on the stream.
    Line(Vec<u8>),
    /// The line exceeded the ceiling. All bytes through the terminating
    /// `\n` (or EOF) have been consumed and discarded; `bytes_discarded`
    /// reports the total dropped, always `>=` the cap.
    Overflow { bytes_discarded: usize },
}

/// Read bytes up to and including the next `\n` from `reader`, but never
/// buffer more than `cap` bytes in memory. Closes F-351: a compromised /
/// buggy / hostile child writing a single enormous line cannot DoS the host.
///
/// Behavior:
/// - Within the cap → returns [`BoundedLine::Line`] with the bytes read
///   (possibly ending in `\n`; may be empty at EOF).
/// - Over the cap → continues reading-and-discarding byte-by-byte from the
///   same reader until the next `\n` (or EOF) so the reader resyncs on the
///   stream without buffering, then returns [`BoundedLine::Overflow`] with
///   the total discarded count (always `>= cap`).
/// - Pure I/O error → surfaces as `Err`.
async fn read_line_bounded<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    cap: usize,
) -> std::io::Result<BoundedLine> {
    // Accumulate up to `cap+1` bytes: any overshoot proves the line exceeded
    // the ceiling without a newline. `Take` enforces the ceiling at the
    // reader layer so the `Vec` never grows past `cap+1`.
    let mut buf: Vec<u8> = Vec::new();
    let mut limited = (&mut *reader).take(cap as u64 + 1);
    let _ = limited.read_until(b'\n', &mut buf).await?;
    if buf.is_empty() {
        return Ok(BoundedLine::Line(Vec::new()));
    }
    let hit_newline = buf.last() == Some(&b'\n');
    if hit_newline || buf.len() <= cap {
        return Ok(BoundedLine::Line(buf));
    }

    // Over the cap and no newline yet — drain the remainder of the line
    // from the underlying reader. We read-and-discard via the `BufRead`
    // fill/consume pair so no intermediate buffer grows beyond the
    // reader's own internal read-ahead (8 KiB by default for tokio's
    // `BufReader`).
    let mut bytes_discarded = buf.len();
    buf.clear();
    loop {
        let chunk = reader.fill_buf().await?;
        if chunk.is_empty() {
            // EOF mid-line — still an overflow, the line never terminated.
            return Ok(BoundedLine::Overflow { bytes_discarded });
        }
        match chunk.iter().position(|&b| b == b'\n') {
            Some(idx) => {
                // Include the newline in the discard count, then stop.
                // `consume(idx + 1)` leaves any bytes *after* the newline
                // in the reader's buffer, so the next `read_line_bounded`
                // call picks up the subsequent line correctly.
                bytes_discarded = bytes_discarded.saturating_add(idx + 1);
                reader.consume(idx + 1);
                return Ok(BoundedLine::Overflow { bytes_discarded });
            }
            None => {
                let n = chunk.len();
                bytes_discarded = bytes_discarded.saturating_add(n);
                reader.consume(n);
            }
        }
    }
}

/// Strip at most one trailing `\n` (and a preceding `\r`) from `bytes`.
/// Matches `BufReader::lines` semantics for downstream JSON parsing.
fn trim_trailing_newline(bytes: &[u8]) -> &[u8] {
    let mut end = bytes.len();
    if end > 0 && bytes[end - 1] == b'\n' {
        end -= 1;
        if end > 0 && bytes[end - 1] == b'\r' {
            end -= 1;
        }
    }
    &bytes[..end]
}

/// Cap a log field at `max` bytes so a runaway frame can't flood the log
/// ring. `line` is guaranteed UTF-8 here, but we still slice on a char
/// boundary via `char_indices` to avoid panicking on multi-byte glyphs.
///
/// Ported from `forge-mcp`'s `transport::truncate` (F-375). Once the
/// Phase-3 `forge_core::process::ManagedStdioChild` extraction lands, this
/// helper will move there and both crates will share a single copy.
fn truncate(line: &str, max: usize) -> String {
    if line.len() <= max {
        return line.to_string();
    }
    let mut end = max;
    for (i, _) in line.char_indices() {
        if i > max {
            break;
        }
        end = i;
    }
    format!("{}…", &line[..end])
}

/// Byte-transparent stdio transport implementing [`MessageTransport`]. The
/// inner `ChildStdin` is swapped in and out by the supervisor around each
/// spawn; if no child is alive, `send` returns [`ServerError::NotRunning`].
pub struct StdioTransport {
    stdin: Mutex<Option<ChildStdin>>,
}

impl StdioTransport {
    fn new_empty() -> Self {
        Self {
            stdin: Mutex::new(None),
        }
    }

    async fn install(&self, stdin: ChildStdin) {
        *self.stdin.lock().await = Some(stdin);
    }

    async fn uninstall(&self) {
        *self.stdin.lock().await = None;
    }
}

#[async_trait]
impl MessageTransport for StdioTransport {
    async fn send(&self, message: serde_json::Value) -> Result<(), ServerError> {
        let mut bytes = serde_json::to_vec(&message)?;
        bytes.push(b'\n');
        let mut guard = self.stdin.lock().await;
        match guard.as_mut() {
            Some(stdin) => {
                stdin
                    .write_all(&bytes)
                    .await
                    .map_err(|e| ServerError::StdinWrite(e.to_string()))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| ServerError::StdinWrite(e.to_string()))?;
                Ok(())
            }
            None => Err(ServerError::NotRunning),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_backoff_is_five_attempts_over_ten_minutes() {
        // Locks the DoD: "max 5 retries / 10 min".
        let p = BackoffPolicy::default();
        assert_eq!(p.max_attempts, 5);
        assert_eq!(p.window, Duration::from_secs(600));
    }

    #[test]
    fn backoff_delay_grows_exponentially_but_caps() {
        let p = BackoffPolicy {
            max_attempts: 5,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(1),
        };
        assert_eq!(backoff_delay(&p, 1), Duration::from_millis(100));
        assert_eq!(backoff_delay(&p, 2), Duration::from_millis(200));
        assert_eq!(backoff_delay(&p, 3), Duration::from_millis(400));
        // Cap kicks in before the raw exponent would overflow the ceiling.
        assert_eq!(backoff_delay(&p, 10), Duration::from_secs(1));
    }

    // -----------------------------------------------------------------------
    // F-374: current-state surface parity with forge-mcp. The supervisor
    // must expose an `LspState` enum, a `Server::state` accessor, and an
    // `LspServerInfo` snapshot that `lsp_list` fans across live entries.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn state_starts_as_starting_before_spawn() {
        // Pre-`start()` the lifecycle is `Starting`. Mirrors MCP's seeded
        // `Starting` in `McpManager::new`.
        let s = Server::new(PathBuf::from("/nonexistent/binary"), Vec::new());
        assert_eq!(s.state().await, LspState::Starting);
    }

    #[tokio::test]
    async fn info_returns_id_and_state_snapshot() {
        // `Server::info` is the single-server shape the shell's `lsp_list`
        // IPC fans across every entry — id + state, nothing else.
        let s = Server::new(PathBuf::from("/opt/bin/lsp-fake"), Vec::new());
        let info = s.info().await;
        assert_eq!(info.id, "lsp-fake");
        assert_eq!(info.state, LspState::Starting);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_transitions_to_gave_up_when_budget_exhausted() {
        // DoD: terminal `GaveUp` state mirrors MCP's cap on restart
        // attempts. Drive a non-existent binary so every attempt fails
        // immediately; after the budget `state()` must report `GaveUp`.
        let policy = BackoffPolicy {
            max_attempts: 2,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let server = Arc::new(Server::with_policy_and_clock(
            PathBuf::from("/nonexistent/forge-lsp/binary"),
            Vec::new(),
            policy,
            Arc::new(SystemClock),
        ));
        let sup_server = server.clone();
        let sup = tokio::spawn(async move { sup_server.start().await });

        // Wait for the supervisor to return (it returns after `GaveUp`).
        let _ = tokio::time::timeout(Duration::from_secs(5), sup)
            .await
            .expect("supervisor must return after GaveUp");
        assert_eq!(server.state().await, LspState::GaveUp);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn state_reaches_running_after_successful_spawn() {
        // A long-lived child (`sleep 30`) keeps the supervisor inside
        // `spawn_once`, so a short time after `start()` the state must
        // have flipped to `Running`.
        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let server = Arc::new(Server::with_policy_and_clock(
            PathBuf::from("/bin/sleep"),
            vec!["30".to_string()],
            policy,
            Arc::new(SystemClock),
        ));
        let sup_server = server.clone();
        let sup = tokio::spawn(async move { sup_server.start().await });

        // Poll for `Running` up to 3s — the spawn is near-instant, but
        // the state flip happens inside the supervisor's async task so
        // we cannot assume it's observable synchronously.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let mut saw_running = false;
        while tokio::time::Instant::now() < deadline {
            if server.state().await == LspState::Running {
                saw_running = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        sup.abort();
        assert!(
            saw_running,
            "state must reach Running after a successful spawn"
        );
    }

    #[tokio::test]
    async fn lsp_state_serde_shape_matches_server_state() {
        // The wire shape has to match `forge_core::ServerState` so the UI
        // renders the LSP pill the same way it renders the MCP one. The
        // internal tag is `state`, variants are snake_case, carrier field
        // is `reason`.
        let json = serde_json::to_value(LspState::Starting).unwrap();
        assert_eq!(json, serde_json::json!({ "state": "starting" }));

        let json = serde_json::to_value(LspState::Running).unwrap();
        assert_eq!(json, serde_json::json!({ "state": "running" }));

        let json = serde_json::to_value(LspState::Failed {
            reason: "boom".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "state": "failed", "reason": "boom" })
        );

        let json = serde_json::to_value(LspState::GaveUp).unwrap();
        assert_eq!(json, serde_json::json!({ "state": "gave_up" }));
    }

    #[tokio::test]
    async fn send_before_start_returns_not_running() {
        // Pre-spawn `send` must not panic or silently succeed. The
        // StdioTransport comes up empty until the supervisor installs a
        // live stdin.
        let s = Server::new(PathBuf::from("/nonexistent/binary"), Vec::new());
        let t = s.transport();
        let err = t.send(serde_json::json!({"hello": 1})).await.unwrap_err();
        assert!(matches!(err, ServerError::NotRunning));
    }

    /// Behavioral test for DoD item 3: "handles restart-with-backoff
    /// (max 5 retries / 10 min)". Spawn a non-existent binary so every
    /// attempt fails immediately, with a tight policy so the test finishes
    /// in milliseconds. Assert exactly `max_attempts` `Exited` events land
    /// before a terminal `GaveUp`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn supervisor_emits_give_up_after_budget_exhausted() {
        let policy = BackoffPolicy {
            max_attempts: 3,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/nonexistent/forge-lsp/binary"),
            Vec::new(),
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event receiver");

        let sup = tokio::spawn(async move { server.start().await });

        let mut exited = 0u32;
        let mut saw_give_up = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(ServerEvent::Exited { .. })) => exited += 1,
                Ok(Some(ServerEvent::GaveUp { attempts, .. })) => {
                    assert_eq!(attempts, 3, "GaveUp must report the policy's budget");
                    saw_give_up = true;
                    break;
                }
                Ok(Some(ServerEvent::Message(_)))
                | Ok(Some(ServerEvent::Malformed { .. }))
                | Ok(None)
                | Err(_) => break,
            }
        }
        assert_eq!(
            exited, 3,
            "supervisor must emit exactly max_attempts Exited events"
        );
        assert!(saw_give_up, "supervisor must emit GaveUp after budget");

        // `start()` returns after GaveUp, so `sup` should finish cleanly.
        let joined = tokio::time::timeout(Duration::from_secs(2), sup)
            .await
            .expect("supervisor returns after GaveUp");
        joined.expect("join handle").expect("start returns Ok");
    }

    // -----------------------------------------------------------------------
    // F-353: arbitrary-binary-exec closure
    //
    // `Server::from_registry` must be the only public constructor. It must
    // reject any registry entry whose resolved binary path escapes the
    // cache-root sandbox, and it must reject unknown server ids — a hostile
    // IPC caller must not be able to name a binary outside the cache root
    // by manipulating either the `binary_name` on the spec or the `id`
    // itself.
    // -----------------------------------------------------------------------

    use crate::bootstrap::{Bootstrap, Downloader};
    use crate::registry::{Checksum, Registry, ServerSpec};

    struct NoopDownloader;
    #[async_trait]
    impl Downloader for NoopDownloader {
        async fn fetch(
            &self,
            _url: &str,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
            unreachable!("from_registry must not hit the network");
        }
    }

    fn single_spec_registry(spec: ServerSpec) -> Registry {
        Registry::from_entries(Box::leak(Box::new([spec])))
    }

    #[test]
    fn from_registry_rejects_unknown_server_id() {
        // DoD: IPC cannot name an id that isn't in the registry — the
        // constructor returns before any PathBuf composition happens.
        let tmp = tempfile::tempdir().unwrap();
        let bootstrap = Bootstrap::with_registry(
            tmp.path().to_path_buf(),
            Box::new(NoopDownloader),
            Registry::from_entries(&[]),
        );
        let err = match Server::from_registry(ServerId("missing"), &bootstrap, Vec::new()) {
            Ok(_) => panic!("unknown id must reject"),
            Err(e) => e,
        };
        assert!(
            matches!(err, ServerError::UnknownServerId { ref id } if id == "missing"),
            "expected UnknownServerId, got {err:?}"
        );
    }

    #[test]
    fn from_registry_rejects_binary_outside_cache_root() {
        // DoD: the core finding. A hostile spec pointing at `/bin/sh` via
        // a traversing `binary_name` must be rejected *before* Command::new
        // is ever called.
        let tmp = tempfile::tempdir().unwrap();
        // `binary_name` traverses out of the cache root into /bin. The
        // server_dir check alone doesn't cover this: the leaf path check
        // in `from_registry` is what closes the surface.
        let hostile = ServerSpec {
            id: ServerId("hostile-srv"),
            language_id: "any",
            binary_name: "../../../../../../bin/sh",
            download_url: "http://example.invalid/",
            checksum: Checksum::Pending,
        };
        let bootstrap = Bootstrap::with_registry(
            tmp.path().to_path_buf(),
            Box::new(NoopDownloader),
            single_spec_registry(hostile),
        );
        let err = match Server::from_registry(ServerId("hostile-srv"), &bootstrap, Vec::new()) {
            Ok(_) => panic!("binary outside cache root must reject"),
            Err(e) => e,
        };
        assert!(
            matches!(err, ServerError::SandboxEscape(_)),
            "expected SandboxEscape, got {err:?}"
        );
    }

    #[test]
    fn from_registry_accepts_binary_inside_cache_root() {
        // Positive case: a well-formed spec resolves to a path rooted
        // under the cache. We don't spawn; `from_registry` is pure path
        // resolution.
        let tmp = tempfile::tempdir().unwrap();
        let benign = ServerSpec {
            id: ServerId("benign-srv"),
            language_id: "any",
            binary_name: "bin",
            download_url: "http://example.invalid/",
            checksum: Checksum::Pending,
        };
        let bootstrap = Bootstrap::with_registry(
            tmp.path().to_path_buf(),
            Box::new(NoopDownloader),
            single_spec_registry(benign),
        );
        let _server = Server::from_registry(ServerId("benign-srv"), &bootstrap, Vec::new())
            .expect("benign spec must resolve");
    }

    /// Proves the F-353 env-scrub: a language-server child does not see a
    /// sentinel env var set in the parent. Mirrors the F-345 canary shape
    /// in `forge-mcp::transport::stdio`.
    ///
    /// We can't use the Tauri mock fixture here (cross-crate) and the
    /// supervisor's backoff loop swallows child exit codes, so we drive
    /// the scrub via a shell-level assertion: spawn `sh -c 'test -z
    /// "$CANARY" && exit 0 || exit 42'`. Pre-fix (no `env_clear`) the
    /// child would observe `CANARY=leak-me` and exit 42. Post-fix the
    /// child exits 0, which the supervisor surfaces as `code: Some(0)`
    /// on the event channel.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn child_env_is_scrubbed_of_parent_secrets() {
        // Pick a key unlikely to collide with anything CI exports.
        const CANARY_KEY: &str = "FORGE_LSP_F353_CANARY";
        std::env::set_var(CANARY_KEY, "leak-me");

        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/bin/sh"),
            vec![
                "-c".to_string(),
                format!("test -z \"${CANARY_KEY}\" && exit 0 || exit 42"),
            ],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut exit_code: Option<i32> = None;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(ServerEvent::Exited { code, .. })) => {
                    exit_code = code;
                    break;
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), sup).await;
        std::env::remove_var(CANARY_KEY);

        assert_eq!(
            exit_code,
            Some(0),
            "child saw parent's CANARY env — env_clear() is missing or bypassed"
        );
    }

    // -----------------------------------------------------------------------
    // F-386: every tracing call in server.rs must carry a `server_id` field
    // so a field engineer can disambiguate logs when two LSP servers are
    // running concurrently. The identifier is the program path — the only
    // always-available handle on `Server` and the one the issue itself
    // names as sufficient.
    // -----------------------------------------------------------------------

    use std::io;
    use std::sync::{Mutex as StdMutex, Once, OnceLock};

    /// Shared capture buffer the global tracing subscriber writes into. A
    /// single global subscriber (installed under a `Once`) avoids the
    /// "set-global-default can only run once" constraint while a `StdMutex`
    /// around a `TEST_LOCK` keeps the two server_id tests from interleaving
    /// into the same buffer.
    fn capture_buf() -> &'static StdMutex<Vec<u8>> {
        static BUF: OnceLock<StdMutex<Vec<u8>>> = OnceLock::new();
        BUF.get_or_init(|| StdMutex::new(Vec::new()))
    }

    /// Serializes the two log-capture tests so they don't share the global
    /// buffer concurrently. Tests in this file that don't read the buffer
    /// are unaffected.
    fn capture_test_lock() -> &'static StdMutex<()> {
        static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| StdMutex::new(()))
    }

    /// `MakeWriter` adapter that appends into `capture_buf`.
    struct CaptureWriter;
    impl io::Write for CaptureWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            capture_buf().lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for CaptureWriter {
        type Writer = CaptureWriter;
        fn make_writer(&'a self) -> Self::Writer {
            CaptureWriter
        }
    }

    fn install_capture_subscriber() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let subscriber = tracing_subscriber::fmt()
                .with_max_level(tracing::Level::DEBUG)
                .with_ansi(false)
                .with_writer(CaptureWriter)
                .finish();
            tracing::subscriber::set_global_default(subscriber)
                .expect("install capture subscriber");
        });
    }

    fn drain_capture() -> String {
        let mut buf = capture_buf().lock().unwrap();
        let out = String::from_utf8(buf.clone()).expect("utf-8 logs");
        buf.clear();
        out
    }

    /// stderr-drain debug line must carry `server_id`. Spawn a child that
    /// writes to stderr and exits; the drain task logs at DEBUG.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    // `_guard` intentionally serializes capture-reading tests across awaits;
    // the whole purpose of the lock is to hold it for the test duration.
    #[allow(clippy::await_holding_lock)]
    async fn stderr_drain_log_carries_server_id_field() {
        let _guard = capture_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        install_capture_subscriber();
        let _ = drain_capture();

        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let program = PathBuf::from("/bin/sh");
        let mut server = Server::with_policy_and_clock(
            program.clone(),
            vec![
                "-c".to_string(),
                "echo forge-lsp-stderr-canary >&2".to_string(),
            ],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(ServerEvent::GaveUp { .. })) | Ok(None) | Err(_) => break,
                _ => continue,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), sup).await;

        let logs = drain_capture();
        assert!(
            logs.contains("forge-lsp-stderr-canary"),
            "expected the drained stderr line to reach the debug log, got:\n{logs}"
        );
        assert!(
            logs.contains("server_id"),
            "stderr-drain log must carry a structured server_id field, got:\n{logs}"
        );
        // The field value is the program path; presence of the path string
        // proves the field is bound to the right identifier, not a bare name.
        assert!(
            logs.contains(program.display().to_string().as_str()),
            "server_id field must carry the program path value, got:\n{logs}"
        );
    }

    /// malformed-frame warn must carry `server_id`. Spawn a child that
    /// writes a non-JSON line to stdout; the reader task falls into the
    /// `warn!` branch.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::await_holding_lock)]
    async fn malformed_frame_warn_carries_server_id_field() {
        let _guard = capture_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        install_capture_subscriber();
        let _ = drain_capture();

        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let program = PathBuf::from("/bin/sh");
        let mut server = Server::with_policy_and_clock(
            program.clone(),
            vec!["-c".to_string(), "echo not-a-json-frame".to_string()],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(ServerEvent::GaveUp { .. })) | Ok(None) | Err(_) => break,
                _ => continue,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), sup).await;

        let logs = drain_capture();
        assert!(
            logs.contains("dropping malformed stdout frame"),
            "expected the malformed-frame warn to fire, got:\n{logs}"
        );
        assert!(
            logs.contains("server_id"),
            "malformed-frame warn must carry a structured server_id field, got:\n{logs}"
        );
        assert!(
            logs.contains(program.display().to_string().as_str()),
            "server_id field must carry the program path value, got:\n{logs}"
        );
    }

    // -----------------------------------------------------------------------
    // F-376: StdioTransport sad-path coverage
    //
    // The happy-path round-trip is already covered by
    // `tests/stdio_roundtrip.rs`. These two tests close the sad-path gaps
    // the issue calls out:
    //   - a malformed stdout frame must be dropped, not terminate the
    //     relay (`warn!` arm around `server.rs`'s stdout reader).
    //   - a `send` after the child has exited must return `NotRunning`
    //     (the transport is `uninstall`-ed before the `Exited` event
    //     reaches subscribers).
    // Both mirror forge-mcp's existing `drops_malformed_lines_without_closing_stream`
    // and `send_errors_when_child_stdin_is_closed`.
    // -----------------------------------------------------------------------

    /// A single malformed stdout line must not kill the reader loop; any
    /// valid JSON frame written after it still reaches the event channel.
    ///
    /// Fixture: `/bin/sh -c 'printf "not json\n{\"ok\":true}\n"'` — the
    /// first line is non-JSON (triggering the `warn!` drop arm), the
    /// second is a valid JSON frame.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn drops_malformed_lines_without_closing_stream() {
        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/bin/sh"),
            vec![
                "-c".to_string(),
                "printf 'not json\\n{\"ok\":true}\\n'".to_string(),
            ],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        let mut saw_valid_message = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(ServerEvent::Message(v))) => {
                    // Only the valid frame must surface. Malformed line
                    // must have been dropped by the `warn!` arm.
                    assert_eq!(
                        v,
                        serde_json::json!({"ok": true}),
                        "reader must surface the valid JSON frame intact"
                    );
                    saw_valid_message = true;
                }
                Ok(Some(ServerEvent::Exited { .. })) => break,
                Ok(Some(ServerEvent::GaveUp { .. }))
                | Ok(Some(ServerEvent::Malformed { .. }))
                | Ok(None)
                | Err(_) => break,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), sup).await;

        assert!(
            saw_valid_message,
            "reader must keep running past the malformed line and deliver \
             the subsequent valid frame"
        );
    }

    /// After the supervised child reaps, the transport is `uninstall`-ed
    /// and any subsequent `send` must return `NotRunning` — not panic, not
    /// succeed into a closed pipe. Mirrors
    /// forge-mcp::`send_errors_when_child_stdin_is_closed` but tightened to
    /// the actual `ServerError::NotRunning` shape the forge-lsp API
    /// documents.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_after_child_exit_returns_not_running() {
        // Long backoff so the supervisor, after its single allowed attempt,
        // is guaranteed to sit in `clock.sleep` — giving us a deterministic
        // post-`uninstall` window to probe `send`. `max_attempts = 1` means
        // `start` falls straight through to `GaveUp` after the `Exited`
        // event fires, but the `Exited` event is emitted *after*
        // `spawn_once` returns, which is in turn *after* `transport.uninstall`
        // runs (see server.rs `spawn_once` tail). So observing `Exited` is
        // a valid "now the transport is uninstalled" signal.
        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), "exit 0".to_string()],
            policy,
            Arc::new(SystemClock),
        );
        let transport = server.transport();
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        // Wait for `Exited` — proves `uninstall` has already run.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut saw_exit = false;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(ServerEvent::Exited { .. })) => {
                    saw_exit = true;
                    break;
                }
                Ok(Some(_)) => continue,
                Ok(None) | Err(_) => break,
            }
        }
        assert!(
            saw_exit,
            "supervisor must emit Exited after the child reaps"
        );

        // Now the transport is uninstalled: `send` must return NotRunning.
        let err = transport
            .send(serde_json::json!({"jsonrpc":"2.0","id":1}))
            .await
            .expect_err("send after child exit must fail");
        assert!(
            matches!(err, ServerError::NotRunning),
            "expected NotRunning after child exit, got {err:?}"
        );

        let _ = tokio::time::timeout(Duration::from_secs(2), sup).await;
    }

    // -----------------------------------------------------------------------
    // F-375: malformed-frame warn must carry a size-capped `line` field so
    // a runaway stdout frame can't flood the log ring. Ported from
    // forge-mcp's `truncate` pattern.
    // -----------------------------------------------------------------------

    #[test]
    fn truncate_is_utf8_safe() {
        // Long ASCII + one multi-byte glyph at the tail forces the slice to
        // land on a char boundary. A naive byte-index slice would panic.
        let s = "a".repeat(600) + "é";
        let out = truncate(&s, 300);
        assert!(out.ends_with('…'));
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }

    /// A malformed stdout frame longer than the 512-byte cap must be
    /// logged with a truncated `line` field, not the raw bytes.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::await_holding_lock)]
    async fn malformed_frame_warn_carries_truncated_line_field() {
        let _guard = capture_test_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        install_capture_subscriber();
        let _ = drain_capture();

        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        // 2000-byte non-JSON line — well past the 512-byte cap.
        let long_bad_line = "x".repeat(2000);
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), format!("printf '{long_bad_line}\\n'")],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                Ok(Some(ServerEvent::GaveUp { .. })) | Ok(None) | Err(_) => break,
                _ => continue,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(2), sup).await;

        let logs = drain_capture();
        assert!(
            logs.contains("dropping malformed stdout frame"),
            "expected the malformed-frame warn to fire, got:\n{logs}"
        );
        assert!(
            logs.contains("line="),
            "malformed-frame warn must carry a structured `line` field, got:\n{logs}"
        );
        assert!(
            logs.contains('…'),
            "the `line` field must be truncated with the ellipsis marker, got:\n{logs}"
        );
        // The raw 2000-byte line must not reach the log ring intact.
        assert!(
            !logs.contains(&"x".repeat(1000)),
            "the untruncated 2000-byte line must not appear in logs"
        );
    }

    // -----------------------------------------------------------------------
    // F-351: stdio reader must enforce a documented max-line ceiling. A
    // compromised / buggy / hostile language server that writes a single
    // enormous line (no newline) must not drive `forge-lsp` into unbounded
    // memory use. Over-cap events surface as `ServerEvent::Malformed` and
    // the reader loop stays alive for subsequent frames.
    // -----------------------------------------------------------------------

    /// A stdout line longer than `MAX_LSP_LINE_BYTES` must surface a
    /// `Malformed { stream: Stdout }` event, be discarded, and the reader
    /// must keep running so a subsequent well-formed frame still reaches
    /// the event channel.
    ///
    /// Fixture: `printf '<cap+1 'x'>\n{"ok":true}\n'` — first line is
    /// over-cap, second is a valid JSON frame. We deliberately stay within
    /// argv-size limits by using a modest test cap via the construction
    /// seam; the cap is reached before OS exec limits matter.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stdout_over_cap_emits_malformed_and_keeps_reader_alive() {
        // A 6 MiB line comfortably exceeds the 4 MiB cap. We use `yes` to
        // avoid blowing out argv — pipe it through `head -c` to bound the
        // byte count, then append a valid JSON frame. POSIX `sh` handles
        // this without external scripting.
        let script = r#"
            head -c 6291456 /dev/zero | tr '\0' 'x'
            printf '\n{"ok":true}\n'
        "#;

        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), script.to_string()],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        let mut saw_malformed_stdout = false;
        let mut saw_valid_message = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(ServerEvent::Malformed {
                    stream: MalformedStream::Stdout,
                    ..
                })) => saw_malformed_stdout = true,
                Ok(Some(ServerEvent::Message(v))) => {
                    assert_eq!(
                        v,
                        serde_json::json!({"ok": true}),
                        "reader must deliver the valid frame after dropping the over-cap line"
                    );
                    saw_valid_message = true;
                }
                Ok(Some(ServerEvent::Exited { .. })) => break,
                Ok(Some(ServerEvent::GaveUp { .. })) | Ok(None) | Err(_) => break,
                Ok(Some(_)) => continue,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), sup).await;

        assert!(
            saw_malformed_stdout,
            "over-cap stdout line must surface ServerEvent::Malformed"
        );
        assert!(
            saw_valid_message,
            "reader must survive the over-cap line and deliver the next valid JSON frame"
        );
    }

    /// A stderr line longer than `MAX_LSP_LINE_BYTES` must be dropped
    /// (not buffered). The supervisor must reach `Exited` cleanly — the
    /// drain task cannot deadlock on an unbounded buffer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stderr_over_cap_drops_without_crashing() {
        let script = r#"
            head -c 6291456 /dev/zero | tr '\0' 'x' >&2
            exit 0
        "#;

        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), script.to_string()],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        // The stderr drain task is awaited by `spawn_once` before `Exited`
        // fires, so the `Malformed` event arrives in-order. We still drain
        // past `Exited` to cover the (legal) ordering where `Exited`
        // lands before the last stderr event — the channel closes on
        // `GaveUp`, which gives us a guaranteed terminator because
        // `max_attempts = 1`.
        let mut saw_exit = false;
        let mut saw_malformed_stderr = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
                Ok(Some(ServerEvent::Malformed {
                    stream: MalformedStream::Stderr,
                    ..
                })) => saw_malformed_stderr = true,
                Ok(Some(ServerEvent::Exited { code, .. })) => {
                    assert_eq!(code, Some(0), "child must exit cleanly");
                    saw_exit = true;
                }
                Ok(Some(ServerEvent::GaveUp { .. })) | Ok(None) | Err(_) => break,
                _ => continue,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(5), sup).await;

        assert!(
            saw_exit,
            "supervisor must observe the child exiting cleanly"
        );
        assert!(
            saw_malformed_stderr,
            "over-cap stderr line must surface ServerEvent::Malformed"
        );
    }

    /// The DoD regression test. Feed 16 MiB of no-newline stdout and
    /// assert a single `Malformed` event fires — the reader must discard
    /// the over-cap bytes without growing its buffer past the cap.
    ///
    /// We cannot measure process RSS from inside the test portably, so we
    /// assert the two observable-from-the-channel invariants: the over-cap
    /// event fires, and the reader task terminates (no deadlock).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stdout_sixteen_mib_no_newline_is_bounded() {
        // 16 MiB of 'x', no trailing newline.
        let script = r#"
            head -c 16777216 /dev/zero | tr '\0' 'x'
        "#;

        let policy = BackoffPolicy {
            max_attempts: 1,
            window: Duration::from_secs(600),
            base_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(1),
        };
        let mut server = Server::with_policy_and_clock(
            PathBuf::from("/bin/sh"),
            vec!["-c".to_string(), script.to_string()],
            policy,
            Arc::new(SystemClock),
        );
        let mut rx = server.take_events().expect("event rx");
        let sup = tokio::spawn(async move { server.start().await });

        let mut saw_malformed = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
                Ok(Some(ServerEvent::Malformed {
                    stream: MalformedStream::Stdout,
                    bytes_discarded,
                })) => {
                    // The discard count must be at or above the cap — it's
                    // the proof the reader stopped buffering at the ceiling.
                    assert!(
                        bytes_discarded >= MAX_LSP_LINE_BYTES,
                        "bytes_discarded ({bytes_discarded}) must be >= MAX_LSP_LINE_BYTES ({MAX_LSP_LINE_BYTES})"
                    );
                    saw_malformed = true;
                }
                Ok(Some(ServerEvent::Exited { .. })) => break,
                Ok(Some(ServerEvent::GaveUp { .. })) | Ok(None) | Err(_) => break,
                _ => continue,
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(10), sup).await;

        assert!(
            saw_malformed,
            "16 MiB no-newline stdout must surface ServerEvent::Malformed"
        );
    }
}
