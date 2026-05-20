//! Shared session-spawn helper used by the CLI's `forge session new` path
//! (`main.rs`) and the Tauri `session_start` IPC (F-725).
//!
//! The function below is the only place that knows how to:
//! 1. allocate a fresh [`forge_core::SessionId`],
//! 2. compose the socket + pid paths,
//! 3. locate the `forged` binary,
//! 4. exec it with the right env vars and `--agent` / `--provider` flags,
//! 5. detach the child, and
//! 6. wait for the socket to appear before returning.
//!
//! Both callers pass owned `String` / `Option<String>` values — the helper
//! has no opinions on how the caller acquired them (clap, Tauri arg
//! deserialization, etc).

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::socket;

/// Result of a successful spawn: the freshly-allocated session id and the
/// UDS path the daemon is now listening on.
#[derive(Debug, Clone)]
pub struct SpawnedSession {
    pub session_id: String,
    pub socket_path: PathBuf,
}

/// Spawn a `forged` session and return its handle once the UDS appears.
///
/// `workspace` is the absolute directory `forged` will operate under. Pass
/// the current working directory when the caller has none.
///
/// `agent` defaults to [`forge_agents::FORGE_DEFAULT_AGENT_NAME`] when `None`.
/// `provider` is forwarded verbatim to `forged` via `--provider <spec>` when
/// present and otherwise omitted so the daemon picks its own default (env
/// var, then mock).
pub async fn spawn_forged_session(
    workspace: &Path,
    agent: Option<&str>,
    provider: Option<&str>,
) -> Result<SpawnedSession> {
    spawn_forged_session_with_id(workspace, agent, provider, None).await
}

/// F-748: spawn a `forged` daemon for a caller-supplied session id, or
/// allocate a fresh one when `existing_id` is `None`. Used by the
/// `session_restart` IPC to re-spawn a daemon for the SAME session id
/// after a crash so the existing event log on disk is reused via
/// `forge_session::session::Session::resume` (the daemon's main loop
/// picks the resume branch when the log file already exists).
///
/// Stale pid / socket artifacts under the resolved paths (the prior
/// daemon's leftovers) are best-effort unlinked before the spawn so the
/// new daemon's `OwnedPidFile::create` (O_EXCL) does not collide with a
/// pid file whose owner is already dead. The session window's `session_close`
/// path normally cleans these up, but a SIGKILL'd daemon never ran its
/// archive arm — F-748's restart is the secondary cleanup point.
pub async fn spawn_forged_session_with_id(
    workspace: &Path,
    agent: Option<&str>,
    provider: Option<&str>,
    existing_id: Option<&str>,
) -> Result<SpawnedSession> {
    let session_id = match existing_id {
        Some(id) => id.to_string(),
        None => forge_core::SessionId::new().to_string(),
    };
    let sock = socket::socket_path(&session_id)?;
    let pid_file = socket::pid_path(&session_id)?;

    // F-748: clear stale artifacts when reusing an id. A graceful shutdown
    // would have removed both via the daemon's drop guards; a crashed /
    // SIGKILL'd daemon leaves them behind. Best-effort — missing files
    // are the normal case for first-spawn (existing_id == None).
    //
    // F-748 review fix: do NOT unlink the pid file while its recorded
    // process is still live. The previous `pump_events` crash signal can
    // race the kernel reaping the old daemon; if we unlink + spawn
    // immediately, the new daemon's `OwnedPidFile::create` (O_EXCL) can
    // collide with the old one or two daemons can briefly hold the same
    // socket. Probe liveness; if the old pid is still alive, give it up
    // to 2s to exit, then SIGKILL it. This is the same escalation pattern
    // F-747's `session_close` runs — reusing the same `forge-cli::socket`
    // primitives that crate already exports as pub helpers.
    if existing_id.is_some() {
        reap_old_daemon_if_alive(&pid_file).await;
        let _ = tokio::fs::remove_file(&sock).await;
        let _ = tokio::fs::remove_file(&pid_file).await;
    }

    if let Some(parent) = sock.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let forged = find_forged_binary()?;
    let mut cmd = std::process::Command::new(&forged);
    cmd.env("FORGE_SESSION_ID", &session_id)
        .env("FORGE_SOCKET_PATH", sock.to_str().unwrap_or(""))
        .env("FORGE_WORKSPACE", workspace.to_str().unwrap_or(""))
        .env("FORGE_PID_FILE", pid_file.to_str().unwrap_or(""));

    let agent_name = agent.unwrap_or(forge_agents::FORGE_DEFAULT_AGENT_NAME);
    cmd.arg("--agent").arg(agent_name);
    // The daemon currently reads the active agent from the environment;
    // forward it here so per-agent memory injection actually fires when
    // the caller selected an agent (the `--agent` CLI flag is parsed for
    // future compatibility but not yet consumed by `forged`).
    cmd.env("FORGE_ACTIVE_AGENT", agent_name);
    if let Some(spec) = provider {
        cmd.arg("--provider").arg(spec);
    }

    let child = cmd.spawn()?;
    // `forged` runs detached; explicitly leak the handle so dropping does
    // not kill it. Mirrors the CLI's existing behaviour in `main.rs`.
    std::mem::forget(child);

    wait_for_socket(&sock).await?;

    Ok(SpawnedSession {
        session_id,
        socket_path: sock,
    })
}

/// Locate the `forged` binary alongside the calling executable, then in
/// `PATH`. Errors with an actionable message when neither lookup turns up
/// an existing file — previously this returned `PathBuf::from("forged")`
/// as a bare PATH fallback, so a missing daemon surfaced as a confusing
/// `No such file or directory (os error 2)` at `Command::spawn` time
/// instead of a clear "build forge-session" hint.
///
/// Honours the `FORGE_FORGED_BIN` env var as a test-only override (when
/// `current_exe` is a test harness binary in `target/debug/deps/`, the
/// sibling lookup misses; integration tests set this var to the absolute
/// path of the `forged` binary they pre-built). A non-existent override
/// is now an error — silently ignoring it would re-introduce the same
/// confusing `Command::spawn` ENOENT we're trying to eliminate.
pub fn find_forged_binary() -> Result<PathBuf> {
    let mut tried: Vec<PathBuf> = Vec::new();

    if let Ok(p) = std::env::var("FORGE_FORGED_BIN") {
        if !p.is_empty() {
            let path = PathBuf::from(&p);
            if path.exists() {
                return Ok(path);
            }
            anyhow::bail!(
                "FORGE_FORGED_BIN points at {p:?} but no file exists there. \
                 Unset the variable or build the daemon with \
                 `cargo build -p forge-session`."
            );
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("forged");
            if sibling.exists() {
                return Ok(sibling);
            }
            tried.push(sibling);

            // Tests live in `target/<profile>/deps/<test>-<hash>` while
            // `forged` sits at `target/<profile>/forged`. Try one level
            // up so the integration test harness finds it without
            // setting `FORGE_FORGED_BIN` (F-748).
            if let Some(parent) = dir.parent() {
                let up = parent.join("forged");
                if up.exists() {
                    return Ok(up);
                }
                tried.push(up);
            }
        }
    }

    if let Some(on_path) = lookup_on_path("forged") {
        return Ok(on_path);
    }

    let tried_lines = tried
        .iter()
        .map(|p| format!("    {}", p.display()))
        .collect::<Vec<_>>()
        .join("\n");
    anyhow::bail!(
        "forged daemon binary not found. Build it with `cargo build -p forge-session` \
         (debug) or `just build-release` (release). Looked at:\n\
         {tried_lines}\n    PATH"
    )
}

/// Probe `$PATH` for `name`. Returns the first directory entry that
/// contains a file named `name`. Used as the final fallback before
/// `find_forged_binary` declares the daemon missing — `Command::new(name)`
/// would do the same PATH walk implicitly at spawn time, but we want to
/// know NOW so the surfaced error can list every place we looked.
fn lookup_on_path(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Poll for the UDS to appear (50 ms × 100 = 5 s) before declaring the
/// spawn timed out. Matches the historical CLI behaviour.
async fn wait_for_socket(path: &Path) -> Result<()> {
    for _ in 0..100 {
        if path.exists() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!("timed out waiting for socket at {}", path.display())
}

/// F-748 review fix: if `pid_file` still names a live process, wait for it
/// to exit before letting the caller unlink the pid+socket artifacts.
///
/// Sequence:
/// 1. If the pid file is missing or unparseable → no-op (no live owner).
/// 2. If `kill(pid, 0)` reports ESRCH → daemon already exited, no-op.
/// 3. Otherwise poll every 100ms for up to 2s, waiting for the process
///    to disappear.
/// 4. If still alive after 2s, send SIGKILL through a pidfd (race-free
///    against PID reuse) and wait briefly for reap.
///
/// Best-effort: this function never returns an error. A locked /proc, a
/// permission-denied signal, or a transient FS hiccup all collapse to
/// "proceed with the unlink + respawn" — the worst outcome is the new
/// daemon's `OwnedPidFile::create(O_EXCL)` failing, which surfaces a
/// clear error to the caller anyway.
async fn reap_old_daemon_if_alive(pid_file: &Path) {
    let Ok(raw) = tokio::fs::read_to_string(pid_file).await else {
        return;
    };
    let Ok((pid, _start_time)) = socket::parse_pid_file_record(&raw) else {
        return;
    };

    if !is_pid_alive(pid) {
        return;
    }

    // Poll for natural exit. The bridge's crash signal often outruns the
    // kernel reap; 2s is generous on Linux where reap is microseconds
    // once the parent reads `waitpid`. Detached daemons reparent to
    // PID 1, which reaps promptly.
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if !is_pid_alive(pid) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Still alive — escalate to SIGKILL via pidfd so we cannot signal a
    // recycled PID. Errors here are tolerated: the worst case is the
    // upcoming O_EXCL pid-file create failing, which the caller surfaces.
    #[cfg(target_os = "linux")]
    {
        if let Ok(()) = socket::pidfd_send_signal_for_pid(pid, libc::SIGKILL) {
            // Brief wait for reap so the new daemon doesn't immediately
            // collide on the pid file. We intentionally don't loop
            // forever here — at this point we've SIGKILL'd; the kernel
            // will reap.
            for _ in 0..20 {
                if !is_pid_alive(pid) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

/// Probe whether `pid` names a live process via `kill(pid, 0)`. Returns
/// `false` on ESRCH (process is gone), `true` for every other outcome
/// (live + signalable OR permission-denied — both treated as live for
/// the purpose of the reap wait, since an EPERM means *something* still
/// owns that pid). Identical policy to forge-shell's
/// `session_close_ipc::is_pid_alive` — factored to socket-level helpers
/// in forge-cli's own `socket` module would create a cycle; an inline
/// `kill(pid, 0)` is the smallest possible reuse footprint.
fn is_pid_alive(pid: libc::pid_t) -> bool {
    // SAFETY: `libc::kill` is FFI; arguments are simple integers; signal
    // 0 means "no signal, just probe".
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    let err = std::io::Error::last_os_error();
    !matches!(err.raw_os_error(), Some(libc::ESRCH))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Process-wide lock for tests that mutate FORGE_FORGED_BIN / PATH.
    /// Mirrors the pattern in `skill.rs` — std env mutation is racy under
    /// the default parallel test runner.
    fn env_lock() -> &'static std::sync::Mutex<()> {
        use std::sync::OnceLock;
        static LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    /// RAII helper: snapshots a set of env vars on construction, restores
    /// them on drop. Lets each test mutate freely without leaking state
    /// into sibling tests in the same process.
    struct EnvSnapshot {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }
    impl EnvSnapshot {
        fn new(keys: &[&'static str]) -> Self {
            Self {
                saved: keys
                    .iter()
                    .map(|k| (*k, std::env::var_os(k)))
                    .collect(),
            }
        }
    }
    impl Drop for EnvSnapshot {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                // SAFETY: env mutation is unsafe in the 2024 edition but
                // the surrounding harness already serializes via `env_lock`.
                unsafe {
                    match v {
                        Some(val) => std::env::set_var(k, val),
                        None => std::env::remove_var(k),
                    }
                }
            }
        }
    }

    fn write_executable(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\nexit 0\n").expect("write fake binary");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    }

    #[test]
    fn forge_forged_bin_returns_the_override_when_path_exists() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _snap = EnvSnapshot::new(&["FORGE_FORGED_BIN", "PATH"]);

        let tmp = tempfile::tempdir().unwrap();
        let bin = write_executable(tmp.path(), "forged");
        // SAFETY: serialized via env_lock.
        unsafe { std::env::set_var("FORGE_FORGED_BIN", &bin) };

        let resolved = find_forged_binary().expect("override should resolve");
        assert_eq!(resolved, bin);
    }

    #[test]
    fn forge_forged_bin_with_missing_path_errors_with_actionable_message() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _snap = EnvSnapshot::new(&["FORGE_FORGED_BIN", "PATH"]);

        // SAFETY: serialized via env_lock.
        unsafe { std::env::set_var("FORGE_FORGED_BIN", "/nonexistent/forge-test-binary") };

        let err = find_forged_binary().expect_err("missing override must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("FORGE_FORGED_BIN"),
            "error should name the env var that pointed nowhere: {msg}"
        );
        assert!(
            msg.contains("cargo build -p forge-session"),
            "error should include the actionable build command: {msg}"
        );
    }

    #[test]
    fn errors_with_candidate_paths_and_build_command_when_not_found() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _snap = EnvSnapshot::new(&["FORGE_FORGED_BIN", "PATH"]);

        // PATH points at an empty dir → sibling+parent paths don't exist
        // either (test harness layout has no `forged` next to it on a
        // fresh checkout), so the function must produce its actionable
        // error rather than the bare ENOENT it used to.
        let empty = tempfile::tempdir().unwrap();
        // SAFETY: serialized via env_lock.
        unsafe {
            std::env::remove_var("FORGE_FORGED_BIN");
            std::env::set_var("PATH", empty.path());
        }

        // The sibling lookup uses current_exe (the test runner). On a
        // workstation that ran `cargo build -p forge-session` recently,
        // `target/debug/forged` exists and the function succeeds —
        // assert the outcome only when the lookup fails. The build
        // command + "not found" wording is the regression target.
        match find_forged_binary() {
            Ok(_) => {
                // forged was already built into target/debug — skip the
                // negative assertion. The other tests cover the override
                // and PATH branches independently.
            }
            Err(err) => {
                let msg = format!("{err}");
                assert!(
                    msg.contains("forged daemon binary not found"),
                    "error should call out the missing daemon: {msg}"
                );
                assert!(
                    msg.contains("cargo build -p forge-session"),
                    "error should include the actionable build command: {msg}"
                );
                assert!(
                    msg.contains("PATH"),
                    "error should list PATH among the places we looked: {msg}"
                );
            }
        }
    }

    #[test]
    fn lookup_on_path_skips_empty_path_segments() {
        let _guard = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let _snap = EnvSnapshot::new(&["PATH"]);

        // Empty segments in PATH conventionally mean "current directory"
        // — we intentionally skip them so `find_forged_binary` never
        // returns a relative path that depends on `pwd`. Sandwich an
        // empty segment between a non-match and a real match to verify
        // the iteration doesn't bail on the empty entry.
        let tmp = tempfile::tempdir().unwrap();
        let _expected = write_executable(tmp.path(), "forged");
        let path_var = format!(
            "{}:{}:{}",
            "/this/does/not/exist",
            "",
            tmp.path().display()
        );
        // SAFETY: serialized via env_lock.
        unsafe { std::env::set_var("PATH", &path_var) };

        let found = lookup_on_path("forged").expect("should find through empty segment");
        assert!(found.is_file());
    }
}
