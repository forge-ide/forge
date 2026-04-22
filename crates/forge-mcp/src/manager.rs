//! MCP server lifecycle manager.
//!
//! [`McpManager`] is the single point of control for every MCP server the
//! user has configured. It owns the transport handle for each server, a
//! JSON-RPC request/response router, a 30s `tools/list` health-check
//! ticker, and a bounded restart policy (max 5 tries per 10-minute
//! sliding window with exponential backoff 1s → 16s). State transitions
//! are fanned out on [`McpManager::state_stream`] as [`McpStateEvent`]s
//! for the IPC layer.
//!
//! The manager is deliberately transport-agnostic above the
//! connect-boundary: once a transport handle is built (stdio or http)
//! the rest of the lifecycle is identical. That keeps the F-128 /
//! F-129 / F-130 layers cleanly separated.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use futures::stream::BoxStream;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use ts_rs::TS;

use crate::transport::{Http, HttpEvent, Stdio, StdioEvent};
use crate::{McpServerSpec, ServerKind};
// F-155: `ServerState` and `McpStateEvent` now live in forge-core so
// `forge_core::Event::McpState` can carry them without creating a cycle.
// forge-mcp re-exports them from `lib.rs` for external callers.
pub use forge_core::{McpStateEvent, ServerState, Tool};

/// Health-check cadence. Every server is pinged with `tools/list` on
/// this interval while running; a failed ping degrades the server and
/// kicks the restart policy.
pub const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(30);

/// Per-request timeout applied to `call()` and the health-check ping.
/// A silently-dead server must not hang tool dispatch forever; the
/// manager surfaces a timeout error and the restart policy takes over.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum number of restart attempts inside `RESTART_WINDOW`. When the
/// window is full the manager refuses further restarts and parks the
/// server in [`ServerState::Failed`] until the user intervenes.
pub const MAX_RESTARTS_PER_WINDOW: usize = 5;

/// Sliding window for counting restart attempts.
pub const RESTART_WINDOW: Duration = Duration::from_secs(600);

/// Backoff ladder for restart attempts. Capped at the last entry once
/// exhausted. Matches the DoD ladder: 1s, 2s, 4s, 8s, 16s.
pub const RESTART_BACKOFF_LADDER: &[Duration] = &[
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
];

/// Depth of the state-event broadcast channel. A slow consumer that
/// falls more than this many events behind is lagged — UI consumers
/// treat a lag as "refetch via `list()`" rather than a fatal error.
const STATE_CHANNEL_CAPACITY: usize = 64;

/// Opaque summary returned by [`McpManager::list`]. Keep it minimal;
/// downstream IPC consumers can grow it over time without disturbing
/// the manager internals.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct McpServerInfo {
    pub name: String,
    pub state: ServerState,
    /// Tools exposed by the server from its most recent `tools/list`
    /// response. Empty when the server is not yet `Healthy`.
    pub tools: Vec<Tool>,
}

/// The live handle + derived state for one managed server.
struct ManagedServer {
    spec: McpServerSpec,
    /// Task driving this server's lifecycle (spawn + handshake +
    /// event-pump + restart). `None` before the first `start()` and
    /// after `stop()`; `Some` while a driver is running or has
    /// completed. We treat "completed" (`is_finished() == true`) the
    /// same as `None` for restart admission — previously a pre-aborted
    /// dummy handle was used as the `None` sentinel, but a freshly
    /// aborted `JoinHandle` does not always observe as finished before
    /// the runtime has polled it once, which caused the first `start()`
    /// on a brand-new manager to race the abort and occasionally
    /// return early without spawning any driver.
    driver: Option<JoinHandle<()>>,
    /// Shared mutable data the driver updates and the public API
    /// (list/call) reads. Separating this out of the driver keeps
    /// calls non-blocking on driver progress.
    shared: Arc<ServerShared>,
    /// Stop signal: when set, the driver exits its loop cleanly.
    stop_tx: Option<oneshot::Sender<()>>,
}

/// Shared-mutable per-server state. All fields are protected by the
/// single outer Mutex on the manager's server map — this struct is
/// itself only cloneable as an `Arc<_>` and every field that needs
/// independent mutation uses its own lock.
struct ServerShared {
    name: String,
    state: Mutex<ServerState>,
    /// `None` until the first successful `initialize`; then holds the
    /// active connection used by `call()` and health checks.
    conn: Mutex<Option<Arc<Connection>>>,
    /// Cached tool list from the latest `tools/list` response.
    tools: Mutex<Vec<Tool>>,
    /// Broadcast channel shared across all servers — passed in so
    /// every server emits to the same `state_stream()`.
    state_tx: broadcast::Sender<McpStateEvent>,
}

impl ServerShared {
    async fn publish(&self, state: ServerState) {
        *self.state.lock().await = state.clone();
        // Broadcast send only fails when there are no active receivers,
        // which is legal — the stream is optional.
        let _ = self.state_tx.send(McpStateEvent {
            server: self.name.clone(),
            state,
            at: Utc::now(),
        });
    }
}

/// One live JSON-RPC connection plus its outbound command channel.
///
/// The pump task *owns* the transport exclusively — so it can `select!`
/// on inbound events (from `recv`) and outbound commands (from this
/// channel) without ever sharing the transport across tasks. `call()`
/// sends a [`Command::Send`] with a oneshot for its response; the pump
/// routes responses by id.
struct Connection {
    /// Monotonic request-id generator. MCP requires integer ids; we use
    /// `u64` and assume the server accepts that representation.
    next_id: AtomicU64,
    /// Outbound-command channel. Closed when the pump exits, which
    /// surfaces as a `Closed` error on `send_cmd`.
    cmd_tx: mpsc::Sender<Command>,
    /// Pump task handle. Aborted via [`Drop`] so a stale connection
    /// cannot outlive the manager.
    pump: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.pump.try_lock() {
            if let Some(handle) = guard.take() {
                handle.abort();
            }
        }
    }
}

/// Commands the pump executes on behalf of callers. Currently just
/// `Send` (optionally with a response slot); keeping it enum-shaped
/// leaves room for future operations like explicit pump shutdown.
enum Command {
    /// Serialise and forward `frame` on the transport. When `respond_to`
    /// is `Some(id)`, the pump registers a pending slot so the inbound
    /// pump can route the reply.
    Send {
        frame: serde_json::Value,
        /// Response channel. Closed + removed from the pending table
        /// when either (a) the matching response arrives, or (b) the
        /// pump task exits (connection torn down).
        respond_to: Option<(u64, oneshot::Sender<serde_json::Value>)>,
    },
}

/// Transport half used by the pump. Owned by the pump task — never
/// shared. The enum shape keeps dispatch monomorphic.
enum TransportHalf {
    Stdio(Stdio),
    Http(Http),
}

impl TransportHalf {
    async fn send(&mut self, value: serde_json::Value) -> Result<()> {
        match self {
            TransportHalf::Stdio(s) => s.send(value).await,
            TransportHalf::Http(h) => h.send(value).await,
        }
    }

    async fn recv(&mut self) -> TransportEvent {
        match self {
            TransportHalf::Stdio(s) => match s.recv().await {
                Some(StdioEvent::Message(v)) => TransportEvent::Message(v),
                Some(StdioEvent::Exit(status)) => {
                    TransportEvent::Closed(format!("stdio child exited: {status:?}"))
                }
                None => TransportEvent::Closed("stdio channel closed".into()),
            },
            TransportHalf::Http(h) => match h.recv().await {
                Some(HttpEvent::Message(v)) => TransportEvent::Message(v),
                // F-361: transport-driven terminal event. Surfacing this
                // as `Closed` lets the lifecycle driver treat a dead
                // HTTP server identically to a crashed stdio child —
                // Degraded within ms, not on the 30s health-check tick.
                Some(HttpEvent::Closed(reason)) => {
                    TransportEvent::Closed(format!("http transport closed: {reason}"))
                }
                None => TransportEvent::Closed("http channel closed".into()),
            },
        }
    }
}

enum TransportEvent {
    Message(serde_json::Value),
    Closed(String),
}

/// The public manager. Cheap to `clone()` — everything behind it is
/// `Arc`-wrapped so handing copies to tool dispatchers is zero-cost.
#[derive(Clone)]
pub struct McpManager {
    inner: Arc<ManagerInner>,
}

/// Tunable overrides, primarily for tests that need to compress the
/// 30s health-check cadence so the restart-backoff path fits inside a
/// reasonable test timeout. Production code should always use the
/// default [`LifecycleTuning::default`] — which matches the DoD.
#[derive(Debug, Clone, Copy)]
pub struct LifecycleTuning {
    pub health_check_interval: Duration,
}

impl Default for LifecycleTuning {
    fn default() -> Self {
        Self {
            health_check_interval: HEALTH_CHECK_INTERVAL,
        }
    }
}

struct ManagerInner {
    servers: Mutex<BTreeMap<String, ManagedServer>>,
    state_tx: broadcast::Sender<McpStateEvent>,
    /// Restart history, per server name. A sliding window of `Instant`s
    /// of when each restart *started*; older than `RESTART_WINDOW`
    /// entries are pruned on every consult. Wrapped in `Arc` so the
    /// lifecycle driver (spawned task) can hold a handle without
    /// borrowing from `self`.
    restart_history: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    /// Lifecycle timing overrides. Production uses the defaults; tests
    /// can shrink the health-check interval.
    tuning: LifecycleTuning,
}

impl McpManager {
    /// Build a manager from a loaded spec map (see [`crate::config`]).
    ///
    /// **Does not connect.** Servers start in the map as unmanaged
    /// entries; [`McpManager::start`] is the explicit trigger. This
    /// matches `forge-lsp` / `forge-providers` where instantiation is
    /// cheap and connect-on-use is the norm.
    pub fn new(config: BTreeMap<String, McpServerSpec>) -> Self {
        Self::with_tuning(config, LifecycleTuning::default())
    }

    /// Like [`McpManager::new`] but with explicit [`LifecycleTuning`].
    /// Exposed so tests can compress the 30s health-check cadence;
    /// production callers should use [`McpManager::new`].
    pub fn with_tuning(config: BTreeMap<String, McpServerSpec>, tuning: LifecycleTuning) -> Self {
        let (state_tx, _rx) = broadcast::channel(STATE_CHANNEL_CAPACITY);
        let servers = config
            .into_iter()
            .map(|(name, spec)| {
                let shared = Arc::new(ServerShared {
                    name: name.clone(),
                    state: Mutex::new(ServerState::Starting),
                    conn: Mutex::new(None),
                    tools: Mutex::new(Vec::new()),
                    state_tx: state_tx.clone(),
                });
                // Seed as `Starting` but don't spawn yet — `start()` is
                // explicit. The driver slot is `None` until the first
                // `start()` replaces it with a real handle.
                let managed = ManagedServer {
                    spec,
                    driver: None,
                    shared,
                    stop_tx: None,
                };
                (name, managed)
            })
            .collect();

        // Seed every server as `Starting` in the broadcast even though
        // we haven't spawned yet — keeps list() and the stream in sync.
        // Actually: skip this; the first `start()` call will publish.
        // Subscribers created now would see noise if we published here.

        Self {
            inner: Arc::new(ManagerInner {
                servers: Mutex::new(servers),
                state_tx,
                restart_history: Arc::new(Mutex::new(HashMap::new())),
                tuning,
            }),
        }
    }

    /// Snapshot of every configured server with its current state and
    /// last-seen tool list.
    pub async fn list(&self) -> Vec<McpServerInfo> {
        let servers = self.inner.servers.lock().await;
        let mut out = Vec::with_capacity(servers.len());
        for (name, managed) in servers.iter() {
            let state = managed.shared.state.lock().await.clone();
            let tools = managed.shared.tools.lock().await.clone();
            out.push(McpServerInfo {
                name: name.clone(),
                state,
                tools,
            });
        }
        out
    }

    /// Subscribe to the state-event stream. Every active call sees
    /// events from the moment of subscription forward. A lagged
    /// consumer silently drops events — pair with `list()` to resync.
    pub fn state_stream(&self) -> BoxStream<'static, McpStateEvent> {
        let rx = self.inner.state_tx.subscribe();
        BroadcastStream::new(rx)
            .filter_map(|r| async move { r.ok() })
            .boxed()
    }

    /// Start `name`: spawn the transport, run the `initialize`
    /// handshake, and install the lifecycle driver that health-checks
    /// and restarts on failure.
    ///
    /// Idempotent: starting an already-running server is a no-op.
    pub async fn start(&self, name: &str) -> Result<()> {
        // Scope the map lock tightly so we don't hold it across the
        // `tokio::spawn` — spawn is non-blocking but keeping the lock
        // across it means every `list()` / `state_stream()` call must
        // wait behind us for no reason.
        let mut servers = self.inner.servers.lock().await;
        let managed = servers
            .get_mut(name)
            .ok_or_else(|| anyhow!("unknown MCP server {name:?}"))?;

        // Already running? A live driver is `Some` and not yet
        // finished. `None` means we've never started (or have been
        // stopped) and a `Some(handle).is_finished()` means the
        // previous driver exited cleanly — in either case we're free
        // to spawn a fresh one.
        if let Some(handle) = &managed.driver {
            if !handle.is_finished() {
                return Ok(());
            }
        }

        let (stop_tx, stop_rx) = oneshot::channel();
        let shared = managed.shared.clone();
        let spec = managed.spec.clone();
        let history = self.inner.restart_history.clone();
        let tuning = self.inner.tuning;
        let server_name = name.to_string();

        let driver = tokio::spawn(async move {
            run_lifecycle(server_name, spec, shared, history, tuning, stop_rx).await;
        });

        managed.driver = Some(driver);
        managed.stop_tx = Some(stop_tx);
        drop(servers);
        Ok(())
    }

    /// Stop `name`: signal the driver to exit, drop the connection.
    /// Idempotent: stopping a stopped server is a no-op.
    pub async fn stop(&self, name: &str) -> Result<()> {
        self.stop_with_terminal(
            name,
            ServerState::Failed {
                reason: "stopped".into(),
            },
        )
        .await
    }

    /// F-155: disable `name` — identical to [`McpManager::stop`] except the
    /// terminal state is [`ServerState::Disabled`]. The distinction is
    /// load-bearing: a `Disabled` server's `call()` returns the canonical
    /// `"MCP server <name> is disabled"` error string that the running-
    /// session toggle test asserts against, and `toggle_mcp_server(name,
    /// true)` knows to treat `Disabled` as restartable (same as `Failed`).
    ///
    /// Use [`McpManager::enable`] to restart a disabled server. The
    /// driver does **not** clear `restart_history` on disable, so repeated
    /// flap-toggles still honour the sliding-window budget.
    pub async fn disable(&self, name: &str) -> Result<()> {
        self.stop_with_terminal(
            name,
            ServerState::Disabled {
                reason: "server disabled".into(),
            },
        )
        .await
    }

    /// F-155: start `name` from a `Disabled`/`Failed` state. Thin wrapper
    /// over [`McpManager::start`] so callers that toggle a server back on
    /// can express intent explicitly. Behaves identically to `start` for
    /// every other initial state.
    pub async fn enable(&self, name: &str) -> Result<()> {
        self.start(name).await
    }

    async fn stop_with_terminal(&self, name: &str, terminal: ServerState) -> Result<()> {
        let mut servers = self.inner.servers.lock().await;
        let managed = servers
            .get_mut(name)
            .ok_or_else(|| anyhow!("unknown MCP server {name:?}"))?;

        if let Some(tx) = managed.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = managed.driver.take() {
            handle.abort();
        }
        // Drop the live connection explicitly so the pump task aborts.
        let mut conn_guard = managed.shared.conn.lock().await;
        *conn_guard = None;
        drop(conn_guard);
        managed.shared.publish(terminal).await;
        Ok(())
    }

    /// Invoke a tool on `name`. Blocks until the server responds or the
    /// per-request timeout fires. Surfaces the server's JSON-RPC error
    /// object as an `Err` when present.
    pub async fn call(
        &self,
        name: &str,
        tool: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let conn = self.connection(name).await?;
        let params = serde_json::json!({ "name": tool, "arguments": args });
        let value = call_request(&conn, "tools/call", params).await?;
        Ok(value)
    }

    /// Acquire a snapshot of the current connection for `name`.
    /// Returns an error when the server is not `Healthy`.
    ///
    /// F-155: when the server is in [`ServerState::Disabled`] the error
    /// surfaces the canonical `"MCP server <name> is disabled"` string so
    /// the session-side tool dispatch layer and the integration test in
    /// `forge-shell/tests/ipc_mcp.rs` can both match it.
    async fn connection(&self, name: &str) -> Result<Arc<Connection>> {
        let servers = self.inner.servers.lock().await;
        let managed = servers
            .get(name)
            .ok_or_else(|| anyhow!("unknown MCP server {name:?}"))?;
        let state = managed.shared.state.lock().await.clone();
        if matches!(state, ServerState::Disabled { .. }) {
            return Err(anyhow!("MCP server {name} is disabled"));
        }
        let guard = managed.shared.conn.lock().await;
        guard
            .clone()
            .ok_or_else(|| anyhow!("MCP server {name:?} is not connected"))
    }
}

/// Drive one server through its complete lifecycle.
///
/// Loop: connect → handshake → run health checks until failure or stop
/// → wait backoff → retry until the restart window caps us. Terminal
/// state is always `Failed { reason }`; callers restart via a fresh
/// `start()`.
async fn run_lifecycle(
    name: String,
    spec: McpServerSpec,
    shared: Arc<ServerShared>,
    history: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    tuning: LifecycleTuning,
    mut stop_rx: oneshot::Receiver<()>,
) {
    let mut attempt: usize = 0;

    loop {
        // Restart-history bookkeeping. We only consult this on retries
        // (attempt > 0) — the very first attempt is not a "restart".
        if attempt > 0 {
            let admit = register_restart(&history, &name).await;
            if !admit {
                shared
                    .publish(ServerState::Failed {
                        reason: format!(
                            "restart cap reached: {MAX_RESTARTS_PER_WINDOW} attempts \
                             in {window_s}s window",
                            window_s = RESTART_WINDOW.as_secs()
                        ),
                    })
                    .await;
                return;
            }

            let backoff = RESTART_BACKOFF_LADDER
                .get(attempt - 1)
                .copied()
                .or_else(|| RESTART_BACKOFF_LADDER.last().copied())
                .unwrap_or(Duration::from_secs(1));

            tokio::select! {
                _ = tokio::time::sleep(backoff) => {}
                _ = &mut stop_rx => {
                    shared.publish(ServerState::Failed { reason: "stopped".into() }).await;
                    return;
                }
            }
        }

        shared.publish(ServerState::Starting).await;

        // One full session: connect, handshake, run until it dies or
        // we're told to stop. `run_session` returns `Ok(())` on a
        // clean stop (exit the outer loop) or `Err(reason)` on a
        // failure that should trigger a restart.
        match run_session(&spec, &shared, &tuning, &mut stop_rx).await {
            Ok(()) => {
                // Clean stop — already published `Failed { stopped }` inside.
                return;
            }
            Err(reason) => {
                shared
                    .publish(ServerState::Degraded {
                        reason: reason.clone(),
                    })
                    .await;
                attempt += 1;
            }
        }
    }
}

/// Open one transport session and pump events until it fails or the
/// stop signal fires. On failure returns `Err(reason)`; on graceful
/// stop returns `Ok(())` and publishes the terminal state itself.
async fn run_session(
    spec: &McpServerSpec,
    shared: &Arc<ServerShared>,
    tuning: &LifecycleTuning,
    stop_rx: &mut oneshot::Receiver<()>,
) -> std::result::Result<(), String> {
    let (conn, mut pump_exit) = match connect(spec, &shared.name).await {
        Ok(pair) => {
            let (c, exit) = pair;
            (Arc::new(c), exit)
        }
        Err(err) => return Err(format!("connect failed: {err:#}")),
    };

    // Handshake. MCP servers require `initialize` before `tools/list`.
    let init_params = serde_json::json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": { "name": "forge-mcp", "version": env!("CARGO_PKG_VERSION") }
    });
    if let Err(err) = call_request(&conn, "initialize", init_params).await {
        return Err(format!("initialize failed: {err:#}"));
    }

    // Publish the connection and pull the first tools list. If the
    // first tools/list fails the server never went Healthy — treat it
    // the same as a later health-check failure so the restart policy
    // kicks in.
    {
        let mut guard = shared.conn.lock().await;
        *guard = Some(conn.clone());
    }
    if let Err(err) = refresh_tools(&conn, shared).await {
        let mut guard = shared.conn.lock().await;
        *guard = None;
        return Err(format!("initial tools/list failed: {err:#}"));
    }
    shared.publish(ServerState::Healthy).await;

    // Health-check loop. One tick per `tuning.health_check_interval`
    // (30s by default, shortened in tests); every failed ping pops us
    // out and triggers a restart. Also select on the pump-exit signal
    // so a crashed server triggers a restart immediately instead of
    // waiting up to the full interval for the next ping.
    let mut ticker = tokio::time::interval(tuning.health_check_interval);
    // Skip the first "immediate" tick — we just confirmed health via
    // the handshake.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if let Err(err) = refresh_tools(&conn, shared).await {
                    {
                        let mut guard = shared.conn.lock().await;
                        *guard = None;
                    }
                    return Err(format!("health check failed: {err:#}"));
                }
            }
            reason = &mut pump_exit => {
                {
                    let mut guard = shared.conn.lock().await;
                    *guard = None;
                }
                let reason = reason
                    .unwrap_or_else(|_| "pump channel closed unexpectedly".into());
                return Err(format!("transport exited: {reason}"));
            }
            _ = &mut *stop_rx => {
                {
                    let mut guard = shared.conn.lock().await;
                    *guard = None;
                }
                shared.publish(ServerState::Failed { reason: "stopped".into() }).await;
                return Ok(());
            }
        }
    }
}

/// Connect and spawn the pump. Returns a ready-to-use [`Connection`]
/// plus a [`oneshot::Receiver`] that fires when the pump task exits
/// (for any reason). The lifecycle driver selects on that receiver so
/// a crashed server surfaces as a health failure immediately, not after
/// the next health-check tick.
async fn connect(
    spec: &McpServerSpec,
    server_name: &str,
) -> Result<(Connection, oneshot::Receiver<String>)> {
    let transport = match &spec.kind {
        ServerKind::Stdio { .. } => TransportHalf::Stdio(Stdio::connect(spec).await?),
        ServerKind::Http { .. } => TransportHalf::Http(Http::connect(spec).await?),
    };
    spawn_pump(transport, server_name)
}

/// Install the outbound command channel + pump task around a ready
/// [`TransportHalf`]. Split out so tests can drive the pump against
/// an in-memory transport (`TransportHalf::InProc`) without touching
/// the real subprocess or HTTP paths.
fn spawn_pump(
    transport: TransportHalf,
    server_name: &str,
) -> Result<(Connection, oneshot::Receiver<String>)> {
    // Outbound command channel. Depth matches the transport event
    // channels so a burst of calls doesn't back-pressure the caller.
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(128);
    let (exit_tx, exit_rx) = oneshot::channel::<String>();

    let name = server_name.to_string();
    let pump = tokio::spawn(async move {
        let reason = run_pump(transport, cmd_rx, name).await;
        let _ = exit_tx.send(reason);
    });

    let conn = Connection {
        next_id: AtomicU64::new(1),
        cmd_tx,
        pump: Mutex::new(Some(pump)),
    };
    Ok((conn, exit_rx))
}

/// Pump task: sole owner of the transport. `select!`s on
///   (a) inbound transport events — route responses by id into pending
///       slots, discard notifications,
///   (b) outbound command channel — serialise frames onto the transport
///       and register response slots.
///
/// Exits when either the transport closes (returns an Err to the current
/// pending and drops the routing table) or the command channel is
/// closed (the `Connection` was dropped — `Drop` aborts us anyway).
async fn run_pump(
    mut transport: TransportHalf,
    mut cmd_rx: mpsc::Receiver<Command>,
    server_name: String,
) -> String {
    let mut pending: HashMap<u64, oneshot::Sender<serde_json::Value>> = HashMap::new();

    loop {
        tokio::select! {
            ev = transport.recv() => {
                match ev {
                    TransportEvent::Message(v) => {
                        if let Some(id) = extract_id(&v) {
                            if let Some(slot) = pending.remove(&id) {
                                let _ = slot.send(v);
                            } else {
                                tracing::debug!(
                                    target: "forge_mcp::manager",
                                    server = %server_name,
                                    id,
                                    "response with no matching pending request",
                                );
                            }
                        } else {
                            tracing::debug!(
                                target: "forge_mcp::manager",
                                server = %server_name,
                                method = ?v.get("method"),
                                "unhandled notification",
                            );
                        }
                    }
                    TransportEvent::Closed(reason) => {
                        tracing::debug!(
                            target: "forge_mcp::manager",
                            server = %server_name,
                            %reason,
                            "pump exiting: transport closed",
                        );
                        // Dropping `pending` closes every remaining
                        // oneshot; waiters observe `RecvError` and
                        // surface a "response channel closed" error.
                        return reason;
                    }
                }
            }
            maybe_cmd = cmd_rx.recv() => {
                match maybe_cmd {
                    Some(Command::Send { frame, respond_to }) => {
                        if let Some((id, slot)) = respond_to {
                            pending.insert(id, slot);
                        }
                        if let Err(err) = transport.send(frame).await {
                            tracing::debug!(
                                target: "forge_mcp::manager",
                                server = %server_name,
                                error = %err,
                                "transport send failed",
                            );
                            // Transport's dead; we'll exit on the next
                            // `recv()` iteration when it yields Closed.
                        }
                    }
                    None => {
                        tracing::debug!(
                            target: "forge_mcp::manager",
                            server = %server_name,
                            "pump exiting: command channel closed",
                        );
                        return "command channel closed".to_string();
                    }
                }
            }
        }
    }
}

/// Parse the integer id off a JSON-RPC response. MCP servers may
/// respond with either a JSON number or a string id depending on what
/// the client sent; we always send numeric ids so we only accept those
/// back.
fn extract_id(v: &serde_json::Value) -> Option<u64> {
    v.get("id").and_then(|id| id.as_u64())
}

/// Send one JSON-RPC request and await its response. Enqueues a
/// [`Command::Send`] (with response slot) onto the pump's command
/// channel; the pump is responsible for writing the frame onto the
/// transport and routing the response back via the oneshot.
async fn call_request(
    conn: &Arc<Connection>,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value> {
    let id = conn.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();

    let frame = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });

    conn.cmd_tx
        .send(Command::Send {
            frame,
            respond_to: Some((id, tx)),
        })
        .await
        .with_context(|| format!("pump channel closed while sending {method}"))?;

    let response = tokio::time::timeout(REQUEST_TIMEOUT, rx)
        .await
        .with_context(|| format!("{method} timed out after {:?}", REQUEST_TIMEOUT))?;

    let value = response.map_err(|_| anyhow!("{method}: response channel closed"))?;

    if let Some(err) = value.get("error") {
        return Err(anyhow!("JSON-RPC error on {method}: {err}"));
    }
    Ok(value
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

/// Run `tools/list` and cache the result in the shared state. On any
/// failure (timeout, JSON-RPC error, schema error) bubble up — the
/// lifecycle driver treats that as a health-check failure.
async fn refresh_tools(conn: &Arc<Connection>, shared: &Arc<ServerShared>) -> Result<()> {
    let result = call_request(conn, "tools/list", serde_json::json!({})).await?;
    let tools = parse_tools_list(&shared.name, &result)?;
    *shared.tools.lock().await = tools;
    Ok(())
}

/// Translate an MCP `tools/list` result into a vector of unified
/// [`Tool`] values.
///
/// Each tool name is namespaced with the server name (`"<server>.<tool>"`)
/// so sessions can dispatch across multiple servers without worrying
/// about collisions. The `read_only` classification comes from the MCP
/// `annotations.readOnlyHint` field when present; absent hints default
/// to `false` (mutating) per the safer-by-default contract in the Tool
/// docs.
fn parse_tools_list(server: &str, result: &serde_json::Value) -> Result<Vec<Tool>> {
    let arr = result
        .get("tools")
        .and_then(|v| v.as_array())
        .ok_or_else(|| anyhow!("tools/list result missing `tools` array"))?;

    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let name = entry
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("tool in tools/list missing `name`"))?;
        let description = entry
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let input_schema = entry
            .get("inputSchema")
            .cloned()
            .unwrap_or(serde_json::json!({}));
        let read_only = entry
            .get("annotations")
            .and_then(|a| a.get("readOnlyHint"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        out.push(Tool {
            name: format!("{server}.{name}"),
            description,
            input_schema,
            read_only,
        });
    }
    Ok(out)
}

/// Sliding-window restart accounting. Returns `true` if the attempt
/// fits the window (record it and proceed); `false` if the cap is
/// reached (caller must park the server in `Failed`).
async fn register_restart(
    history: &Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
    name: &str,
) -> bool {
    let now = Instant::now();
    let cutoff = now.checked_sub(RESTART_WINDOW).unwrap_or(now);
    let mut all = history.lock().await;
    let dq = all.entry(name.to_string()).or_default();
    while dq.front().map(|t| *t < cutoff).unwrap_or(false) {
        dq.pop_front();
    }
    if dq.len() >= MAX_RESTARTS_PER_WINDOW {
        return false;
    }
    dq.push_back(now);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tools_list_namespaces_names_and_reads_read_only_hint() {
        let result = serde_json::json!({
            "tools": [
                {
                    "name": "list_files",
                    "description": "List files under a path.",
                    "inputSchema": { "type": "object" },
                    "annotations": { "readOnlyHint": true }
                },
                {
                    "name": "write_file",
                    "description": "Mutate a file.",
                    "inputSchema": { "type": "object" },
                    "annotations": { "readOnlyHint": false }
                },
                {
                    "name": "no_hints",
                    "description": "",
                    "inputSchema": {}
                }
            ]
        });

        let tools = parse_tools_list("fs", &result).expect("parse");
        assert_eq!(tools.len(), 3);
        assert_eq!(tools[0].name, "fs.list_files");
        assert!(tools[0].read_only);
        assert_eq!(tools[1].name, "fs.write_file");
        assert!(!tools[1].read_only);
        assert_eq!(tools[2].name, "fs.no_hints");
        // Default is mutating when annotations are missing.
        assert!(!tools[2].read_only);
    }

    #[test]
    fn parse_tools_list_rejects_non_array() {
        let result = serde_json::json!({ "tools": "nope" });
        assert!(parse_tools_list("x", &result).is_err());
    }

    #[test]
    fn parse_tools_list_rejects_missing_tool_name() {
        let result = serde_json::json!({ "tools": [ { "description": "x" } ] });
        assert!(parse_tools_list("x", &result).is_err());
    }

    #[test]
    fn backoff_ladder_matches_dod() {
        // DoD: 1s, 2s, 4s, 8s, 16s, capped at 5 attempts per 10min window.
        assert_eq!(
            RESTART_BACKOFF_LADDER.len(),
            MAX_RESTARTS_PER_WINDOW,
            "ladder must have exactly MAX_RESTARTS_PER_WINDOW rungs"
        );
        assert_eq!(RESTART_BACKOFF_LADDER[0], Duration::from_secs(1));
        assert_eq!(RESTART_BACKOFF_LADDER[1], Duration::from_secs(2));
        assert_eq!(RESTART_BACKOFF_LADDER[2], Duration::from_secs(4));
        assert_eq!(RESTART_BACKOFF_LADDER[3], Duration::from_secs(8));
        assert_eq!(RESTART_BACKOFF_LADDER[4], Duration::from_secs(16));
        assert_eq!(RESTART_WINDOW, Duration::from_secs(600));
    }

    #[test]
    fn restart_window_lookup_caps_at_last_rung() {
        // Scripted ladder lookup matches what `run_lifecycle` does on
        // each restart attempt. `attempt - 1` indexes the ladder;
        // attempts beyond the ladder length are admitted only when
        // `register_restart` admits them, but *if* admitted, the
        // backoff saturates at the final rung.
        let ladder_for = |attempt: usize| {
            RESTART_BACKOFF_LADDER
                .get(attempt - 1)
                .copied()
                .or_else(|| RESTART_BACKOFF_LADDER.last().copied())
                .unwrap_or(Duration::from_secs(1))
        };
        assert_eq!(ladder_for(1), Duration::from_secs(1));
        assert_eq!(ladder_for(5), Duration::from_secs(16));
        // Saturation guard — if the admission ever lets through a 6th
        // attempt, we still get a bounded backoff rather than a panic.
        assert_eq!(ladder_for(6), Duration::from_secs(16));
        assert_eq!(ladder_for(100), Duration::from_secs(16));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_restart_caps_at_five_per_window() {
        let h = Arc::new(Mutex::new(HashMap::new()));
        for i in 0..MAX_RESTARTS_PER_WINDOW {
            assert!(
                register_restart(&h, "srv").await,
                "attempt {i} should be admitted"
            );
        }
        // 6th must be refused.
        assert!(
            !register_restart(&h, "srv").await,
            "window cap did not refuse the {}th attempt",
            MAX_RESTARTS_PER_WINDOW + 1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_restart_is_per_server() {
        // A different server name gets its own budget — a noisy
        // server shouldn't block restarts on a healthy one.
        let h = Arc::new(Mutex::new(HashMap::new()));
        for _ in 0..MAX_RESTARTS_PER_WINDOW {
            assert!(register_restart(&h, "noisy").await);
        }
        assert!(
            !register_restart(&h, "noisy").await,
            "noisy: cap must be per-server"
        );
        assert!(
            register_restart(&h, "quiet").await,
            "quiet: unrelated server should still be admitted"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn register_restart_drops_entries_older_than_window() {
        let h: Arc<Mutex<HashMap<String, VecDeque<Instant>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        // Seed the deque with an entry that is definitely outside the
        // window, then confirm the next registration succeeds.
        {
            let mut g = h.lock().await;
            let dq = g.entry("srv".to_string()).or_default();
            dq.push_back(
                Instant::now()
                    .checked_sub(RESTART_WINDOW + Duration::from_secs(1))
                    .expect("instant math"),
            );
        }
        assert!(
            register_restart(&h, "srv").await,
            "stale entry should be pruned"
        );
        let g = h.lock().await;
        // The stale entry is gone, the new one is present.
        assert_eq!(g["srv"].len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn list_reflects_configured_servers_even_before_start() {
        let mut cfg = BTreeMap::new();
        cfg.insert(
            "a".to_string(),
            McpServerSpec {
                kind: ServerKind::Stdio {
                    command: "/bin/true".into(),
                    args: Vec::new(),
                    env: Default::default(),
                },
            },
        );
        cfg.insert(
            "b".to_string(),
            McpServerSpec {
                kind: ServerKind::Http {
                    url: "https://example.com".into(),
                    headers: Default::default(),
                },
            },
        );

        let mgr = McpManager::new(cfg);
        let listed = mgr.list().await;
        assert_eq!(listed.len(), 2);
        let names: Vec<&str> = listed.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
        // Pre-start state is Starting — nothing has run yet.
        for s in &listed {
            assert!(matches!(s.state, ServerState::Starting));
            assert!(s.tools.is_empty());
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_on_unknown_server_errors() {
        let mgr = McpManager::new(BTreeMap::new());
        let err = mgr.stop("nope").await.unwrap_err();
        assert!(format!("{err:#}").contains("unknown"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn call_on_unstarted_server_errors() {
        let mut cfg = BTreeMap::new();
        cfg.insert(
            "srv".to_string(),
            McpServerSpec {
                kind: ServerKind::Stdio {
                    command: "/bin/true".into(),
                    args: Vec::new(),
                    env: Default::default(),
                },
            },
        );
        let mgr = McpManager::new(cfg);
        let err = mgr.call("srv", "any", serde_json::json!({})).await;
        let err = err.expect_err("must fail before start");
        assert!(format!("{err:#}").contains("not connected"));
    }
}
