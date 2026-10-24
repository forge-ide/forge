//! The per-session egress proxy: every connection a sandboxed process makes to the outside
//! goes through it, and it lets through only hosts on the session's allowlist.
//!
//! It speaks HTTP `CONNECT` (for TLS and anything else tunnelled) and absolute-form plain HTTP.
//! It matches the requested host name against the allowlist (exact names, ports 80 and 443),
//! resolves the name itself, and refuses names that resolve to anything but public addresses,
//! so an allowlisted name can't be pointed at the user's own machine or network. IP literals
//! are never allowed. Every connection is reported once through the log callback: when it
//! closes, or at once when it is denied.
//!
//! It listens on a loopback TCP port (Light, Seatbelt, Unsandboxed) and, when a backend asks,
//! on a Unix socket that bubblewrap binds into a Standard sandbox.
//!
//! It also serves one route per injected secret (see [`crate::secrets`]): a plain-HTTP request
//! to `/_forge/<token>/<route>/<path>` is sent over TLS to that secret's fixed host as
//! `/<path>`, with the secret's header set to the real value. The agent only ever holds a
//! placeholder. `<token>` is random per proxy and reaches the sandbox only in the base-URL
//! variables, so other local programs (or a web page rebinding a name to loopback) can't use
//! the route; requests must also name one loopback `Host`, carry no browser `Origin` or Fetch
//! Metadata headers (but `Sec-Fetch-Mode` alone, which Node's `fetch` always sends), use `GET`
//! or `POST`, and stay on the secret's allowed paths.
//!
//! And it serves the daemon's own endpoint for the session, when it has one: a `POST` to
//! `/_forge/<token>/mcp`, with the same caller checks and a JSON body, is answered by a
//! [`LocalService`]
//! (Forge's MCP server) instead of going anywhere.

use std::collections::HashSet;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UnixListener};
use tokio::sync::Semaphore;
use tokio::task::{JoinHandle, JoinSet};

use crate::secrets::{KnownSecret, PLACEHOLDER, Secret, SecretSource};

/// Longest request head the proxy reads before giving up.
const MAX_HEAD: usize = 16 * 1024;
/// How long a client has to send its request head.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
/// How long to wait for the upstream connection.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest host name and reason kept in a logged connection.
const MAX_LOGGED_HOST: usize = 253;
const MAX_LOGGED_REASON: usize = 512;
/// Ports an allowlisted host may be reached on.
const PORTS: [u16; 2] = [80, 443];

/// One connection through the proxy, as the log callback sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connection {
    pub host: String,
    pub port: u16,
    pub allowed: bool,
    /// Why it was denied.
    pub reason: Option<String>,
    /// Bytes from the sandbox to the host.
    pub bytes_sent: u64,
    /// Bytes from the host to the sandbox.
    pub bytes_received: u64,
    pub duration: Duration,
}

/// Called once per connection.
pub type ConnectionLog = Arc<dyn Fn(Connection) + Send + Sync>;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Turns an allowlisted host into the addresses to connect to.
pub trait Resolve: Send + Sync + 'static {
    /// The addresses for `host:port`, or why it can't be reached.
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, String>>;
}

/// System DNS, refusing a name if any of its addresses isn't public.
#[derive(Clone, Copy, Debug, Default)]
pub struct PublicDns;

impl Resolve for PublicDns {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<SocketAddr>, String>> {
        Box::pin(async move {
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
                .await
                .map_err(|e| format!("{host} does not resolve: {e}"))?
                .collect();
            if addrs.is_empty() {
                return Err(format!("{host} does not resolve"));
            }
            if let Some(private) = addrs.iter().find(|a| !is_public(a.ip())) {
                return Err(format!(
                    "{host} resolves to {}, which is not a public address",
                    private.ip()
                ));
            }
            Ok(addrs)
        })
    }
}

/// Whether `ip` is a public internet address: not loopback, private, link-local (which holds
/// cloud metadata services), shared, documentation, multicast, or otherwise reserved.
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_unspecified()
                || v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..128).contains(&b)) // shared address space
                || (a == 192 && b == 0) // IETF protocol assignments
                || (a == 198 && (18..20).contains(&b)) // benchmarking
                || a >= 240) // reserved
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            // 6to4 carries an IPv4 address that a relay may route to.
            if seg[0] == 0x2002 {
                let [a, b] = seg[1].to_be_bytes();
                let [c, d] = seg[2].to_be_bytes();
                return is_public(IpAddr::V4(std::net::Ipv4Addr::new(a, b, c, d)));
            }
            !(v6.is_unspecified()
                || v6.is_loopback()
                || v6.is_multicast()
                || seg[..6] == [0; 6] // IPv4-compatible, deprecated
                || (seg[0] & 0xfe00) == 0xfc00 // unique local
                || (seg[0] & 0xffc0) == 0xfe80 // link-local
                || (seg[0] & 0xffc0) == 0xfec0 // site-local, deprecated
                || seg[0] == 0x2001 && seg[1] == 0x0db8 // documentation
                || seg[0] == 0x0064 && seg[1] == 0xff9b) // NAT64, well-known and local-use
        }
    }
}

/// Hosts a session may reach.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Allowlist {
    hosts: HashSet<String>,
}

impl Allowlist {
    /// From host names such as `crates.io`. Names are matched exactly, ignoring case and a
    /// trailing dot; entries with a scheme, port, path, wildcard, or IP address are refused.
    pub fn new<I: IntoIterator<Item = S>, S: AsRef<str>>(hosts: I) -> Result<Self, String> {
        let mut set = HashSet::new();
        for host in hosts {
            let host = host.as_ref();
            set.insert(normalize(host).ok_or_else(|| {
                format!(
                    "{host:?} is not a host name (no scheme, port, path, wildcard, or IP address)"
                )
            })?);
        }
        Ok(Self { hosts: set })
    }

    /// Why `host:port` may not be reached, if it may not.
    fn check(&self, host: &str, port: u16) -> Result<(), String> {
        if host.parse::<IpAddr>().is_ok() || host.starts_with('[') {
            return Err(format!(
                "{host} is an IP address; only allowlisted host names may be reached"
            ));
        }
        let name = normalize(host).ok_or_else(|| format!("{host:?} is not a host name"))?;
        if !self.hosts.contains(&name) {
            return Err(format!("{name} is not on this session's network allowlist"));
        }
        if !PORTS.contains(&port) {
            return Err(format!(
                "{name} may only be reached on ports 80 and 443, not {port}"
            ));
        }
        Ok(())
    }
}

/// A lowercase host name without a trailing dot, if `host` is one.
fn normalize(host: &str) -> Option<String> {
    let host = host.strip_suffix('.').unwrap_or(host).to_ascii_lowercase();
    let label_ok = |l: &str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    let valid = !host.is_empty()
        && host.len() <= 253
        && host.split('.').all(label_ok)
        && host.parse::<IpAddr>().is_err()
        // An all-numeric last label would make it look like an address.
        && !host.rsplit('.').next().is_some_and(|l| l.chars().all(|c| c.is_ascii_digit()));
    valid.then_some(host)
}

/// What a proxy serves.
#[derive(Clone)]
pub struct ProxyConfig {
    pub allow: Allowlist,
    pub resolver: Arc<dyn Resolve>,
    pub log: ConnectionLog,
    pub limits: Limits,
    /// Secrets to inject, each on its own route.
    pub routes: Vec<SecretRoute>,
    /// Secures the connection to a route's host; required when there are routes.
    pub upstream: Option<Arc<dyn Upstream>>,
    /// Answers requests on the local route.
    pub local: Option<Arc<dyn LocalService>>,
}

/// The local route's name: `/_forge/<token>/mcp`.
pub const LOCAL_ROUTE: &str = "mcp";

/// Largest request body the local route accepts.
const MAX_LOCAL_BODY: usize = 1024 * 1024;

/// A request on the local route.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalRequest {
    /// Request headers, names lowercased.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// The answer to a [`LocalRequest`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalResponse {
    pub status: u16,
    pub content_type: &'static str,
    pub body: Vec<u8>,
}

/// Serves the local route. A call may wait on a human (an approval), so it isn't timed out.
pub trait LocalService: Send + Sync + 'static {
    fn call(
        &self,
        request: LocalRequest,
    ) -> Pin<Box<dyn Future<Output = LocalResponse> + Send + '_>>;
}

/// A secret and where its value comes from, served on the secret's route.
#[derive(Clone)]
pub struct SecretRoute {
    pub secret: &'static KnownSecret,
    pub value: Arc<dyn SecretSource>,
}

impl SecretRoute {
    /// A route whose value never changes, such as an API key from the keyring.
    pub fn fixed(secret: &'static KnownSecret, value: Secret) -> Self {
        Self {
            secret,
            value: Arc::new(value),
        }
    }
}

impl std::fmt::Debug for SecretRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretRoute")
            .field("secret", &self.secret.name)
            .finish_non_exhaustive()
    }
}

/// A connection to a route's host, ready for the rewritten request.
pub trait UpstreamStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> UpstreamStream for T {}

/// Turns a TCP connection to a route's host into the stream the request goes over.
pub trait Upstream: Send + Sync + 'static {
    fn wrap<'a>(
        &'a self,
        host: &'a str,
        tcp: TcpStream,
    ) -> BoxFuture<'a, std::io::Result<Box<dyn UpstreamStream>>>;
}

/// TLS with the OS's trusted roots, verifying the route's host name.
pub struct Tls {
    connector: tokio_rustls::TlsConnector,
}

impl Tls {
    /// The shared client, loading the OS's trusted roots on first use; a failure is retried
    /// on the next call. Blocking.
    pub fn shared() -> Result<Arc<Tls>, String> {
        static TLS: std::sync::OnceLock<Arc<Tls>> = std::sync::OnceLock::new();
        if let Some(tls) = TLS.get() {
            return Ok(tls.clone());
        }
        let tls = (|| {
            let mut roots = tokio_rustls::rustls::RootCertStore::empty();
            let found = rustls_native_certs::load_native_certs();
            for cert in found.certs {
                let _ = roots.add(cert);
            }
            if roots.is_empty() {
                return Err(format!(
                    "no trusted root certificates found on this machine ({} errors)",
                    found.errors.len()
                ));
            }
            let config = tokio_rustls::rustls::ClientConfig::builder_with_provider(Arc::new(
                tokio_rustls::rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .with_root_certificates(roots)
            .with_no_client_auth();
            Ok(Arc::new(Tls {
                connector: tokio_rustls::TlsConnector::from(Arc::new(config)),
            }))
        })()?;
        Ok(TLS.get_or_init(|| tls).clone())
    }
}

impl Upstream for Tls {
    fn wrap<'a>(
        &'a self,
        host: &'a str,
        tcp: TcpStream,
    ) -> BoxFuture<'a, std::io::Result<Box<dyn UpstreamStream>>> {
        Box::pin(async move {
            let name = tokio_rustls::rustls::pki_types::ServerName::try_from(host.to_owned())
                .map_err(std::io::Error::other)?;
            let stream = self.connector.connect(name, tcp).await?;
            Ok(Box::new(stream) as Box<dyn UpstreamStream>)
        })
    }
}

/// No TLS: the request goes over the TCP connection as is. Only for tests, against a local
/// stand-in for a route's host.
pub struct PlainUpstream;

impl Upstream for PlainUpstream {
    fn wrap<'a>(
        &'a self,
        _host: &'a str,
        tcp: TcpStream,
    ) -> BoxFuture<'a, std::io::Result<Box<dyn UpstreamStream>>> {
        Box::pin(async move { Ok(Box::new(tcp) as Box<dyn UpstreamStream>) })
    }
}

/// Bounds that keep a misbehaving sandbox from wearing down the daemon the proxy runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Connections served at once. Beyond this the proxy stops accepting, so further
    /// connections wait in the kernel's backlog instead of using the daemon's file descriptors.
    pub max_connections: usize,
    /// A tunnel with no traffic either way for this long is closed.
    pub idle: Duration,
    /// Denials logged per second; the rest are counted and reported with the next one logged.
    pub denials_per_second: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            idle: Duration::from_secs(300),
            denials_per_second: 20,
        }
    }
}

/// A running proxy. Dropping it stops it, closes every connection through it, and removes
/// its Unix socket.
pub struct Proxy {
    port: u16,
    state: Arc<Shared>,
    runtime: tokio::runtime::Handle,
    tasks: Mutex<Vec<JoinHandle<()>>>,
    sockets: Mutex<Vec<PathBuf>>,
}

struct Shared {
    config: ProxyConfig,
    slots: Arc<Semaphore>,
    denials: Mutex<Denials>,
    /// The secret path segment every route request must start with.
    token: String,
}

/// Denials logged in the current one-second window, and those dropped since the last logged.
struct Denials {
    window: Instant,
    logged: u32,
    dropped: u64,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Proxy")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl Proxy {
    /// Start listening on a loopback TCP port.
    pub async fn start(config: ProxyConfig) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        let token = random_token().await?;
        let state = Arc::new(Shared {
            token,
            slots: Arc::new(Semaphore::new(config.limits.max_connections)),
            config,
            denials: Mutex::new(Denials {
                window: Instant::now(),
                logged: 0,
                dropped: 0,
            }),
        });
        let accept = tokio::spawn(accept_loop(listener, state.clone()));
        Ok(Self {
            port,
            state,
            runtime: tokio::runtime::Handle::current(),
            tasks: Mutex::new(vec![accept]),
            sockets: Mutex::new(Vec::new()),
        })
    }

    /// The loopback TCP port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Also listen on a Unix socket at `path`, replacing a stale one. Callable from blocking
    /// code on the proxy's runtime.
    pub fn serve_unix(&self, path: &Path) -> std::io::Result<()> {
        // sun_path holds about 104 bytes on macOS and 108 on Linux.
        if path.as_os_str().len() > 100 {
            return Err(std::io::Error::other(format!(
                "{} is too long for a Unix socket; use a shorter data directory",
                path.display()
            )));
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let std_listener = std::os::unix::net::UnixListener::bind(path)?;
        std_listener.set_nonblocking(true)?;
        let _guard = self.runtime.enter();
        let listener = UnixListener::from_std(std_listener)?;
        let task = self
            .runtime
            .spawn(accept_loop(listener, self.state.clone()));
        lock(&self.tasks).push(task);
        lock(&self.sockets).push(path.to_path_buf());
        Ok(())
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        for task in lock(&self.tasks).drain(..) {
            task.abort();
        }
        for socket in lock(&self.sockets).drain(..) {
            let _ = std::fs::remove_file(socket);
        }
    }
}

/// 128 random bits, as hex.
async fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 16];
    tokio::fs::File::open("/dev/urandom")
        .await?
        .read_exact(&mut bytes)
        .await?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A listener the proxy accepts connections from.
trait Listen: Send + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;
    fn next_stream(&self) -> impl Future<Output = std::io::Result<Self::Stream>> + Send;
}

impl Listen for TcpListener {
    type Stream = TcpStream;
    async fn next_stream(&self) -> std::io::Result<TcpStream> {
        self.accept().await.map(|(s, _)| s)
    }
}

impl Listen for UnixListener {
    type Stream = tokio::net::UnixStream;
    async fn next_stream(&self) -> std::io::Result<Self::Stream> {
        self.accept().await.map(|(s, _)| s)
    }
}

/// Accept connections while a slot is free and serve each in a task of its own. The tasks
/// live in a set this loop owns, so aborting the loop closes every connection.
async fn accept_loop<L: Listen>(listener: L, state: Arc<Shared>) {
    let mut connections = JoinSet::new();
    loop {
        while connections.try_join_next().is_some() {}
        let Ok(slot) = state.slots.clone().acquire_owned().await else {
            return;
        };
        match listener.next_stream().await {
            Ok(stream) => {
                let state = state.clone();
                connections.spawn(async move {
                    let _slot = slot;
                    serve(stream, &state).await;
                });
            }
            Err(e) => {
                tracing::warn!(error = %e, "egress proxy accept");
                drop(slot);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// Report a denial, unless the limit for this second was already reported.
fn log_denial(state: &Shared, mut connection: Connection) {
    let dropped = {
        let mut d = lock(&state.denials);
        if d.window.elapsed() >= Duration::from_secs(1) {
            d.window = Instant::now();
            d.logged = 0;
        }
        if d.logged >= state.config.limits.denials_per_second {
            d.dropped += 1;
            return;
        }
        d.logged += 1;
        std::mem::take(&mut d.dropped)
    };
    if dropped > 0 {
        let reason = connection.reason.unwrap_or_default();
        connection.reason = Some(format!(
            "{reason} ({dropped} earlier denials were not logged)"
        ));
    }
    (state.config.log)(clipped(connection));
}

/// `connection` with its host and reason cut to loggable lengths.
fn clipped(mut connection: Connection) -> Connection {
    connection.host = clip(connection.host, MAX_LOGGED_HOST);
    connection.reason = connection.reason.map(|r| clip(r, MAX_LOGGED_REASON));
    connection
}

fn clip(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut cut = max;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push('…');
    }
    s
}

/// A parsed proxy request.
enum Request {
    /// `CONNECT host:port`.
    Connect { host: String, port: u16 },
    /// Absolute-form plain HTTP; `head` is the request rewritten for the origin server.
    Http {
        host: String,
        port: u16,
        head: Vec<u8>,
    },
    /// A secret's route; `head` is the request rewritten for the secret's host, still
    /// without the secret's header, which [`finish_route_head`] adds.
    Route {
        host: String,
        route: SecretRoute,
        head: Vec<u8>,
    },
    /// `/_forge/...` without this proxy's token, or for a route this session doesn't have.
    UnknownRoute,
    /// A request on a secret's route that may not carry it.
    RouteRefused { host: String, reason: String },
    /// A `POST` on the local route with a body of `length` bytes.
    Local {
        headers: Vec<(String, String)>,
        length: usize,
    },
    /// A request on the local route that may not be served.
    LocalRefused { code: u16, reason: String },
}

async fn serve<S>(mut client: S, state: &Shared)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = &state.config;
    let started = Instant::now();
    let (request, rest) = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut client)).await {
        Ok(Ok((head, rest))) => {
            match parse(&head, &config.routes, &state.token, config.local.is_some()) {
                Ok(request) => (request, rest),
                Err(why) => {
                    // Nothing to log against a host: the client didn't name one.
                    let _ = respond(&mut client, 400, &why).await;
                    return;
                }
            }
        }
        _ => return,
    };
    let (host, port) = match &request {
        Request::Connect { host, port } | Request::Http { host, port, .. } => (host.clone(), *port),
        Request::Route { host, .. } | Request::RouteRefused { host, .. } => (host.clone(), 443),
        Request::UnknownRoute => {
            let _ = respond(&mut client, 404, "no such route").await;
            return;
        }
        Request::LocalRefused { code, reason } => {
            let _ = respond(&mut client, *code, reason).await;
            return;
        }
        Request::Local { headers, length } => {
            if let Some(service) = &config.local {
                serve_local(client, service.as_ref(), headers.clone(), *length, rest).await;
            }
            return;
        }
    };
    let deny = |reason: String| Connection {
        host: host.clone(),
        port,
        allowed: false,
        reason: Some(reason),
        bytes_sent: 0,
        bytes_received: 0,
        duration: started.elapsed(),
    };
    if let Request::RouteRefused { reason, .. } = &request {
        let _ = respond(&mut client, 403, reason).await;
        log_denial(state, deny(reason.clone()));
        return;
    }
    // The secret's value, read now: a login's changes as its CLI refreshes it.
    let request = match request {
        Request::Route { host, route, head } => {
            let source = route.value.clone();
            let value = tokio::task::spawn_blocking(move || source.current())
                .await
                .unwrap_or_else(|e| Err(format!("reading the secret: {e}")));
            match value.and_then(|value| finish_route_head(head, &route, &value)) {
                Ok(head) => Request::Route { host, route, head },
                Err(why) => {
                    let _ = respond(&mut client, 401, &why).await;
                    log_denial(state, deny(why));
                    return;
                }
            }
        }
        other => other,
    };
    // A route's host is fixed by the secret, not by the allowlist.
    let routed = matches!(request, Request::Route { .. });
    if !routed && let Err(why) = config.allow.check(&host, port) {
        let _ = respond(&mut client, 403, &why).await;
        log_denial(state, deny(why));
        return;
    }
    let upstream = match connect(config, &host, port).await {
        Ok(tcp) if routed => match &config.upstream {
            Some(upstream) => upstream
                .wrap(&host, tcp)
                .await
                .map_err(|e| format!("securing the connection to {host}: {e}")),
            None => Err(format!("no TLS client to reach {host} with")),
        },
        Ok(tcp) => Ok(Box::new(tcp) as Box<dyn UpstreamStream>),
        Err(why) => Err(why),
    };
    let mut upstream = match upstream {
        Ok(stream) => stream,
        Err(why) => {
            let _ = respond(&mut client, 502, &why).await;
            log_denial(state, deny(why));
            return;
        }
    };
    let mut sent = rest.len() as u64;
    match &request {
        Request::Connect { .. } => {
            if client
                .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
                .await
                .is_err()
            {
                return;
            }
        }
        Request::Http { head, .. } | Request::Route { head, .. } => {
            sent += head.len() as u64;
            if upstream.write_all(head).await.is_err() {
                return;
            }
        }
        Request::UnknownRoute
        | Request::RouteRefused { .. }
        | Request::Local { .. }
        | Request::LocalRefused { .. } => unreachable!("answered above"),
    }
    if !rest.is_empty() && upstream.write_all(&rest).await.is_err() {
        return;
    }
    let (up, down) = pump(client, upstream, config.limits.idle).await;
    (config.log)(clipped(Connection {
        host,
        port,
        allowed: true,
        reason: None,
        bytes_sent: sent + up,
        bytes_received: down,
        duration: started.elapsed(),
    }));
}

/// Copy both ways until both sides finish or nothing moves for `idle`; the bytes copied from
/// `a` to `b` and from `b` to `a`.
async fn pump<A, B>(a: A, b: B, idle: Duration) -> (u64, u64)
where
    A: AsyncRead + AsyncWrite + Unpin,
    B: AsyncRead + AsyncWrite + Unpin,
{
    let (mut a_read, mut a_write) = tokio::io::split(a);
    let (mut b_read, mut b_write) = tokio::io::split(b);
    let (mut a_buf, mut b_buf) = (vec![0u8; 16 * 1024], vec![0u8; 16 * 1024]);
    let (mut a_open, mut b_open) = (true, true);
    let (mut a_to_b, mut b_to_a) = (0u64, 0u64);
    while a_open || b_open {
        tokio::select! {
            read = a_read.read(&mut a_buf), if a_open => match read {
                Ok(0) | Err(_) => {
                    a_open = false;
                    let _ = b_write.shutdown().await;
                }
                Ok(n) => {
                    if b_write.write_all(&a_buf[..n]).await.is_err() {
                        break;
                    }
                    a_to_b += n as u64;
                }
            },
            read = b_read.read(&mut b_buf), if b_open => match read {
                Ok(0) | Err(_) => {
                    b_open = false;
                    let _ = a_write.shutdown().await;
                }
                Ok(n) => {
                    if a_write.write_all(&b_buf[..n]).await.is_err() {
                        break;
                    }
                    b_to_a += n as u64;
                }
            },
            () = tokio::time::sleep(idle) => break,
        }
    }
    (a_to_b, b_to_a)
}

/// Read up to the end of the request head; the head and any bytes after it.
async fn read_head<S: AsyncRead + Unpin>(client: &mut S) -> std::io::Result<(Vec<u8>, Vec<u8>)> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            return Ok((buf, rest));
        }
        if buf.len() > MAX_HEAD {
            return Err(std::io::Error::other("request head too long"));
        }
        let n = client.read(&mut chunk).await?;
        if n == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

fn parse(head: &[u8], routes: &[SecretRoute], token: &str, local: bool) -> Result<Request, String> {
    let mut headers = [httparse::EMPTY_HEADER; 64];
    let mut req = httparse::Request::new(&mut headers);
    match req.parse(head) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return Err("malformed request".into()),
    }
    let method = req.method.ok_or("malformed request")?;
    let target = req.path.ok_or("malformed request")?;
    if method.eq_ignore_ascii_case("CONNECT") {
        let (host, port) = split_authority(target, None)?;
        return Ok(Request::Connect { host, port });
    }
    if let Some(routed) = target.strip_prefix("/_forge/") {
        let Some(routed) = routed.strip_prefix(token).and_then(|r| r.strip_prefix('/')) else {
            return Ok(Request::UnknownRoute);
        };
        let (name, path) = match routed.find('/') {
            Some(i) => (&routed[..i], &routed[i..]),
            None => (routed, "/"),
        };
        if local && name == LOCAL_ROUTE && path == "/" {
            return Ok(local_request(method, req.headers));
        }
        let Some(route) = routes.iter().find(|r| r.secret.route == name) else {
            return Ok(Request::UnknownRoute);
        };
        let refuse = |reason: String| {
            Ok(Request::RouteRefused {
                host: route.secret.host.to_owned(),
                reason,
            })
        };
        if let Err(why) = route_caller_ok(req.headers) {
            return refuse(why);
        }
        if !matches!(method, "GET" | "POST") {
            return refuse(format!("{method} is not allowed on a secret's route"));
        }
        if !route.secret.allows_path(path) {
            // The path without its query, which may carry anything.
            let asked = path.split(['?', '#']).next().unwrap_or_default();
            return refuse(format!(
                "{} may only be sent to {}, not {}",
                route.secret.name,
                route.secret.paths.join(", "),
                clip(asked.to_owned(), 120)
            ));
        }
        return Ok(Request::Route {
            host: route.secret.host.to_owned(),
            route: route.clone(),
            head: route_head(method, path, req.headers, route),
        });
    }
    let Some(rest) = target.strip_prefix("http://") else {
        return Err("only CONNECT and absolute http:// requests are proxied".into());
    };
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let (host, port) = split_authority(authority, Some(80))?;
    let mut out = format!("{method} {path} HTTP/1.1\r\n").into_bytes();
    for h in req.headers.iter() {
        let name = h.name.to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "connection" | "proxy-connection" | "proxy-authorization"
        ) {
            continue;
        }
        out.extend_from_slice(h.name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(h.value);
        out.extend_from_slice(b"\r\n");
    }
    out.extend_from_slice(b"Connection: close\r\n\r\n");
    Ok(Request::Http {
        host,
        port,
        head: out,
    })
}

/// A request on the local route: only a `POST` from a sandboxed client, with a body of known
/// length that fits.
fn local_request(method: &str, headers: &[httparse::Header<'_>]) -> Request {
    let refuse = |code, reason: String| Request::LocalRefused { code, reason };
    if let Err(why) = route_caller_ok(headers) {
        return refuse(403, why);
    }
    if method != "POST" {
        return refuse(
            405,
            format!("{method} is not served on Forge's endpoint; POST a request"),
        );
    }
    let mut length = None;
    let mut json = false;
    let mut kept = Vec::new();
    for h in headers {
        let name = h.name.to_ascii_lowercase();
        let value = String::from_utf8_lossy(h.value).into_owned();
        match name.as_str() {
            // JSON-RPC is always JSON, and a page can't send JSON without a preflight the
            // proxy refuses; a form's text/plain POST can't pass for it.
            "content-type" => {
                json = value
                    .split(';')
                    .next()
                    .is_some_and(|t| t.trim().eq_ignore_ascii_case("application/json"));
            }
            "transfer-encoding" => {
                return refuse(411, "a request body must have a Content-Length".into());
            }
            "content-length" => match value.trim().parse::<usize>() {
                Ok(n) if length.is_none() => length = Some(n),
                _ => return refuse(400, "bad Content-Length".into()),
            },
            _ => {}
        }
        kept.push((name, value));
    }
    if !json {
        return refuse(
            415,
            "requests to Forge's endpoint must be application/json".into(),
        );
    }
    match length {
        None => refuse(411, "a request body must have a Content-Length".into()),
        Some(n) if n > MAX_LOCAL_BODY => refuse(413, "request body too large".into()),
        Some(length) => Request::Local {
            headers: kept,
            length,
        },
    }
}

/// Read the local request's body, have `service` answer it, and send the answer.
async fn serve_local<S>(
    mut client: S,
    service: &dyn LocalService,
    headers: Vec<(String, String)>,
    length: usize,
    mut body: Vec<u8>,
) where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if body.len() > length {
        let _ = respond(&mut client, 400, "more data than the Content-Length").await;
        return;
    }
    let missing = length - body.len();
    let mut more = vec![0; missing];
    match tokio::time::timeout(HEAD_TIMEOUT, client.read_exact(&mut more)).await {
        Ok(Ok(_)) => body.extend_from_slice(&more),
        _ => return,
    }
    let answer = service.call(LocalRequest { headers, body }).await;
    let status = match answer.status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let mut response = format!(
        "HTTP/1.1 {} {status}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        answer.status,
        answer.content_type,
        answer.body.len()
    )
    .into_bytes();
    response.extend_from_slice(&answer.body);
    if client.write_all(&response).await.is_ok() {
        let _ = client.shutdown().await;
    }
}

/// Why a route request's headers show it isn't from a sandboxed client, if they do: its one
/// `Host` must be loopback (a page rebinding its own name to 127.0.0.1 still sends that name),
/// and a browser's `Origin` and Fetch Metadata headers are refused. `Sec-Fetch-Mode` alone is
/// accepted: browsers only send it with `Sec-Fetch-Site` (and page script can drop neither,
/// nor an `Origin`), while Node's `fetch` sends it alone on every request. Its modes that only
/// a browser uses (navigations and WebSockets) are refused even alone.
fn route_caller_ok(headers: &[httparse::Header<'_>]) -> Result<(), String> {
    let mut host = None;
    for h in headers {
        let name = h.name.to_ascii_lowercase();
        let browser_mode = name == "sec-fetch-mode"
            && matches!(
                String::from_utf8_lossy(h.value)
                    .trim()
                    .to_ascii_lowercase()
                    .as_str(),
                "navigate" | "nested-navigate" | "websocket"
            );
        if name == "origin"
            || (name.starts_with("sec-fetch-") && name != "sec-fetch-mode")
            || browser_mode
        {
            return Err(format!(
                "requests with {} are refused on Forge's routes",
                h.name
            ));
        }
        if name == "host" {
            if host.is_some() {
                return Err("a request may have only one Host header".into());
            }
            host = Some(String::from_utf8_lossy(h.value).to_ascii_lowercase());
        }
    }
    let host = host.ok_or("a Host header is required on Forge's routes")?;
    let name = match host.rsplit_once(':') {
        Some((name, port)) if port.chars().all(|c| c.is_ascii_digit()) => name,
        _ => host.as_str(),
    };
    if matches!(name, "127.0.0.1" | "localhost" | "[::1]") {
        Ok(())
    } else {
        Err(format!("Host {host:?} is not loopback"))
    }
}

/// The request for a route's host: the path without the route, its own `Host`, and none of
/// the secret's header the client sent; [`finish_route_head`] adds the real one.
fn route_head(
    method: &str,
    path: &str,
    headers: &[httparse::Header<'_>],
    route: &SecretRoute,
) -> Vec<u8> {
    let secret = route.secret;
    let mut out = format!("{method} {path} HTTP/1.1\r\nHost: {}\r\n", secret.host).into_bytes();
    for h in headers {
        let name = h.name.to_ascii_lowercase();
        let dropped = matches!(
            name.as_str(),
            "host" | "connection" | "proxy-connection" | "proxy-authorization"
        ) || name == secret.header.name();
        if dropped {
            continue;
        }
        out.extend_from_slice(h.name.as_bytes());
        out.extend_from_slice(b": ");
        out.extend_from_slice(h.value);
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// End a route's request head with the secret's header carrying `value`, refusing a value
/// that would inject headers of its own.
fn finish_route_head(
    mut head: Vec<u8>,
    route: &SecretRoute,
    value: &Secret,
) -> Result<Vec<u8>, String> {
    if value.expose().is_empty() || value.expose().chars().any(char::is_control) {
        return Err(format!(
            "the value of {} is empty or contains control characters",
            route.secret.name
        ));
    }
    head.extend_from_slice(route.secret.header.line(value.expose()).as_bytes());
    head.extend_from_slice(b"Connection: close\r\n\r\n");
    Ok(head)
}

/// `host:port`, with `default` when the port is missing.
fn split_authority(authority: &str, default: Option<u16>) -> Result<(String, u16), String> {
    if authority.contains('@') {
        return Err("credentials in the request target are not allowed".into());
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') || host.ends_with(']') => {
            let port = port
                .parse::<u16>()
                .map_err(|_| format!("bad port in {authority:?}"))?;
            (host, port)
        }
        _ => (
            authority,
            default.ok_or_else(|| format!("{authority:?} has no port"))?,
        ),
    };
    if host.is_empty() {
        return Err("empty host".into());
    }
    Ok((host.to_owned(), port))
}

async fn connect(config: &ProxyConfig, host: &str, port: u16) -> Result<TcpStream, String> {
    let addrs = config.resolver.resolve(host, port).await?;
    let mut last = format!("{host} has no addresses");
    for addr in addrs {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(addr)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last = format!("connecting to {host}: {e}"),
            Err(_) => last = format!("connecting to {host} timed out"),
        }
    }
    Err(last)
}

async fn respond<S: AsyncWrite + Unpin>(
    client: &mut S,
    code: u16,
    why: &str,
) -> std::io::Result<()> {
    let status = match code {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        _ => "Bad Gateway",
    };
    let body = format!("forge egress proxy: {why}\n");
    let response = format!(
        "HTTP/1.1 {code} {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    client.write_all(response.as_bytes()).await?;
    client.shutdown().await
}

impl Proxy {
    /// The secret path segment of every route, for the log redactor.
    pub fn token(&self) -> &str {
        &self.state.token
    }

    /// The local route's URL, for a sandboxed process that reaches the proxy at `url`.
    pub fn local_url(&self, url: &str) -> String {
        format!("{url}/_forge/{}/{LOCAL_ROUTE}", self.state.token)
    }

    /// The variables that point a sandboxed process at this proxy, reached at `url`: the proxy
    /// variables, and for each secret a placeholder value and the base URL of its route.
    pub fn env(&self, url: &str) -> Vec<(String, String)> {
        let mut vars = proxy_vars(url);
        for route in &self.state.config.routes {
            let secret = route.secret;
            vars.push((secret.name.to_owned(), PLACEHOLDER.to_owned()));
            vars.push((
                secret.base_url_var.to_owned(),
                format!(
                    "{url}/_forge/{}/{}{}",
                    self.state.token, secret.route, secret.base_url_suffix
                ),
            ));
        }
        vars
    }
}

fn proxy_vars(url: &str) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    for name in ["HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"] {
        vars.push((name.to_owned(), url.to_owned()));
        vars.push((name.to_ascii_lowercase(), url.to_owned()));
    }
    let local = "localhost,127.0.0.1,::1";
    vars.push(("NO_PROXY".into(), local.into()));
    vars.push(("no_proxy".into(), local.into()));
    vars
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::net::Ipv4Addr;

    /// Resolves listed names to fixed addresses, and nothing else.
    struct Mapped(HashMap<String, SocketAddr>);

    impl Resolve for Mapped {
        fn resolve<'a>(
            &'a self,
            host: &'a str,
            _port: u16,
        ) -> BoxFuture<'a, Result<Vec<SocketAddr>, String>> {
            let found = self.0.get(host).copied();
            Box::pin(async move {
                found
                    .map(|a| vec![a])
                    .ok_or_else(|| format!("{host} unknown"))
            })
        }
    }

    struct Harness {
        proxy: Proxy,
        log: Arc<Mutex<Vec<Connection>>>,
    }

    async fn harness(allow: &[&str], resolver: Arc<dyn Resolve>) -> Harness {
        harness_with(allow, resolver, Limits::default()).await
    }

    async fn harness_with(allow: &[&str], resolver: Arc<dyn Resolve>, limits: Limits) -> Harness {
        harness_full(allow, resolver, limits, vec![]).await
    }

    /// Serving `routes`, with no TLS to their hosts.
    async fn routed(resolver: Arc<dyn Resolve>, routes: Vec<SecretRoute>) -> Harness {
        harness_full(&[], resolver, Limits::default(), routes).await
    }

    fn route(name: &str, value: &str) -> SecretRoute {
        SecretRoute::fixed(crate::secrets::known(name).unwrap(), Secret::new(value))
    }

    async fn harness_full(
        allow: &[&str],
        resolver: Arc<dyn Resolve>,
        limits: Limits,
        routes: Vec<SecretRoute>,
    ) -> Harness {
        let log: Arc<Mutex<Vec<Connection>>> = Arc::default();
        let sink = log.clone();
        let proxy = Proxy::start(ProxyConfig {
            allow: Allowlist::new(allow).unwrap(),
            resolver,
            log: Arc::new(move |c| lock(&sink).push(c)),
            limits,
            routes,
            upstream: Some(Arc::new(PlainUpstream)),
            local: None,
        })
        .await
        .unwrap();
        Harness { proxy, log }
    }

    /// Echoes the body back with the headers it saw, as JSON.
    struct Echo;

    impl LocalService for Echo {
        fn call(
            &self,
            request: LocalRequest,
        ) -> Pin<Box<dyn Future<Output = LocalResponse> + Send + '_>> {
            Box::pin(async move {
                let mut body = request.body;
                body.extend_from_slice(format!(" {}", request.headers.len()).as_bytes());
                LocalResponse {
                    status: 200,
                    content_type: "application/json",
                    body,
                }
            })
        }
    }

    #[tokio::test]
    async fn the_local_route_is_served_only_to_sandboxed_posts_with_the_token() {
        let log: Arc<Mutex<Vec<Connection>>> = Arc::default();
        let sink = log.clone();
        let proxy = Proxy::start(ProxyConfig {
            allow: Allowlist::new(&[] as &[&str]).unwrap(),
            resolver: Arc::new(PublicDns),
            log: Arc::new(move |c| lock(&sink).push(c)),
            limits: Limits::default(),
            routes: vec![],
            upstream: None,
            local: Some(Arc::new(Echo)),
        })
        .await
        .unwrap();
        let url = proxy.local_url("http://127.0.0.1:1");
        let path = url.strip_prefix("http://127.0.0.1:1").unwrap();
        assert!(
            path.ends_with("/mcp") && path.starts_with("/_forge/"),
            "{path}"
        );
        let post = |path: &str, extra: &str, body: &str| {
            format!(
                "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\n\r\n{body}",
                body.len()
            )
        };
        let reply = exchange(proxy.port(), post(path, "", "{}").as_bytes()).await;
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(reply.ends_with("{} 3"), "{reply}");
        // Node's fetch, as Gemini CLI's MCP client sends it.
        let reply = exchange(
            proxy.port(),
            post(
                path,
                "accept: application/json, text/event-stream\r\nsec-fetch-mode: cors\r\n",
                "{}",
            )
            .as_bytes(),
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        // A body sent in pieces is read whole.
        let mut s = TcpStream::connect(("127.0.0.1", proxy.port()))
            .await
            .unwrap();
        let head = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: 6\r\n\r\nabc"
        );
        s.write_all(head.as_bytes()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        s.write_all(b"def").await.unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).await.unwrap();
        assert!(String::from_utf8_lossy(&out).ends_with("abcdef 3"));

        let wrong_token = "/_forge/nottoken/mcp";
        for (request, code) in [
            (post(wrong_token, "", "{}"), "404"),
            (post(path, "Origin: https://evil.example\r\n", "{}"), "403"),
            (
                post(
                    path,
                    "Sec-Fetch-Mode: cors\r\nSec-Fetch-Site: cross-site\r\n",
                    "{}",
                ),
                "403",
            ),
            // A form can post text/plain without a preflight; JSON-RPC is always JSON.
            (
                format!(
                    "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{{}}"
                ),
                "415",
            ),
            (
                format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{{}}"),
                "415",
            ),
            (
                format!(
                    "POST {path} HTTP/1.1\r\nHost: evil.example\r\nContent-Length: 2\r\n\r\n{{}}"
                ),
                "403",
            ),
            (
                format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
                "405",
            ),
            (
                format!(
                    "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n"
                ),
                "411",
            ),
            (
                format!(
                    "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 99999999\r\n\r\n"
                ),
                "413",
            ),
        ] {
            let reply = exchange(proxy.port(), request.as_bytes()).await;
            assert!(
                reply.starts_with(&format!("HTTP/1.1 {code}")),
                "{request}: {reply}"
            );
        }
        // Nothing on the local route leaves the machine, so nothing is logged as a connection.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(lock(&log).is_empty(), "{:?}", lock(&log));

        // A proxy without a local service has no such route.
        let h = harness(&[], Arc::new(PublicDns)).await;
        let path = h.proxy.local_url("");
        let reply = exchange(h.proxy.port(), post(&path, "", "{}").as_bytes()).await;
        assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");
    }

    /// A server that answers `pong` to anything and records what it got.
    async fn upstream() -> (SocketAddr, Arc<Mutex<Vec<u8>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Arc<Mutex<Vec<u8>>> = Arc::default();
        let record = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut s, _)) = listener.accept().await {
                let record = record.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 4096];
                    if let Ok(n) = s.read(&mut buf).await {
                        lock(&record).extend_from_slice(&buf[..n]);
                        let _ = s.write_all(b"pong").await;
                    }
                });
            }
        });
        (addr, seen)
    }

    async fn exchange(port: u16, request: &[u8]) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(request).await.unwrap();
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
        String::from_utf8_lossy(&out).into_owned()
    }

    async fn logged(h: &Harness, n: usize) -> Vec<Connection> {
        for _ in 0..100 {
            if lock(&h.log).len() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        lock(&h.log).clone()
    }

    #[tokio::test]
    async fn connect_tunnels_to_allowlisted_hosts_and_logs_bytes() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("api.example.com".into(), addr)])));
        let h = harness(&["API.example.com."], resolver).await;
        let reply = exchange(
            h.proxy.port(),
            b"CONNECT api.example.com:443 HTTP/1.1\r\n\r\nping",
        )
        .await;
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(reply.ends_with("pong"), "{reply}");
        assert_eq!(&*lock(&seen), b"ping");
        let log = logged(&h, 1).await;
        assert_eq!(log.len(), 1);
        assert!(log[0].allowed);
        assert_eq!(
            (log[0].host.as_str(), log[0].port),
            ("api.example.com", 443)
        );
        assert_eq!((log[0].bytes_sent, log[0].bytes_received), (4, 4));
    }

    #[tokio::test]
    async fn hosts_off_the_allowlist_are_denied_and_logged() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([
            ("evil.example".into(), addr),
            ("api.example.com".into(), addr),
        ])));
        let h = harness(&["api.example.com"], resolver).await;
        let cases: [&[u8]; 4] = [
            b"CONNECT evil.example:443 HTTP/1.1\r\n\r\n",
            b"CONNECT api.example.com:22 HTTP/1.1\r\n\r\n",
            b"CONNECT 127.0.0.1:443 HTTP/1.1\r\n\r\n",
            b"CONNECT [::1]:443 HTTP/1.1\r\n\r\n",
        ];
        for request in cases {
            let reply = exchange(h.proxy.port(), request).await;
            assert!(reply.starts_with("HTTP/1.1 403"), "{reply}");
        }
        let log = logged(&h, 4).await;
        assert_eq!(log.len(), 4);
        assert!(
            log.iter().all(|c| !c.allowed && c.reason.is_some()),
            "{log:?}"
        );
        assert!(log[0].reason.as_deref().unwrap().contains("allowlist"));
        assert!(lock(&seen).is_empty(), "nothing reached the upstream");
    }

    #[tokio::test]
    async fn allowlisted_names_that_resolve_to_private_addresses_are_refused() {
        // Real DNS: `localhost` resolves to loopback.
        let h = harness(&["localhost"], Arc::new(PublicDns)).await;
        let reply = exchange(h.proxy.port(), b"CONNECT localhost:443 HTTP/1.1\r\n\r\n").await;
        assert!(reply.starts_with("HTTP/1.1 502"), "{reply}");
        let log = logged(&h, 1).await;
        assert!(!log[0].allowed);
        assert!(
            log[0]
                .reason
                .as_deref()
                .unwrap()
                .contains("not a public address"),
            "{log:?}"
        );
    }

    #[tokio::test]
    async fn plain_http_is_rewritten_for_the_origin_server() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("example.com".into(), addr)])));
        let h = harness(&["example.com"], resolver).await;
        let reply = exchange(
            h.proxy.port(),
            b"GET http://example.com/a?b HTTP/1.1\r\nHost: example.com\r\nProxy-Connection: keep-alive\r\nProxy-Authorization: x\r\n\r\n",
        )
        .await;
        assert_eq!(reply, "pong");
        let got = String::from_utf8(lock(&seen).clone()).unwrap();
        assert!(got.starts_with("GET /a?b HTTP/1.1\r\n"), "{got}");
        assert!(got.contains("Host: example.com\r\n") && got.contains("Connection: close\r\n"));
        assert!(!got.to_ascii_lowercase().contains("proxy-"), "{got}");
        assert!(logged(&h, 1).await[0].allowed);
    }

    #[tokio::test]
    async fn non_proxy_requests_are_rejected() {
        let h = harness(&["example.com"], Arc::new(PublicDns)).await;
        for request in [
            &b"GET /x HTTP/1.1\r\nHost: example.com\r\n\r\n"[..],
            b"GET https://example.com/ HTTP/1.1\r\n\r\n",
            b"CONNECT user@example.com:443 HTTP/1.1\r\n\r\n",
            b"not http\r\n\r\n",
        ] {
            let reply = exchange(h.proxy.port(), request).await;
            assert!(reply.starts_with("HTTP/1.1 400"), "{reply}");
        }
    }

    #[tokio::test]
    async fn serves_a_unix_socket_and_removes_it_when_dropped() {
        let (addr, _) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("example.com".into(), addr)])));
        let h = harness(&["example.com"], resolver).await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("proxy.sock");
        std::fs::write(&path, "stale").unwrap();
        h.proxy.serve_unix(&path).unwrap();
        let mut s = tokio::net::UnixStream::connect(&path).await.unwrap();
        s.write_all(b"CONNECT example.com:443 HTTP/1.1\r\n\r\nping")
            .await
            .unwrap();
        let mut out = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
        assert!(String::from_utf8_lossy(&out).ends_with("pong"));
        drop(h);
        assert!(!path.exists());
    }

    #[test]
    fn allowlist_entries_must_be_plain_host_names() {
        assert!(Allowlist::new(["crates.io", "static.crates.io", "a-b.example."]).is_ok());
        for bad in [
            "https://crates.io",
            "crates.io:443",
            "*.npmjs.org",
            "10.0.0.1",
            "::1",
            "a/b",
            "",
            "x..y",
            "1.2.3",
        ] {
            assert!(Allowlist::new([bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_public_addresses_are_public() {
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "2606:4700:4700::1111",
            "2002:808:808::1",
        ] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "127.0.0.1",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "240.0.0.1",
            "::1",
            "::",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "ff02::1",
            "2001:db8::1",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            "64:ff9b:1::a00:1",
            "::7f00:1",
            "::a00:1",
            "fec0::1",
            "2002:7f00:1::1",
            "2002:a00:1::1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
        assert!(is_public(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))));
    }

    #[tokio::test]
    async fn connections_beyond_the_limit_wait_unaccepted() {
        let (addr, _) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("example.com".into(), addr)])));
        let limits = Limits {
            max_connections: 1,
            ..Limits::default()
        };
        let h = harness_with(&["example.com"], resolver, limits).await;
        // The first connection takes the only slot and sends nothing.
        let first = TcpStream::connect(("127.0.0.1", h.proxy.port()))
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut second = TcpStream::connect(("127.0.0.1", h.proxy.port()))
            .await
            .unwrap();
        second
            .write_all(b"CONNECT example.com:443 HTTP/1.1\r\n\r\nping")
            .await
            .unwrap();
        let mut buf = [0u8; 64];
        let waited = tokio::time::timeout(Duration::from_millis(300), second.read(&mut buf)).await;
        assert!(waited.is_err(), "served while the only slot was taken");
        drop(first);
        let n = tokio::time::timeout(Duration::from_secs(5), second.read(&mut buf))
            .await
            .unwrap()
            .unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
    }

    /// Waiting connections cost the daemon no file descriptors. Accepted sockets on the
    /// proxy's port show in `/proc/net/tcp` with an inode; connections still in the kernel's
    /// backlog have none.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn waiting_connections_use_no_daemon_descriptors() {
        let limits = Limits {
            max_connections: 1,
            ..Limits::default()
        };
        let h = harness_with(&["example.com"], Arc::new(PublicDns), limits).await;
        let port = h.proxy.port();
        let accepted = || {
            std::fs::read_to_string("/proc/net/tcp")
                .unwrap()
                .lines()
                .skip(1)
                .filter(|line| {
                    let f: Vec<&str> = line.split_whitespace().collect();
                    let local_port = f[1].rsplit(':').next().unwrap();
                    // State 01 is established; field 9 is the socket's inode.
                    u16::from_str_radix(local_port, 16) == Ok(port) && f[3] == "01" && f[9] != "0"
                })
                .count()
        };
        let mut clients = Vec::new();
        for _ in 0..40 {
            clients.push(TcpStream::connect(("127.0.0.1", port)).await.unwrap());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            accepted(),
            1,
            "only the connection holding the slot is accepted"
        );
    }

    #[tokio::test]
    async fn idle_tunnels_are_closed() {
        let (addr, _) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("example.com".into(), addr)])));
        let limits = Limits {
            idle: Duration::from_millis(200),
            ..Limits::default()
        };
        let h = harness_with(&["example.com"], resolver, limits).await;
        let mut s = TcpStream::connect(("127.0.0.1", h.proxy.port()))
            .await
            .unwrap();
        s.write_all(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let mut out = Vec::new();
        // The upstream never answers without input, so only the idle timeout ends this.
        let ended = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
        assert!(ended.is_ok(), "the idle tunnel stayed open");
        assert!(logged(&h, 1).await[0].allowed);
    }

    #[tokio::test]
    async fn denial_floods_are_rate_limited_and_clipped() {
        let limits = Limits {
            denials_per_second: 3,
            ..Limits::default()
        };
        let h = harness_with(&["example.com"], Arc::new(PublicDns), limits).await;
        let long = format!("CONNECT {}:443 HTTP/1.1\r\n\r\n", "a".repeat(300));
        for _ in 0..10 {
            exchange(h.proxy.port(), long.as_bytes()).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let log = lock(&h.log).clone();
        assert_eq!(log.len(), 3, "only the per-second limit is logged");
        assert!(
            log.iter()
                .all(|c| c.host.len() <= MAX_LOGGED_HOST + '…'.len_utf8())
        );
        assert!(
            log.iter()
                .all(|c| c.reason.as_ref().unwrap().len() <= MAX_LOGGED_REASON + 3)
        );
        tokio::time::sleep(Duration::from_millis(1000)).await;
        exchange(
            h.proxy.port(),
            b"CONNECT other.example:443 HTTP/1.1\r\n\r\n",
        )
        .await;
        let log = logged(&h, 4).await;
        let reason = log[3].reason.clone().unwrap();
        assert!(
            reason.contains("7 earlier denials were not logged"),
            "{reason}"
        );
    }

    #[tokio::test]
    async fn dropping_the_proxy_closes_open_tunnels() {
        let (addr, _) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("example.com".into(), addr)])));
        let h = harness(&["example.com"], resolver).await;
        let mut s = TcpStream::connect(("127.0.0.1", h.proxy.port()))
            .await
            .unwrap();
        s.write_all(b"CONNECT example.com:443 HTTP/1.1\r\n\r\n")
            .await
            .unwrap();
        let mut buf = [0u8; 64];
        let n = s.read(&mut buf).await.unwrap();
        assert!(String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"));
        drop(h);
        let mut rest = Vec::new();
        let ended = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut rest)).await;
        assert!(ended.is_ok(), "the tunnel outlived the proxy");
    }

    #[tokio::test]
    async fn routes_send_the_real_secret_to_the_fixed_host_only() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("api.anthropic.com".into(), addr)])));
        // No allowlist at all: a route's host is fixed by the secret.
        let h = routed(resolver, vec![route("ANTHROPIC_API_KEY", "sk-real")]).await;
        let request = format!(
            "POST /_forge/{}/anthropic/v1/messages?beta=1 HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nX-Api-Key: {PLACEHOLDER}\r\nContent-Length: 2\r\n\r\nhi",
            h.proxy.state.token,
            h.proxy.port()
        );
        let reply = exchange(h.proxy.port(), request.as_bytes()).await;
        assert_eq!(reply, "pong");
        let got = String::from_utf8(lock(&seen).clone()).unwrap();
        assert!(
            got.starts_with("POST /v1/messages?beta=1 HTTP/1.1\r\nHost: api.anthropic.com\r\n"),
            "{got}"
        );
        assert!(got.contains("x-api-key: sk-real\r\n"), "{got}");
        assert!(
            !got.contains(PLACEHOLDER) && !got.contains("127.0.0.1"),
            "{got}"
        );
        assert!(
            got.contains("Content-Length: 2\r\n") && got.ends_with("hi"),
            "{got}"
        );
        let log = logged(&h, 1).await;
        assert!(log[0].allowed);
        assert_eq!(
            (log[0].host.as_str(), log[0].port),
            ("api.anthropic.com", 443)
        );
        assert!(!format!("{log:?}{:?}", h.proxy).contains("sk-real"));
    }

    #[tokio::test]
    async fn bearer_routes_replace_the_authorization_header() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("api.openai.com".into(), addr)])));
        let h = routed(resolver, vec![route("OPENAI_API_KEY", "sk-open")]).await;
        let request = format!(
            "GET /_forge/{}/openai/v1/models HTTP/1.1\r\nHost: localhost:3128\r\nauthorization: Bearer {PLACEHOLDER}\r\n\r\n",
            h.proxy.state.token
        );
        assert_eq!(exchange(h.proxy.port(), request.as_bytes()).await, "pong");
        let got = String::from_utf8(lock(&seen).clone()).unwrap();
        assert!(got.starts_with("GET /v1/models HTTP/1.1\r\n"), "{got}");
        assert_eq!(
            got.to_ascii_lowercase().matches("authorization:").count(),
            1,
            "{got}"
        );
        assert!(got.contains("Authorization: Bearer sk-open\r\n"), "{got}");
    }

    /// A value that changes between requests, or can't be read: a login.
    struct Changing(Mutex<Vec<Result<Secret, String>>>);

    impl SecretSource for Changing {
        fn current(&self) -> Result<Secret, String> {
            lock(&self.0).remove(0)
        }
    }

    #[tokio::test]
    async fn routes_read_their_value_for_each_request() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("chatgpt.com".into(), addr)])));
        let values = Changing(Mutex::new(vec![
            Ok(Secret::new("tok-1")),
            Ok(Secret::new("tok-2")),
            Err("the ChatGPT login has expired; run codex".into()),
            Ok(Secret::new("tok\r\nX-Injected: 1")),
        ]));
        let login = SecretRoute {
            secret: crate::secrets::known("CODEX_CHATGPT_TOKEN").unwrap(),
            value: Arc::new(values),
        };
        let h = routed(resolver, vec![login]).await;
        let request = format!(
            "POST /_forge/{}/chatgpt/backend-api/codex/responses HTTP/1.1\r\nHost: localhost:3128\r\nAuthorization: Bearer {PLACEHOLDER}\r\nContent-Length: 0\r\n\r\n",
            h.proxy.state.token
        );
        for _ in 0..2 {
            assert_eq!(exchange(h.proxy.port(), request.as_bytes()).await, "pong");
        }
        let got = String::from_utf8(lock(&seen).clone()).unwrap();
        assert!(got.contains("Authorization: Bearer tok-1\r\n"), "{got}");
        assert!(got.contains("Authorization: Bearer tok-2\r\n"), "{got}");
        // An unreadable value, or one that would add headers, sends nothing.
        let expired = exchange(h.proxy.port(), request.as_bytes()).await;
        assert!(expired.starts_with("HTTP/1.1 401"), "{expired}");
        assert!(expired.contains("run codex"), "{expired}");
        let injected = exchange(h.proxy.port(), request.as_bytes()).await;
        assert!(injected.starts_with("HTTP/1.1 401"), "{injected}");
        let got = String::from_utf8(lock(&seen).clone()).unwrap();
        assert!(!got.contains("X-Injected"), "{got}");
        assert_eq!(got.matches("POST ").count(), 2, "{got}");
        let log = logged(&h, 4).await;
        assert_eq!(log.iter().filter(|c| !c.allowed).count(), 2);
    }

    #[tokio::test]
    async fn routes_a_session_lacks_are_not_found() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("api.openai.com".into(), addr)])));
        let h = routed(resolver, vec![route("ANTHROPIC_API_KEY", "sk-real")]).await;
        let token = h.proxy.state.token.clone();
        for path in [
            format!("/_forge/{token}/openai/v1/models"),
            format!("/_forge/{token}/nope"),
            format!("/_forge/{token}"),
            // Without the token, even an existing route is not there.
            "/_forge/anthropic/v1/messages".to_owned(),
            "/_forge/0000/anthropic/v1/messages".to_owned(),
            "/_forge/".to_owned(),
        ] {
            let request = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n");
            let reply = exchange(h.proxy.port(), request.as_bytes()).await;
            assert!(reply.starts_with("HTTP/1.1 404"), "{path}: {reply}");
            assert!(!reply.contains(&token), "{reply}");
        }
        assert!(lock(&seen).is_empty());
    }

    #[tokio::test]
    async fn env_points_clients_at_their_routes_with_placeholders() {
        let h = routed(
            Arc::new(PublicDns),
            vec![
                route("ANTHROPIC_API_KEY", "sk-a"),
                route("OPENAI_API_KEY", "sk-o"),
            ],
        )
        .await;
        let env: HashMap<String, String> =
            h.proxy.env("http://127.0.0.1:3128").into_iter().collect();
        assert_eq!(env["ANTHROPIC_API_KEY"], PLACEHOLDER);
        assert_eq!(env["OPENAI_API_KEY"], PLACEHOLDER);
        let token = &h.proxy.state.token;
        assert_eq!(token.len(), 32);
        assert_eq!(
            env["ANTHROPIC_BASE_URL"],
            format!("http://127.0.0.1:3128/_forge/{token}/anthropic")
        );
        assert_eq!(
            env["OPENAI_BASE_URL"],
            format!("http://127.0.0.1:3128/_forge/{token}/openai/v1")
        );
        assert_eq!(env["HTTPS_PROXY"], "http://127.0.0.1:3128");
        assert!(!env.contains_key("GOOGLE_GEMINI_BASE_URL"));
        assert!(!env.values().any(|v| v.starts_with("sk-")));
    }

    #[test]
    fn tls_loads_the_systems_trusted_roots() {
        assert!(Tls::shared().is_ok(), "{:?}", Tls::shared().err());
    }

    /// Requests that know the token but don't look like a sandboxed client, or that would use
    /// the secret for something other than inference, are refused and logged.
    #[tokio::test]
    async fn node_fetch_reaches_a_secrets_route() {
        // What Node's fetch (undici) sends on every request: Sec-Fetch-Mode, alone.
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("api.anthropic.com".into(), addr)])));
        let h = routed(resolver, vec![route("ANTHROPIC_API_KEY", "sk-real")]).await;
        let base = format!("/_forge/{}/anthropic", h.proxy.state.token);
        for mode in ["cors", "no-cors", "same-origin"] {
            lock(&seen).clear();
            let request = format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1:3128\r\nx-api-key: {PLACEHOLDER}\r\naccept-language: *\r\nsec-fetch-mode: {mode}\r\nuser-agent: node\r\nContent-Length: 2\r\n\r\n{{}}"
            );
            let reply = exchange(h.proxy.port(), request.as_bytes()).await;
            assert!(reply.ends_with("pong"), "{mode}: {reply}");
            let sent = String::from_utf8_lossy(&lock(&seen)).into_owned();
            assert!(sent.contains("x-api-key: sk-real"), "{sent}");
        }
    }

    #[tokio::test]
    async fn route_requests_from_browsers_or_for_other_uses_are_refused() {
        let (addr, seen) = upstream().await;
        let resolver = Arc::new(Mapped(HashMap::from([("api.anthropic.com".into(), addr)])));
        let h = routed(resolver, vec![route("ANTHROPIC_API_KEY", "sk-real")]).await;
        let base = format!("/_forge/{}/anthropic", h.proxy.state.token);
        let cases = [
            // A page that rebound its name to 127.0.0.1 still sends that name.
            format!("POST {base}/v1/messages HTTP/1.1\r\nHost: evil.example:8080\r\n\r\n"),
            format!("POST {base}/v1/messages HTTP/1.1\r\n\r\n"),
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://127.0.0.1\r\n\r\n"
            ),
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-Site: cross-site\r\n\r\n"
            ),
            format!("TRACE {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
            format!("DELETE {base}/v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
            format!("POST {base}/v1/organizations/api_keys HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
            format!("POST {base}/v1/messages/../organizations HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"),
            format!(
                "POST {base}/v1/messages/%2E%2E/organizations HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n"
            ),
            // Browsers send Sec-Fetch-Mode only with Sec-Fetch-Site, or with an Origin.
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-Mode: cors\r\nSec-Fetch-Site: cross-site\r\n\r\n"
            ),
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-Mode: cors\r\nOrigin: http://localhost:5173\r\n\r\n"
            ),
            format!("POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: null\r\n\r\n"),
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: evil.example\r\nSec-Fetch-Mode: cors\r\nSec-Fetch-Site: same-origin\r\n\r\n"
            ),
            format!(
                "GET {base}/v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-Mode: navigate\r\nSec-Fetch-Dest: document\r\n\r\n"
            ),
            // Modes only a browser uses are refused even alone.
            format!(
                "GET {base}/v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-Mode: navigate\r\n\r\n"
            ),
            format!(
                "GET {base}/v1/models HTTP/1.1\r\nHost: 127.0.0.1\r\nsec-fetch-mode: WebSocket\r\n\r\n"
            ),
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-Dest: empty\r\n\r\n"
            ),
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nSec-Fetch-User: ?1\r\n\r\n"
            ),
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nsec-fetch-mode: cors\r\nSEC-FETCH-SITE: none\r\n\r\n"
            ),
            format!(
                "OPTIONS {base}/v1/messages HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://evil.example\r\nAccess-Control-Request-Private-Network: true\r\n\r\n"
            ),
            // Two Hosts: which one a server believes is anyone's guess.
            format!(
                "POST {base}/v1/messages HTTP/1.1\r\nHost: evil.example\r\nHost: 127.0.0.1\r\n\r\n"
            ),
        ];
        for request in &cases {
            let reply = exchange(h.proxy.port(), request.as_bytes()).await;
            assert!(reply.starts_with("HTTP/1.1 403"), "{request}: {reply}");
            assert!(!reply.contains("Access-Control-Allow"), "{reply}");
        }
        assert!(lock(&seen).is_empty(), "nothing reached the secret's host");
        // Denials are logged at most so many a second.
        let expected = cases
            .len()
            .min(Limits::default().denials_per_second as usize);
        let log = logged(&h, expected).await;
        assert_eq!(log.len(), expected);
        assert!(
            log.iter()
                .all(|c| !c.allowed && c.host == "api.anthropic.com"),
            "{log:?}"
        );
    }
}
