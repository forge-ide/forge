//! The sandbox escape suite: every escape must fail inside each tier, and the normal work next
//! to it must still succeed. Run with `just test-sandbox`; a tier this machine can't run fails
//! the suite instead of skipping.

use std::net::{TcpListener, UdpSocket};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use forge_api::model::SandboxTier;
use forge_sandbox::proxy::{
    Allowlist, BoxFuture, Connection, Limits, PlainUpstream, Proxy, ProxyConfig, PublicDns,
    Resolve, SecretRoute,
};
use forge_sandbox::secrets::{PLACEHOLDER, Secret};
use forge_sandbox::{SessionSandbox, Spec, WorktreeMount};
use tempfile::TempDir;

const PROBE: &str = env!("CARGO_BIN_EXE_forge-sandbox-probe");
const HELPER: &str = env!("CARGO_BIN_EXE_forge-sandbox-exec");
const SECRET: &str = "FORGE_ESCAPE_TEST_SECRET";
/// The value the proxy injects on the Anthropic route.
const ROUTE_SECRET: &str = "sk-escape-route";

/// A home with an SSH key, a repo inside it with two session worktrees, a stand-in daemon
/// process and socket, and listeners outside the sandbox.
struct Fixture {
    _dir: TempDir,
    root: PathBuf,
    home: PathBuf,
    repo: PathBuf,
    mine: WorktreeMount,
    other: WorktreeMount,
    daemon: Child,
    _socket: UnixListener,
    socket: PathBuf,
    _bus: UnixListener,
    bus: PathBuf,
    tcp: TcpListener,
    udp: UdpSocket,
    proxy: Arc<Proxy>,
    /// Request heads the stand-in upstream received.
    upstream_requests: Arc<std::sync::Mutex<Vec<String>>>,
    connections: Arc<std::sync::Mutex<Vec<Connection>>>,
}

/// `allowed.test` resolves to a listener on this machine; every other name goes through real
/// DNS with the proxy's public-address check, so allowlisting `localhost` is still refused.
struct TestDns(std::net::SocketAddr);

impl Resolve for TestDns {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
        port: u16,
    ) -> BoxFuture<'a, Result<Vec<std::net::SocketAddr>, String>> {
        if host == "allowed.test" || host == "api.anthropic.com" {
            let addr = self.0;
            return Box::pin(async move { Ok(vec![addr]) });
        }
        PublicDns.resolve(host, port)
    }
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_test"), "PRIVATE KEY\n").unwrap();
        std::fs::write(home.join("notes.txt"), "private\n").unwrap();

        let repo = home.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "--quiet", "-m", "initial"]);
        let mine = worktree(&repo, "feat/mine");
        let other = worktree(&repo, "feat/other");

        let run = root.join("run");
        std::fs::create_dir_all(&run).unwrap();
        let socket = run.join("forged.sock");
        let bus = run.join("bus");
        let daemon = Command::new("sleep")
            .arg("300")
            .env(SECRET, "hunter2")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        udp.set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = upstream.local_addr().unwrap();
        let upstream_requests: Arc<std::sync::Mutex<Vec<String>>> = Arc::default();
        let record = upstream_requests.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            while let Ok((mut stream, _)) = upstream.accept().await {
                let record = record.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte).await {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    record
                        .lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&head).into_owned());
                    let _ = stream
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        )
                        .await;
                });
            }
        });
        let connections: Arc<std::sync::Mutex<Vec<Connection>>> = Arc::default();
        let log = connections.clone();
        let proxy = Proxy::start(ProxyConfig {
            allow: Allowlist::new(["allowed.test", "localhost"]).unwrap(),
            resolver: Arc::new(TestDns(upstream_addr)),
            log: Arc::new(move |c| log.lock().unwrap().push(c)),
            limits: Limits::default(),
            routes: vec![SecretRoute::fixed(
                forge_sandbox::secrets::known("ANTHROPIC_API_KEY").unwrap(),
                Secret::new(ROUTE_SECRET),
            )],
            upstream: Some(Arc::new(PlainUpstream)),
            local: None,
        })
        .await
        .unwrap();
        Fixture {
            proxy: Arc::new(proxy),
            connections,
            upstream_requests,
            _socket: UnixListener::bind(&socket).unwrap(),
            _bus: UnixListener::bind(&bus).unwrap(),
            tcp,
            udp,
            _dir: dir,
            root,
            home,
            repo,
            mine,
            other,
            daemon,
            socket,
            bus,
        }
    }

    fn sandbox(&self, tier: SandboxTier) -> Arc<dyn SessionSandbox> {
        let backend = forge_sandbox::system(PathBuf::from(HELPER));
        let status = backend
            .tiers()
            .into_iter()
            .find(|s| s.tier == tier)
            .expect("tier listed");
        if let Some(why) = status.unavailable {
            panic!("the {tier:?} tier is unavailable on this machine: {why}");
        }
        backend
            .prepare(Spec {
                tier,
                worktrees: vec![self.mine.clone()],
                scratch: self.root.join("data/sessions/s1"),
                home: self.home.clone(),
                read_only: vec![PathBuf::from(PROBE)],
                proxy: Some(self.proxy.clone()),
            })
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

fn worktree(repo: &Path, branch: &str) -> WorktreeMount {
    let path = repo.join(".worktrees").join(branch);
    git(
        repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            branch,
            path.to_str().unwrap(),
        ],
    );
    let gitfile = std::fs::read_to_string(path.join(".git")).unwrap();
    let admin = PathBuf::from(gitfile.trim().strip_prefix("gitdir: ").unwrap());
    WorktreeMount {
        path,
        git_common: repo.join(".git"),
        git_admin: admin,
    }
}

/// Run the probe inside the sandbox; its exit code and stderr.
async fn probe(
    sandbox: &dyn SessionSandbox,
    cwd: &Path,
    action: &str,
    target: &str,
) -> (i32, String) {
    let output = sandbox
        .command(PROBE, &[action.to_owned(), target.to_owned()], cwd, &[])
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    let code = output.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        code == 0 || code == 1,
        "probe {action} {target} did not run ({}): stderr {stderr:?}, stdout {:?}",
        describe(output.status),
        String::from_utf8_lossy(&output.stdout),
    );
    (code, stderr)
}

fn describe(status: std::process::ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit {code}"),
        (None, Some(signal)) => format!("killed by signal {signal}"),
        _ => "no status".into(),
    }
}

async fn allowed(sandbox: &dyn SessionSandbox, cwd: &Path, action: &str, target: &Path) {
    let (code, stderr) = probe(sandbox, cwd, action, target.to_str().unwrap()).await;
    assert_eq!(
        code,
        0,
        "{action} {} should work: {stderr}",
        target.display()
    );
}

/// Something a sandboxed process must not be able to do.
enum Escape {
    Read(String),
    List(String),
    /// Judged by the host file, whatever the probe saw: in Standard a path can exist inside the
    /// sandbox but map to the session's scratch dir instead.
    Write(PathBuf),
    Tcp(String),
    /// Judged by whether the datagram arrived outside.
    Udp(String),
    Unix(String),
    Signal(u32),
}

/// Every escape, in an order where earlier attempts can't make later ones look successful.
fn escapes(fx: &Fixture) -> Vec<(&'static str, Escape)> {
    use Escape::*;
    let s = |p: PathBuf| p.to_str().unwrap().to_owned();
    let mut list = vec![
        ("read an SSH key", Read(s(fx.home.join(".ssh/id_test")))),
        ("list ~/.ssh", List(s(fx.home.join(".ssh")))),
        (
            "read a file in the real home",
            Read(s(fx.home.join("notes.txt"))),
        ),
        (
            "write outside the worktree",
            Write(fx.root.join("outside.txt")),
        ),
        ("write in the real home", Write(fx.home.join(".bashrc"))),
        (
            "reach a host over TCP",
            Tcp(fx.tcp.local_addr().unwrap().to_string()),
        ),
        (
            "reach a host over UDP",
            Udp(fx.udp.local_addr().unwrap().to_string()),
        ),
        (
            "read another session's worktree",
            Read(s(fx.other.path.join("README.md"))),
        ),
        (
            "list another session's worktree",
            List(s(fx.other.path.clone())),
        ),
        (
            "write another session's git dir",
            Write(fx.other.git_admin.join("HEAD")),
        ),
        ("signal a daemon process", Signal(fx.daemon.id())),
        // Read before writing: in Standard the write lands in the scratch home.
        ("read the main checkout", Read(s(fx.repo.join("README.md")))),
        ("write the main checkout", Write(fx.repo.join("README.md"))),
        (
            "plant a git hook",
            Write(fx.repo.join(".git/hooks/pre-commit")),
        ),
        (
            "change the repo's git config",
            Write(fx.repo.join(".git/config")),
        ),
        ("connect to the daemon socket", Unix(s(fx.socket.clone()))),
        ("connect to the session bus", Unix(s(fx.bus.clone()))),
    ];
    if cfg!(target_os = "linux") {
        list.push((
            "read the daemon's environment",
            Read(format!("/proc/{}/environ", fx.daemon.id())),
        ));
    }
    // The real keyring, where Forge's secrets live, on machines that have one: the login
    // keychain file on macOS, and the session bus that reaches the Secret Service on Linux.
    let real = |var: &str, rest: &str| std::env::var_os(var).map(|v| PathBuf::from(v).join(rest));
    if cfg!(target_os = "macos")
        && let Some(keychain) = real("HOME", "Library/Keychains/login.keychain-db")
        && keychain.exists()
    {
        list.push(("read the login keychain", Read(s(keychain))));
    }
    if cfg!(target_os = "linux")
        && let Some(bus) = real("XDG_RUNTIME_DIR", "bus")
        && bus.exists()
    {
        list.push(("connect to the real session bus", Unix(s(bus))));
    }
    list
}

/// Try `escape`; whether it worked.
async fn attempt(sandbox: &dyn SessionSandbox, fx: &Fixture, escape: &Escape) -> bool {
    let cwd = &fx.mine.path;
    match escape {
        Escape::Read(p) => probe(sandbox, cwd, "read", p).await.0 == 0,
        Escape::List(p) => probe(sandbox, cwd, "list", p).await.0 == 0,
        Escape::Tcp(a) => probe(sandbox, cwd, "connect-tcp", a).await.0 == 0,
        Escape::Unix(p) => probe(sandbox, cwd, "connect-unix", p).await.0 == 0,
        Escape::Signal(pid) => probe(sandbox, cwd, "signal", &pid.to_string()).await.0 == 0,
        Escape::Write(p) => {
            let before = std::fs::read(p).ok();
            probe(sandbox, cwd, "write", p.to_str().unwrap()).await;
            std::fs::read(p).ok() != before
        }
        Escape::Udp(a) => {
            probe(sandbox, cwd, "udp", a).await;
            fx.udp.recv(&mut [0u8; 8]).is_ok()
        }
    }
}

async fn escape_suite(tier: SandboxTier) {
    let fx = Fixture::new().await;
    let sandbox = fx.sandbox(tier);
    let sb = sandbox.as_ref();
    let cwd = fx.mine.path.clone();

    // Normal work: the worktree, its git metadata, and temp files.
    allowed(sb, &cwd, "read", &cwd.join("README.md")).await;
    allowed(sb, &cwd, "write", &cwd.join("new.txt")).await;
    allowed(sb, &cwd, "write", &fx.mine.git_admin.join("forge-probe")).await;
    allowed(sb, &cwd, "read", &fx.repo.join(".git/HEAD")).await;
    allowed(sb, &cwd, "list", Path::new("/usr")).await;

    proxy_checks(sb, &fx).await;
    env_checks(sb, &fx).await;
    #[cfg(target_os = "linux")]
    assert!(
        !preload_escapes(sb, &fx).await,
        "LD_PRELOAD ran agent code outside the {tier:?} sandbox"
    );

    let mut escaped = Vec::new();
    for (name, escape) in escapes(&fx) {
        if attempt(sb, &fx, &escape).await {
            escaped.push(name);
        }
    }
    assert!(
        escaped.is_empty(),
        "escapes succeeded in {tier:?}: {escaped:?}"
    );
}

/// The egress proxy is the way out: an allowlisted host works through it, and it refuses
/// hosts off the allowlist, allowlisted names that resolve to private addresses, IP literals,
/// and other ports, logging each refusal. This holds in every tier, Unsandboxed included.
async fn proxy_checks(sb: &dyn SessionSandbox, fx: &Fixture) {
    let cwd = &fx.mine.path;
    let (code, stderr) = probe(sb, cwd, "proxy-connect", "allowed.test:443").await;
    assert_eq!(code, 0, "an allowlisted host should be reachable: {stderr}");
    let tcp = fx.tcp.local_addr().unwrap();
    let refused = [
        "blocked.test:443".to_owned(),
        "localhost:443".to_owned(),
        format!("127.0.0.1:{}", tcp.port()),
        "allowed.test:22".to_owned(),
    ];
    for target in &refused {
        let (code, _) = probe(sb, cwd, "proxy-connect", target).await;
        assert_eq!(code, 1, "the proxy let {target} through");
    }
    // The secret's route: the agent sends the placeholder, the host gets the real value.
    let (code, stderr) = probe(sb, cwd, "route-get", "ANTHROPIC_BASE_URL").await;
    assert_eq!(code, 0, "the secret's route should work: {stderr}");
    let requests = fx.upstream_requests.lock().unwrap().clone();
    let routed = requests
        .iter()
        .find(|r| r.starts_with("GET /v1/models "))
        .unwrap_or_else(|| panic!("the route's request never arrived: {requests:?}"));
    assert!(routed.contains("Host: api.anthropic.com\r\n"), "{routed}");
    assert!(
        routed.contains(&format!("x-api-key: {ROUTE_SECRET}\r\n")),
        "{routed}"
    );
    assert!(!routed.contains(PLACEHOLDER), "{routed}");
    let log = fx.connections.lock().unwrap().clone();
    for target in &refused {
        let (host, port) = target.rsplit_once(':').unwrap();
        assert!(
            log.iter()
                .any(|c| c.host == host && c.port.to_string() == port && !c.allowed),
            "{target} was not logged as denied: {log:?}"
        );
    }
}

/// Variables the sandbox sets, which a command's own environment (agent-chosen, for ACP
/// terminals) must not override.
const PINNED: &[&str] = &[
    "PATH",
    "HOME",
    "TMPDIR",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "ANTHROPIC_BASE_URL",
];

/// A command's extra environment reaches it, but never replaces what the sandbox set: an agent
/// that could set `PATH` would pick which program runs, and one that could set `HOME` or the
/// proxy variables would step outside the sandbox's view of the world. This holds in every
/// tier, Unsandboxed included.
async fn env_checks(sb: &dyn SessionSandbox, fx: &Fixture) {
    let run = async |env: &[(String, String)]| {
        let output = sb
            .command("/usr/bin/env", &[], &fx.mine.path, env)
            .stdin(Stdio::null())
            .output()
            .await
            .unwrap();
        assert!(output.status.success(), "env did not run: {output:?}");
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let vars: std::collections::HashMap<String, String> = stdout
            .lines()
            .filter_map(|l| l.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        vars
    };
    let baseline = run(&[]).await;
    for key in [
        "PATH",
        "HOME",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ANTHROPIC_BASE_URL",
    ] {
        assert!(baseline.contains_key(key), "the sandbox should set {key}");
    }
    let mut env: Vec<(String, String)> = PINNED
        .iter()
        .map(|k| ((*k).to_owned(), format!("/agent/{k}")))
        .collect();
    env.push(("FORGE_AGENT_VAR".into(), "set".into()));
    let seen = run(&env).await;
    assert_eq!(seen.get("FORGE_AGENT_VAR").map(String::as_str), Some("set"));
    for key in PINNED {
        if let Some(value) = baseline.get(*key) {
            assert_eq!(
                seen.get(*key),
                Some(value),
                "the command's environment replaced {key}"
            );
        }
    }

    // A name with `=` in it would reach the process as a second `PATH=...` entry, which
    // shells read instead of the real one.
    let sneaky = vec![("PATH=/agent/bin:/usr/bin".to_owned(), String::new())];
    let output = sb
        .command(
            "/bin/sh",
            &["-c".into(), "printf %s \"$PATH\"".into()],
            &fx.mine.path,
            &sneaky,
        )
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        baseline["PATH"],
        "a variable named PATH=... replaced PATH for the shell"
    );
}

/// Where the preloaded library's constructor tries to write: the hidden home.
#[cfg(target_os = "linux")]
fn preload_target(fx: &Fixture) -> PathBuf {
    fx.home.join("preload-escaped")
}

/// Build a shared library in the worktree whose constructor writes `target`, and return its
/// path. Needs a C compiler, as the sandbox CI hosts have.
#[cfg(target_os = "linux")]
fn preload_library(fx: &Fixture) -> PathBuf {
    let dir = &fx.mine.path;
    let source = dir.join("preload.c");
    std::fs::write(
        &source,
        format!(
            "#include <fcntl.h>\n#include <unistd.h>\n\
             __attribute__((constructor)) static void escape(void) {{\n\
             int fd = open(\"{}\", O_WRONLY | O_CREAT, 0644);\n\
             if (fd >= 0) close(fd);\n}}\n",
            preload_target(fx).display()
        ),
    )
    .unwrap();
    let library = dir.join("preload.so");
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-o"])
        .arg(&library)
        .arg(&source)
        .status()
        .expect("the preload check needs a C compiler (cc)");
    assert!(status.success(), "building the preload library failed");
    library
}

/// Whether a command run with the agent's `LD_PRELOAD` got its library to write outside the
/// sandbox. Agent variables must reach only the sandboxed command, never the launcher that
/// sets the sandbox up, where the loader would run the library before any restriction.
#[cfg(target_os = "linux")]
async fn preload_escapes(sb: &dyn SessionSandbox, fx: &Fixture) -> bool {
    let target = preload_target(fx);
    let _ = std::fs::remove_file(&target);
    let library = preload_library(fx);
    let env = vec![("LD_PRELOAD".to_owned(), library.display().to_string())];
    let _ = sb
        .command("/bin/true", &[], &fx.mine.path, &env)
        .stdin(Stdio::null())
        .output()
        .await
        .unwrap();
    let escaped = target.exists();
    let _ = std::fs::remove_file(&target);
    escaped
}

/// The suite's control: without a sandbox every escape works, so a probe that fails for some
/// other reason can't pass as a blocked escape.
#[tokio::test]
async fn every_escape_works_without_a_sandbox() {
    let fx = Fixture::new().await;
    let sandbox = fx.sandbox(SandboxTier::Unsandboxed);
    proxy_checks(sandbox.as_ref(), &fx).await;
    env_checks(sandbox.as_ref(), &fx).await;
    #[cfg(target_os = "linux")]
    assert!(
        preload_escapes(sandbox.as_ref(), &fx).await,
        "the preload library should write outside when nothing confines it"
    );
    let mut blocked = Vec::new();
    for (name, escape) in escapes(&fx) {
        if !attempt(sandbox.as_ref(), &fx, &escape).await {
            blocked.push(name);
        }
    }
    assert!(
        blocked.is_empty(),
        "these probes can't detect an escape: {blocked:?}"
    );
}

#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn light_escapes_fail() {
    escape_suite(SandboxTier::Light).await;
}

#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn standard_escapes_fail() {
    escape_suite(SandboxTier::Standard).await;
}

/// Killing the command's process group ends everything in a Standard sandbox, even a process
/// that started its own session to leave the group.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn killing_a_standard_sandbox_reaches_detached_processes() {
    let fx = Fixture::new().await;
    let sandbox = fx.sandbox(SandboxTier::Standard);
    let marker = format!("{}", 3_000_000 + std::process::id());
    let script = format!("setsid sleep {marker} & sleep {marker}");
    let mut child = sandbox
        .command("sh", &["-c".into(), script], &fx.mine.path, &[])
        .stdin(Stdio::null())
        .process_group(0)
        .spawn()
        .unwrap();
    let running = || sleepers(&marker);
    for _ in 0..100 {
        if running() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(running(), 2, "both sleeps should be running");
    let pid = rustix::process::Pid::from_raw(child.id().unwrap() as i32).unwrap();
    rustix::process::kill_process_group(pid, rustix::process::Signal::KILL).unwrap();
    child.wait().await.unwrap();
    for _ in 0..100 {
        if running() == 0 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("{} sandboxed processes outlived the kill", running());
}

/// Processes on this machine running `sleep <marker>`.
#[cfg(target_os = "linux")]
fn sleepers(marker: &str) -> usize {
    std::fs::read_dir("/proc")
        .unwrap()
        .filter_map(|e| std::fs::read(e.ok()?.path().join("cmdline")).ok())
        .filter(|cmd| {
            let args: Vec<&[u8]> = cmd.split(|b| *b == 0).filter(|a| !a.is_empty()).collect();
            args.len() == 2 && args[0].ends_with(b"sleep") && args[1] == marker.as_bytes()
        })
        .count()
}
