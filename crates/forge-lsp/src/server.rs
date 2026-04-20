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
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, Mutex};

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
    program: PathBuf,
    args: Vec<String>,
    policy: BackoffPolicy,
    clock: Arc<dyn Clock>,
    transport: Arc<StdioTransport>,
    event_tx: mpsc::Sender<ServerEvent>,
    event_rx: Option<mpsc::Receiver<ServerEvent>>,
}

impl Server {
    /// Build a supervised server from a binary path + args. The server is
    /// not spawned until [`Server::start`].
    pub fn new(program: PathBuf, args: Vec<String>) -> Self {
        Self::with_policy_and_clock(
            program,
            args,
            BackoffPolicy::default(),
            Arc::new(SystemClock),
        )
    }

    /// Construct with explicit backoff + clock (used by tests).
    pub fn with_policy_and_clock(
        program: PathBuf,
        args: Vec<String>,
        policy: BackoffPolicy,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let (tx, rx) = mpsc::channel(128);
        Self {
            program,
            args,
            policy,
            clock,
            transport: Arc::new(StdioTransport::new_empty()),
            event_tx: tx,
            event_rx: Some(rx),
        }
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
    pub async fn start(&self) -> Result<(), ServerError> {
        let mut attempts = 0u32;
        let mut window_start = self.clock.now();

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
            let code = self.spawn_once().await.unwrap_or_default();

            attempts = attempts.saturating_add(1);
            let remaining = self.policy.max_attempts.saturating_sub(attempts);
            let _ = self
                .event_tx
                .send(ServerEvent::Exited {
                    code,
                    restarts_remaining: remaining,
                })
                .await;

            if attempts >= self.policy.max_attempts {
                let _ = self
                    .event_tx
                    .send(ServerEvent::GaveUp {
                        attempts,
                        window: self.policy.window,
                    })
                    .await;
                return Ok(());
            }

            let delay = backoff_delay(&self.policy, attempts);
            self.clock.sleep(delay).await;
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
            .stdin(StdStdio::piped())
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

        // Drain stderr at DEBUG — stderr on LSP is free-form logs.
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "forge_lsp::server", stderr = %line);
            }
        });

        // Stdout → `ServerEvent::Message`.
        let tx = self.event_tx.clone();
        let reader_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        if line.trim().is_empty() {
                            continue;
                        }
                        match serde_json::from_str::<serde_json::Value>(&line) {
                            Ok(v) => {
                                if tx.send(ServerEvent::Message(v)).await.is_err() {
                                    break;
                                }
                            }
                            Err(err) => {
                                tracing::warn!(
                                    target: "forge_lsp::server",
                                    error = %err,
                                    "dropping malformed stdout frame",
                                );
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
        });

        // Wait for the child to reap.
        let status = child.wait().await.ok();
        // Drain the reader (stdout closed with the child).
        let _ = reader_task.await;
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
                Ok(Some(ServerEvent::Message(_))) | Ok(None) | Err(_) => break,
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
}
