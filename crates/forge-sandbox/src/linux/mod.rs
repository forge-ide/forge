//! Linux backends: Light (Landlock and seccomp) and Standard (bubblewrap).

pub mod landlock;
pub mod seccomp;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use forge_api::model::SandboxTier;
use tokio::process::Command;

use crate::policy::{Forward, Mode, Policy};
use crate::proxy::Proxy;
use crate::{
    Sandbox, SandboxError, SessionSandbox, Spec, TierStatus, Unsandboxed, View, not_implemented,
    require,
};

/// Where a Standard sandbox sees the egress proxy's socket, and the loopback port the helper
/// forwards to it inside the network namespace.
const PROXY_SOCKET: &str = "/run/forge-proxy.sock";
const PROXY_PORT: u16 = 3128;

/// Devices every sandboxed process may read and write.
const DEVICES_RW: &[&str] = &["/dev/null", "/dev/zero", "/dev/full"];
/// Devices every sandboxed process may read.
const DEVICES_RO: &[&str] = &["/dev/urandom", "/dev/random"];

/// Which CPUs exist, which Standard shows read-only; not the rest of the CPU tree (which
/// mitigations are on, frequencies).
const CPU_INFO: &[&str] = &[
    "/sys/devices/system/cpu/possible",
    "/sys/devices/system/cpu/online",
    "/sys/devices/system/cpu/present",
];

/// The Linux backends, with availability probed once.
pub struct Linux {
    helper: PathBuf,
    bwrap: Option<PathBuf>,
    statuses: Vec<TierStatus>,
}

impl Linux {
    /// Probe each tier. Blocking.
    pub fn probe(helper: PathBuf) -> Self {
        let bwrap = find_in_path("bwrap");
        let statuses = vec![
            probe_light(&helper),
            probe_standard(&helper, bwrap.as_deref()),
            not_implemented(SandboxTier::Container),
            TierStatus::available(SandboxTier::Unsandboxed),
        ];
        for status in &statuses {
            if let Some(why) = &status.unavailable {
                tracing::info!(tier = crate::tier_name(status.tier), %why, "sandbox tier unavailable");
            }
        }
        Self {
            helper,
            bwrap,
            statuses,
        }
    }
}

impl Sandbox for Linux {
    fn tiers(&self) -> Vec<TierStatus> {
        self.statuses.clone()
    }

    fn prepare(&self, spec: Spec) -> Result<Arc<dyn SessionSandbox>, SandboxError> {
        require(&self.statuses, spec.tier)?;
        match spec.tier {
            SandboxTier::Unsandboxed => Ok(Arc::new(Unsandboxed::with_proxy(spec.proxy))),
            SandboxTier::Light => {
                let view = View::new(&spec)?;
                crate::view::refuse_locked(&view, "Light")?;
                Ok(Arc::new(Light::new(self.helper.clone(), &view, spec.proxy)))
            }
            SandboxTier::Standard => {
                let view = View::new(&spec)?;
                let socket = match &spec.proxy {
                    Some(proxy) => {
                        let path = spec.scratch.join("proxy.sock");
                        proxy
                            .serve_unix(&path)
                            .map_err(|e| SandboxError::io("starting the egress proxy socket", e))?;
                        Some(path)
                    }
                    None => None,
                };
                let bwrap = self.bwrap.clone().expect("probed");
                Ok(Arc::new(Bwrap::new(
                    bwrap,
                    self.helper.clone(),
                    &view,
                    socket.as_deref(),
                    spec.proxy,
                )))
            }
            SandboxTier::Container => unreachable!("never available"),
        }
    }
}

/// Light: the helper applies Landlock and seccomp to itself, then execs the command.
struct Light {
    helper: PathBuf,
    policy: String,
    env: Vec<(String, String)>,
    _proxy: Option<Arc<Proxy>>,
}

impl Light {
    fn new(helper: PathBuf, view: &View, proxy: Option<Arc<Proxy>>) -> Self {
        let mut read = view.read_only();
        read.extend(["/proc", "/sys"].iter().map(PathBuf::from));
        read.extend(DEVICES_RO.iter().map(PathBuf::from));
        read.push(helper.clone());
        let mut write = view.read_write();
        write.extend(DEVICES_RW.iter().map(PathBuf::from));
        // Landlock can only filter TCP by port, so the proxy's port is reachable on any
        // address; DNS and every other port and socket type are not.
        let connect_ports = proxy.iter().map(|p| p.port()).collect();
        let policy = Policy {
            mode: Mode::Light,
            read,
            write,
            connect_ports,
            forward: None,
        };
        let mut env = crate::env::from_daemon_in(view);
        env.push(("HOME".into(), view.scratch_home.display().to_string()));
        env.push(("TMPDIR".into(), view.scratch_tmp.display().to_string()));
        env.extend(crate::env::startup_files_off());
        env.extend(view.toolchain_env.iter().cloned());
        if let Some(proxy) = &proxy {
            env.extend(proxy.env(&format!("http://127.0.0.1:{}", proxy.port())));
        }
        Self {
            helper,
            policy: policy.to_json(),
            env,
            _proxy: proxy,
        }
    }
}

impl SessionSandbox for Light {
    fn tier(&self) -> SandboxTier {
        SandboxTier::Light
    }

    fn var(&self, name: &str) -> Option<String> {
        crate::env::lookup(&self.env, name)
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> Command {
        let mut cmd = Command::new(&self.helper);
        cmd.arg("light")
            .arg(&self.policy)
            .args(crate::helper::env_args(&crate::env::extra(&self.env, env)))
            .arg("--")
            .arg(program)
            .args(args)
            .current_dir(cwd)
            // The helper gets only the sandbox's variables; the command's own are applied
            // after the restrictions are.
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

/// Standard: bubblewrap builds the filesystem view in fresh namespaces (no network but
/// loopback, its own PID space), and the helper adds the seccomp filter inside.
struct Bwrap {
    bwrap: PathBuf,
    helper: PathBuf,
    args: Vec<String>,
    policy: String,
    env: Vec<(String, String)>,
    _proxy: Option<Arc<Proxy>>,
}

impl Bwrap {
    /// `socket` is the egress proxy's Unix socket, bound into the sandbox when there is one.
    fn new(
        bwrap: PathBuf,
        helper: PathBuf,
        view: &View,
        socket: Option<&Path>,
        proxy: Option<Arc<Proxy>>,
    ) -> Self {
        let mut args = bwrap_args(view, &helper);
        let forward = socket.map(|socket| {
            bind(&mut args, "--bind", socket, Path::new(PROXY_SOCKET));
            Forward {
                port: PROXY_PORT,
                socket: PROXY_SOCKET.into(),
            }
        });
        let mut env = crate::env::from_daemon_in(view);
        env.push(("HOME".into(), view.home.display().to_string()));
        env.push(("TMPDIR".into(), "/tmp".into()));
        env.extend(crate::env::startup_files_off());
        env.extend(view.toolchain_env.iter().cloned());
        if let (Some(_), Some(proxy)) = (&forward, &proxy) {
            env.extend(proxy.env(&format!("http://127.0.0.1:{PROXY_PORT}")));
        }
        let policy = Policy {
            mode: Mode::Standard,
            read: vec![],
            write: vec![],
            connect_ports: vec![],
            forward,
        };
        Self {
            bwrap,
            helper,
            args,
            policy: policy.to_json(),
            env,
            _proxy: proxy,
        }
    }
}

impl SessionSandbox for Bwrap {
    fn tier(&self) -> SandboxTier {
        SandboxTier::Standard
    }

    fn var(&self, name: &str) -> Option<String> {
        crate::env::lookup(&self.env, name)
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> Command {
        let mut cmd = Command::new(&self.bwrap);
        cmd.args(&self.args)
            .arg("--chdir")
            .arg(cwd)
            .arg("--")
            .arg(&self.helper)
            .arg("standard")
            .arg(&self.policy)
            .args(crate::helper::env_args(&crate::env::extra(&self.env, env)))
            .arg("--")
            .arg(program)
            .args(args)
            // bubblewrap and the helper inside it get only the sandbox's variables; the
            // command's own are applied after the restrictions are.
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

/// bubblewrap's arguments for `view`, before `--chdir`. Later mounts cover earlier ones, so the
/// scratch home goes over the real home first and everything visible inside it comes after.
fn bwrap_args(view: &View, helper: &Path) -> Vec<String> {
    let mut args: Vec<String> = [
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--unshare-cgroup-try",
        "--hostname",
        "forge-sandbox",
    ]
    .map(str::to_owned)
    .to_vec();
    for path in &view.system {
        bind(&mut args, "--ro-bind", path, path);
    }
    args.extend(["--proc", "/proc", "--dev", "/dev"].map(str::to_owned));
    // Which CPUs exist, read-only: allocators such as tcmalloc refuse to start without
    // `/sys/devices/system/cpu/possible`. Nothing else of /sys is shown.
    for file in CPU_INFO {
        let path = Path::new(file);
        if path.is_file() {
            bind(&mut args, "--ro-bind", path, path);
        }
    }
    bind(&mut args, "--bind", &view.scratch_tmp, Path::new("/tmp"));
    bind(&mut args, "--bind", &view.scratch_home, &view.home);
    // The home's writable dirs, as mounts of their own, so they stay writable when the rest
    // of the home is made read-only at the end.
    for dir in crate::HOME_WRITABLE {
        bind(
            &mut args,
            "--bind",
            &view.scratch_home.join(dir),
            &view.home.join(dir),
        );
    }
    for path in view.toolchains.iter().chain(&view.extra) {
        bind(&mut args, "--ro-bind", path, path);
    }
    for w in &view.worktrees {
        bind(&mut args, "--ro-bind", &w.git_common, &w.git_common);
        bind(&mut args, "--bind", &w.git_admin, &w.git_admin);
        bind(&mut args, "--bind", &w.path, &w.path);
    }
    // After the home, so one inside it shows through at its real path.
    for path in &view.extra_rw {
        bind(&mut args, "--bind", path, path);
    }
    // Then what must stay read-only inside them, mounted over.
    for path in view.locked() {
        bind(&mut args, "--ro-bind", &path, &path);
    }
    for lock in &view.home_locked {
        bind(
            &mut args,
            "--ro-bind",
            &view.scratch_home.join(&lock.path),
            &view.home.join(&lock.path),
        );
    }
    bind(&mut args, "--ro-bind", helper, helper);
    // Last, once bubblewrap has made every mount point it needs in the home: the home itself
    // read-only. The remount isn't recursive, so the mounts inside it keep theirs.
    args.push("--remount-ro".to_owned());
    args.push(view.home.display().to_string());
    args
}

fn bind(args: &mut Vec<String>, flag: &str, src: &Path, dst: &Path) {
    args.push(flag.to_owned());
    args.push(src.display().to_string());
    args.push(dst.display().to_string());
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn probe_light(helper: &Path) -> TierStatus {
    let tier = SandboxTier::Light;
    if !helper.is_file() {
        return missing_helper(tier, helper);
    }
    match run(std::process::Command::new(helper).arg("check-light")) {
        Ok(()) => TierStatus::available(tier),
        Err(stderr) => TierStatus::unavailable(
            tier,
            format!("Landlock and seccomp could not be applied ({stderr})"),
            "the light tier needs Landlock ABI 6 or newer (Linux 6.12+) enabled at boot (\"landlock\" in the kernel's lsm= list); use the standard tier instead",
        ),
    }
}

fn probe_standard(helper: &Path, bwrap: Option<&Path>) -> TierStatus {
    let tier = SandboxTier::Standard;
    if !helper.is_file() {
        return missing_helper(tier, helper);
    }
    let Some(bwrap) = bwrap else {
        return TierStatus::unavailable(
            tier,
            "bubblewrap (bwrap) is not installed",
            "install the bubblewrap package (for example `sudo apt install bubblewrap`), then restart forged",
        );
    };
    let probe = run(std::process::Command::new(bwrap).args([
        "--die-with-parent",
        "--unshare-user",
        "--unshare-pid",
        "--unshare-net",
        "--unshare-ipc",
        "--ro-bind",
        "/",
        "/",
        "--",
        "/bin/sh",
        "-c",
        "exit 0",
    ]));
    match probe {
        Ok(()) => TierStatus::available(tier),
        Err(stderr) => TierStatus::unavailable(
            tier,
            format!("bubblewrap could not create its namespaces ({stderr})"),
            userns_fix(|p| std::fs::read_to_string(p).ok()),
        ),
    }
}

fn missing_helper(tier: SandboxTier, helper: &Path) -> TierStatus {
    TierStatus::unavailable(
        tier,
        format!("the sandbox helper {} is missing", helper.display()),
        format!(
            "reinstall Forge; forged expects {} next to its own executable",
            crate::HELPER
        ),
    )
}

/// What to do when unprivileged user namespaces are blocked, from the sysctls that block them.
fn userns_fix(read: impl Fn(&str) -> Option<String>) -> String {
    let is = |path: &str, value: &str| read(path).is_some_and(|v| v.trim() == value);
    if is(
        "/proc/sys/kernel/apparmor_restrict_unprivileged_userns",
        "1",
    ) {
        "AppArmor restricts unprivileged user namespaces: run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` (persist it in /etc/sysctl.d/), or install an AppArmor profile that lets bwrap use them".into()
    } else if is("/proc/sys/kernel/unprivileged_userns_clone", "0") {
        "unprivileged user namespaces are disabled: run `sudo sysctl -w kernel.unprivileged_userns_clone=1` (persist it in /etc/sysctl.d/)".into()
    } else if is("/proc/sys/user/max_user_namespaces", "0") {
        "user namespaces are disabled: run `sudo sysctl -w user.max_user_namespaces=15000` (persist it in /etc/sysctl.d/)".into()
    } else {
        "enable unprivileged user namespaces for bubblewrap, or use the light tier".into()
    }
}

/// Run a probe to completion; its trimmed stderr on failure.
fn run(cmd: &mut std::process::Command) -> Result<(), String> {
    let output = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .env_clear()
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(if stderr.is_empty() {
        format!("exited with {}", output.status)
    } else {
        stderr
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorktreeMount;

    fn view() -> View {
        View {
            system: vec!["/usr".into()],
            toolchains: vec!["/home/u/.cargo".into()],
            toolchain_env: vec![],
            extra: vec![],
            extra_rw: vec![],
            worktrees: vec![WorktreeMount {
                path: "/home/u/repo/.worktrees/feat/x".into(),
                git_common: "/home/u/repo/.git".into(),
                git_admin: "/home/u/repo/.git/worktrees/x".into(),
            }],
            home: "/home/u".into(),
            scratch_home: "/data/sessions/s/home".into(),
            scratch_tmp: "/data/sessions/s/tmp".into(),
            home_locked: vec![crate::HomeLock {
                path: ".claude/jobs".into(),
                dir: true,
            }],
        }
    }

    fn position(args: &[String], window: &[&str]) -> usize {
        args.windows(window.len())
            .position(|w| w.iter().zip(window).all(|(a, b)| a == b))
            .unwrap_or_else(|| panic!("{window:?} not in {args:?}"))
    }

    #[test]
    fn bwrap_hides_home_and_the_main_checkout() {
        let args = bwrap_args(&view(), Path::new("/opt/forge/forge-sandbox-exec"));
        for flag in [
            "--unshare-user",
            "--unshare-pid",
            "--unshare-net",
            "--die-with-parent",
        ] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        let home = position(&args, &["--bind", "/data/sessions/s/home", "/home/u"]);
        let cargo = position(&args, &["--ro-bind", "/home/u/.cargo", "/home/u/.cargo"]);
        let git = position(
            &args,
            &["--ro-bind", "/home/u/repo/.git", "/home/u/repo/.git"],
        );
        let admin = position(
            &args,
            &[
                "--bind",
                "/home/u/repo/.git/worktrees/x",
                "/home/u/repo/.git/worktrees/x",
            ],
        );
        let wt = position(
            &args,
            &[
                "--bind",
                "/home/u/repo/.worktrees/feat/x",
                "/home/u/repo/.worktrees/feat/x",
            ],
        );
        // The scratch home goes first, so what is visible inside home covers it.
        assert!(home < cargo && cargo < git && git < admin && admin < wt);
        // Its writable dirs are mounts of their own, before what shows inside them; a locked
        // path inside one is mounted over it; and then the home is made read-only.
        let cache = position(
            &args,
            &["--bind", "/data/sessions/s/home/.cache", "/home/u/.cache"],
        );
        let jobs = position(
            &args,
            &[
                "--ro-bind",
                "/data/sessions/s/home/.claude/jobs",
                "/home/u/.claude/jobs",
            ],
        );
        let ro = position(&args, &["--remount-ro", "/home/u"]);
        assert!(home < cache && cache < cargo && wt < jobs && jobs < ro);
        assert_eq!(ro, args.len() - 2, "the remount comes last");
        // The main checkout and the real home are never bound.
        assert!(!args.iter().any(|a| a == "/home/u/repo" || a == "/"));
    }

    #[test]
    fn userns_fixes_name_the_blocking_setting() {
        let apparmor = userns_fix(|p| {
            (p == "/proc/sys/kernel/apparmor_restrict_unprivileged_userns").then(|| "1\n".into())
        });
        assert!(
            apparmor.contains("apparmor_restrict_unprivileged_userns=0"),
            "{apparmor}"
        );
        let debian =
            userns_fix(|p| (p == "/proc/sys/kernel/unprivileged_userns_clone").then(|| "0".into()));
        assert!(debian.contains("unprivileged_userns_clone=1"), "{debian}");
        let max = userns_fix(|p| (p == "/proc/sys/user/max_user_namespaces").then(|| "0".into()));
        assert!(max.contains("max_user_namespaces"), "{max}");
        assert!(userns_fix(|_| None).contains("user namespaces"));
    }

    #[test]
    fn a_missing_helper_makes_sandboxed_tiers_unavailable() {
        let linux = Linux::probe("/no/such/forge-sandbox-exec".into());
        for status in linux.tiers() {
            match status.tier {
                SandboxTier::Unsandboxed => assert!(status.is_available()),
                _ => assert!(!status.is_available(), "{:?}", status.tier),
            }
        }
    }
}
