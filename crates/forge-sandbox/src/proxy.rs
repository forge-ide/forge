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
        let state = Arc::new(Shared {
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
}

async fn serve<S>(mut client: S, state: &Shared)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = &state.config;
    let started = Instant::now();
    let (request, rest) = match tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut client)).await {
        Ok(Ok((head, rest))) => match parse(&head) {
            Ok(request) => (request, rest),
            Err(why) => {
                // Nothing to log against a host: the client didn't name one.
                let _ = respond(&mut client, 400, &why).await;
                return;
            }
        },
        _ => return,
    };
    let (host, port) = match &request {
        Request::Connect { host, port } | Request::Http { host, port, .. } => (host.clone(), *port),
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
    if let Err(why) = config.allow.check(&host, port) {
        let _ = respond(&mut client, 403, &why).await;
        log_denial(state, deny(why));
        return;
    }
    let upstream = match connect(config, &host, port).await {
        Ok(stream) => stream,
        Err(why) => {
            let _ = respond(&mut client, 502, &why).await;
            log_denial(state, deny(why));
            return;
        }
    };
    let mut upstream = upstream;
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
        Request::Http { head, .. } => {
            sent += head.len() as u64;
            if upstream.write_all(head).await.is_err() {
                return;
            }
        }
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

fn parse(head: &[u8]) -> Result<Request, String> {
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
        403 => "Forbidden",
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

/// The proxy URL a sandboxed process should use.
pub(crate) fn env(url: &str) -> Vec<(String, String)> {
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
        let log: Arc<Mutex<Vec<Connection>>> = Arc::default();
        let sink = log.clone();
        let proxy = Proxy::start(ProxyConfig {
            allow: Allowlist::new(allow).unwrap(),
            resolver,
            log: Arc::new(move |c| lock(&sink).push(c)),
            limits,
        })
        .await
        .unwrap();
        Harness { proxy, log }
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
}
