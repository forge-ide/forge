//! Level 1 process isolation per `docs/architecture/isolation-model.md` §8.2.
//!
//! Wraps [`tokio::process::Command`] with:
//! - Environment whitelist (no inheritance from the session daemon).
//! - CPU / address-space soft limits via `setrlimit`.
//! - A fresh process group so the entire tree can be killed on drop or
//!   session shutdown.
//!
//! Linux-only. macOS and Windows support is deferred beyond Phase 1.

use std::sync::{Arc, Mutex};

/// Resource limits applied to sandboxed children via `setrlimit(2)`.
///
/// Defaults: 30 s of CPU time, 512 MiB of address space. These are conservative
/// values intended for short-lived tool invocations; callers should override
/// via [`SandboxedCommand::with_config`] for workloads that need more.
#[derive(Debug, Clone, Copy)]
pub struct SandboxConfig {
    /// `RLIMIT_CPU` soft limit in seconds. Exceeding this sends `SIGXCPU`.
    pub cpu_seconds: u64,
    /// `RLIMIT_AS` soft limit in bytes (address space ceiling).
    pub address_space_bytes: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            cpu_seconds: 30,
            address_space_bytes: 512 * 1024 * 1024,
        }
    }
}

/// Environment variables that are always injected, regardless of the
/// caller-provided allow-list.
pub const BASE_ENV_WHITELIST: &[&str] = &["PATH", "HOME", "LANG", "LC_ALL"];

/// Shared registry of live sandboxed children scoped to a session. On session
/// shutdown, [`ChildRegistry::kill_all`] sends `SIGKILL` to every tracked
/// process group so that stray descendants do not survive the daemon.
#[derive(Default, Clone)]
pub struct ChildRegistry {
    pgids: Arc<Mutex<Vec<i32>>>,
}

impl ChildRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a pgid. The caller is expected to deregister it via
    /// [`ChildRegistry::remove`] once the child exits cleanly.
    pub fn insert(&self, pgid: i32) {
        self.pgids.lock().unwrap().push(pgid);
    }

    /// Deregister a pgid.
    pub fn remove(&self, pgid: i32) {
        let mut guard = self.pgids.lock().unwrap();
        if let Some(idx) = guard.iter().position(|p| *p == pgid) {
            guard.swap_remove(idx);
        }
    }

    /// Send `SIGKILL` to every tracked process group and clear the registry.
    #[cfg(target_os = "linux")]
    pub fn kill_all(&self) {
        let mut guard = self.pgids.lock().unwrap();
        for pgid in guard.drain(..) {
            // SAFETY: killpg is async-signal-safe and takes no Rust references.
            unsafe {
                libc::killpg(pgid, libc::SIGKILL);
            }
        }
    }

    /// Non-Linux builds do not currently sandbox — kill_all is a no-op.
    #[cfg(not(target_os = "linux"))]
    pub fn kill_all(&self) {
        self.pgids.lock().unwrap().clear();
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.pgids.lock().unwrap().len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.pgids.lock().unwrap().is_empty()
    }
}

// Linux implementation ──────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod imp {
    use super::{ChildRegistry, SandboxConfig, BASE_ENV_WHITELIST};
    use std::ffi::OsString;
    use std::io;
    use tokio::process::{Child, Command};

    /// Pre-configured sandbox wrapper around [`tokio::process::Command`].
    ///
    /// Created via [`SandboxedCommand::new`]. Mutate further (args, stdio)
    /// through [`SandboxedCommand::command_mut`], then call
    /// [`SandboxedCommand::spawn`] to produce a [`SandboxedChild`].
    pub struct SandboxedCommand {
        cmd: Command,
        config: SandboxConfig,
        registry: Option<ChildRegistry>,
    }

    impl SandboxedCommand {
        /// Build a sandboxed command for `program`, executed with
        /// `workspace_root` as the current working directory.
        ///
        /// The environment is cleared and re-populated with
        /// [`BASE_ENV_WHITELIST`]. Callers can extend the whitelist via
        /// [`SandboxedCommand::allow_env`].
        pub fn new(
            program: impl Into<OsString>,
            workspace_root: impl AsRef<std::path::Path>,
        ) -> Self {
            Self::with_config(program, workspace_root, SandboxConfig::default())
        }

        /// Same as [`SandboxedCommand::new`] with an explicit [`SandboxConfig`].
        pub fn with_config(
            program: impl Into<OsString>,
            workspace_root: impl AsRef<std::path::Path>,
            config: SandboxConfig,
        ) -> Self {
            let mut cmd = Command::new(program.into());
            cmd.current_dir(workspace_root);
            cmd.env_clear();
            // Re-inject base whitelist from the daemon's own env. Missing vars
            // are simply skipped.
            for key in BASE_ENV_WHITELIST {
                if let Ok(val) = std::env::var(key) {
                    cmd.env(key, val);
                }
            }

            // `setsid` in pre_exec gives the child a new process group (equal
            // to its pid) so a single killpg tears down the whole tree.
            // setrlimit values are captured into the closure.
            let cpu_seconds = config.cpu_seconds;
            let address_space_bytes = config.address_space_bytes;
            // SAFETY: `pre_exec` runs between fork and exec. We only call
            // async-signal-safe libc functions (`setsid`, `setrlimit`) and
            // never touch Rust allocation or locks.
            unsafe {
                cmd.pre_exec(move || {
                    if libc::setsid() == -1 {
                        return Err(io::Error::last_os_error());
                    }
                    apply_rlimit(libc::RLIMIT_CPU, cpu_seconds)?;
                    apply_rlimit(libc::RLIMIT_AS, address_space_bytes)?;
                    Ok(())
                });
            }

            Self {
                cmd,
                config,
                registry: None,
            }
        }

        /// Add a caller-scoped environment variable to the whitelist. The
        /// value is taken verbatim — callers pass explicit (key, value)
        /// pairs rather than inheriting from the daemon's environment.
        pub fn allow_env(
            &mut self,
            key: impl AsRef<std::ffi::OsStr>,
            value: impl AsRef<std::ffi::OsStr>,
        ) -> &mut Self {
            self.cmd.env(key, value);
            self
        }

        /// Bulk version of [`SandboxedCommand::allow_env`].
        pub fn allow_envs<I, K, V>(&mut self, vars: I) -> &mut Self
        where
            I: IntoIterator<Item = (K, V)>,
            K: AsRef<std::ffi::OsStr>,
            V: AsRef<std::ffi::OsStr>,
        {
            for (k, v) in vars {
                self.cmd.env(k, v);
            }
            self
        }

        /// Register spawned children with a [`ChildRegistry`] so session
        /// shutdown can kill them.
        pub fn with_registry(&mut self, registry: ChildRegistry) -> &mut Self {
            self.registry = Some(registry);
            self
        }

        /// Access the underlying [`tokio::process::Command`] for further
        /// configuration (args, stdio, etc.).
        pub fn command_mut(&mut self) -> &mut Command {
            &mut self.cmd
        }

        /// Spawn the sandboxed command.
        pub fn spawn(mut self) -> io::Result<SandboxedChild> {
            let child = self.cmd.spawn()?;
            let pid = child
                .id()
                .ok_or_else(|| io::Error::other("spawned child has no pid (already exited)"))?
                as i32;
            // `setsid` set pgid == pid.
            let pgid = pid;
            if let Some(registry) = &self.registry {
                registry.insert(pgid);
            }
            Ok(SandboxedChild {
                child: Some(child),
                pgid,
                registry: self.registry,
                _config: self.config,
                released: false,
            })
        }

        /// Returns the config that will be applied to spawned children.
        pub fn config(&self) -> SandboxConfig {
            self.config
        }
    }

    /// Handle to a running sandboxed child. Dropping the handle sends
    /// `SIGKILL` to the entire process group, cleaning up any descendants
    /// the child may have forked.
    pub struct SandboxedChild {
        child: Option<Child>,
        pgid: i32,
        registry: Option<ChildRegistry>,
        _config: SandboxConfig,
        /// When true, Drop does not send SIGKILL to the process group.
        /// Set by [`SandboxedChild::into_child`] to hand off ownership.
        released: bool,
    }

    impl SandboxedChild {
        /// Process group id (== child pid).
        pub fn pgid(&self) -> i32 {
            self.pgid
        }

        /// Borrow the underlying [`tokio::process::Child`].
        pub fn as_child_mut(&mut self) -> &mut Child {
            self.child.as_mut().expect("child not taken")
        }

        /// Consume the handle, returning the underlying [`tokio::process::Child`].
        /// Skips the Drop-based killpg so callers that `wait().await` for
        /// natural exit can avoid killing an already-reaped group.
        pub fn into_child(mut self) -> Child {
            if let Some(reg) = &self.registry {
                reg.remove(self.pgid);
            }
            self.released = true;
            self.child.take().expect("child not taken")
        }
    }

    impl Drop for SandboxedChild {
        fn drop(&mut self) {
            if self.released {
                return;
            }
            if let Some(reg) = &self.registry {
                reg.remove(self.pgid);
            }
            // SAFETY: killpg is async-signal-safe; we just send SIGKILL.
            unsafe {
                libc::killpg(self.pgid, libc::SIGKILL);
            }
        }
    }

    fn apply_rlimit(resource: libc::__rlimit_resource_t, value: u64) -> io::Result<()> {
        let lim = libc::rlimit {
            rlim_cur: value as libc::rlim_t,
            rlim_max: value as libc::rlim_t,
        };
        // SAFETY: setrlimit only mutates kernel state and reads from a stack
        // pointer. Safe to call post-fork.
        let rc = unsafe { libc::setrlimit(resource, &lim) };
        if rc == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }
}

#[cfg(target_os = "linux")]
pub use imp::{SandboxedChild, SandboxedCommand};

// Linux-only tests ──────────────────────────────────────────────────────────

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;
    use tokio::process::Command as TokioCommand;

    #[tokio::test]
    async fn env_whitelist_excludes_secret_but_keeps_path() {
        // Arrange: a secret var in the daemon env; PATH is already set.
        std::env::set_var("FORGE_TEST_SECRET_ENV", "top-secret-value");

        let tmp = tempfile::tempdir().unwrap();
        let mut sb = SandboxedCommand::new("/usr/bin/env", tmp.path());
        sb.command_mut()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = sb.spawn().expect("spawn env").into_child();
        let mut stdout = child.stdout.take().unwrap();
        let mut out = String::new();
        stdout.read_to_string(&mut out).await.unwrap();
        let status = child.wait().await.unwrap();
        assert!(status.success(), "env exited non-zero: {status:?}");

        std::env::remove_var("FORGE_TEST_SECRET_ENV");

        assert!(
            !out.contains("top-secret-value"),
            "secret leaked into child env:\n{out}"
        );
        // PATH is in the base whitelist.
        assert!(
            out.lines().any(|l| l.starts_with("PATH=")),
            "expected PATH in child env, got:\n{out}"
        );
    }

    #[tokio::test]
    async fn caller_provided_allowlist_passes_through() {
        let tmp = tempfile::tempdir().unwrap();
        let mut sb = SandboxedCommand::new("/usr/bin/env", tmp.path());
        sb.allow_env("FORGE_ALLOWED", "yes-please");
        sb.command_mut()
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut child = sb.spawn().unwrap().into_child();
        let mut stdout = child.stdout.take().unwrap();
        let mut out = String::new();
        stdout.read_to_string(&mut out).await.unwrap();
        child.wait().await.unwrap();

        assert!(
            out.lines().any(|l| l == "FORGE_ALLOWED=yes-please"),
            "expected allow-listed var in child env, got:\n{out}"
        );
    }

    #[tokio::test]
    async fn drop_kills_descendants_via_pgid() {
        // Spawn a shell that forks a long-sleeping grandchild and prints its
        // pid. We then drop the SandboxedChild and verify the grandchild is
        // dead.
        let tmp = tempfile::tempdir().unwrap();
        let mut sb = SandboxedCommand::new("/bin/sh", tmp.path());
        sb.command_mut()
            .arg("-c")
            // Double-fork so the grandchild is a sibling process group
            // member (same pgid thanks to setsid, but distinct pid).
            .arg("sleep 60 & echo $! ; wait")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());

        let mut sandboxed = sb.spawn().unwrap();
        let mut stdout = sandboxed.as_child_mut().stdout.take().unwrap();

        // Read the grandchild pid (first line).
        let mut buf = vec![0u8; 32];
        let n = stdout.read(&mut buf).await.unwrap();
        let grandchild_pid: i32 = std::str::from_utf8(&buf[..n])
            .unwrap()
            .trim()
            .parse()
            .expect("parse grandchild pid");

        // Sanity: grandchild is alive.
        assert!(
            process_alive(grandchild_pid),
            "grandchild {grandchild_pid} should be alive before drop"
        );

        drop(sandboxed);

        // Give the kernel a moment to reap.
        for _ in 0..50 {
            if !process_alive(grandchild_pid) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("grandchild {grandchild_pid} still alive after drop (pgid kill failed)");
    }

    #[tokio::test]
    async fn cpu_rlimit_terminates_busy_loop() {
        let tmp = tempfile::tempdir().unwrap();
        let config = SandboxConfig {
            cpu_seconds: 1,
            ..SandboxConfig::default()
        };
        let mut sb = SandboxedCommand::with_config("/bin/sh", tmp.path(), config);
        sb.command_mut()
            .arg("-c")
            .arg("while :; do :; done")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());

        let mut child = sb.spawn().unwrap().into_child();
        // Bound waiting — CPU budget is 1 s, so we give it 10 s of wall clock
        // to account for scheduler variance on loaded CI.
        let status = tokio::time::timeout(Duration::from_secs(10), child.wait())
            .await
            .expect("child did not exit after CPU rlimit")
            .unwrap();

        // SIGXCPU is signal 24 on Linux. Either killed by SIGXCPU directly or
        // the shell reports it; accept any signalled exit.
        use std::os::unix::process::ExitStatusExt;
        assert!(
            status.signal().is_some() || !status.success(),
            "expected busy-loop child to be terminated, got {status:?}"
        );
    }

    #[tokio::test]
    async fn registry_tracks_and_clears_children() {
        let tmp = tempfile::tempdir().unwrap();
        let registry = ChildRegistry::new();

        let mut sb = SandboxedCommand::new("/bin/sh", tmp.path());
        sb.command_mut()
            .arg("-c")
            .arg("sleep 30")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        sb.with_registry(registry.clone());

        let sandboxed = sb.spawn().unwrap();
        let pgid = sandboxed.pgid();
        assert_eq!(registry.len(), 1);
        assert!(process_alive(pgid), "child {pgid} should be alive");

        registry.kill_all();
        assert_eq!(registry.len(), 0);

        // Reap the child so process_alive returns false (killpg'd children
        // otherwise linger as zombies which kill(pid, 0) reports as alive).
        let mut child = sandboxed.into_child();
        let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .expect("child did not exit after registry.kill_all()")
            .unwrap();
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            status.signal(),
            Some(libc::SIGKILL),
            "expected SIGKILL, got {status:?}"
        );
        assert!(!process_alive(pgid));
    }

    fn process_alive(pid: i32) -> bool {
        // SAFETY: `kill(pid, 0)` only checks for existence & permission.
        let rc = unsafe { libc::kill(pid, 0) };
        if rc == 0 {
            return true;
        }
        // ESRCH = no such process → dead. EPERM = exists, we just can't signal it.
        std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }

    // Canary that a plain tokio::process::Command does NOT isolate env
    // (proves our env_clear actually does something).
    #[tokio::test]
    async fn baseline_tokio_command_inherits_env() {
        std::env::set_var("FORGE_BASELINE_SECRET", "leaked");
        let mut cmd = TokioCommand::new("/usr/bin/env");
        cmd.stdout(std::process::Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        let mut stdout = child.stdout.take().unwrap();
        let mut out = String::new();
        stdout.read_to_string(&mut out).await.unwrap();
        child.wait().await.unwrap();
        std::env::remove_var("FORGE_BASELINE_SECRET");
        assert!(out.contains("FORGE_BASELINE_SECRET=leaked"));
    }
}
