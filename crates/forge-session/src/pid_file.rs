//! `forged`'s pid-file lifecycle (F-049, cross-platform via F-338).
//!
//! Persistent-mode `forged` owns the pid file pointed at by
//! `$FORGE_PID_FILE`. It writes `"<pid>\n<start_time>\n"` atomically via
//! `O_EXCL` (so a leftover file from a crashed prior run cannot be
//! silently clobbered), and removes the file on clean shutdown.
//!
//! The start-time field is an opaque, platform-dependent `u64` produced
//! by [`crate::starttime::read_self_starttime`]; `forge-cli`'s
//! `session_kill` uses it to detect kernel PID reuse before signalling.
//! See `forge_cli::socket::{parse_pid_file_record, kill_session_from_pid_file}`.
//!
//! Ephemeral `forged` does not write a pid file — it's spawned by
//! `forge run agent` whose wait is synchronous, so there is no external
//! caller that would need to locate its pid.

use crate::starttime::read_self_starttime;
use anyhow::{Context, Result};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// Handle to a pid file this process owns. Removes the file on drop so a
/// panic or unexpected exit still cleans up.
#[derive(Debug)]
pub struct OwnedPidFile {
    path: PathBuf,
}

impl OwnedPidFile {
    /// Atomically create the pid file with `O_EXCL`, writing this
    /// process's pid and start-time. Fails if the file already exists.
    pub fn create(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let pid = std::process::id() as libc::pid_t;
        let start_time = read_self_starttime()
            .context("failed to read process start-time for pid-file record")?;
        let contents = format!("{pid}\n{start_time}\n");

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create pid-file parent dir {}", parent.display()))?;
        }

        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .with_context(|| {
                format!(
                    "refusing to overwrite existing pid file {} (O_EXCL); \
                     a prior forged may still be running or crashed without cleanup",
                    path.display()
                )
            })?;
        f.write_all(contents.as_bytes())
            .with_context(|| format!("write pid file {}", path.display()))?;
        f.sync_all().ok();
        Ok(Self { path })
    }

    /// Absolute path of the pid file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for OwnedPidFile {
    fn drop(&mut self) {
        // Best-effort unlink. If the file was already removed externally
        // that's fine — we don't want to mask the real shutdown error.
        let _ = std::fs::remove_file(&self.path);
    }
}

// Unit tests exercise the pid-file lifecycle (create / O_EXCL refusal /
// drop-removes-file). They are platform-agnostic now that the start-time
// probe lives behind `crate::starttime::read_self_starttime` (F-338), so
// the unit-gate introduced by #333 (`target_os = "linux"`-only) has been
// lifted. The parser-level tests that used to live here moved into
// `crate::starttime::linux::tests` because only the Linux probe parses
// text — macOS/Windows go through FFI.
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn create_writes_two_line_record() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("session.pid");
        let _owned = OwnedPidFile::create(&p).expect("create");
        let raw = std::fs::read_to_string(&p).expect("read");
        let mut lines = raw.lines();
        let pid_line = lines.next().expect("pid");
        let st_line = lines.next().expect("starttime");
        let pid: libc::pid_t = pid_line.parse().expect("pid int");
        assert_eq!(pid, std::process::id() as libc::pid_t);
        let _: u64 = st_line.parse().expect("starttime int");
    }

    #[test]
    fn create_refuses_when_file_exists() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("session.pid");
        std::fs::write(&p, "0\n0\n").unwrap();
        let err = OwnedPidFile::create(&p).expect_err("must refuse existing file");
        assert!(
            err.to_string().to_lowercase().contains("pid file")
                || err.to_string().to_lowercase().contains("exists"),
            "expected O_EXCL refusal, got: {err}"
        );
    }

    #[test]
    fn drop_removes_file() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("session.pid");
        {
            let _owned = OwnedPidFile::create(&p).expect("create");
            assert!(p.exists());
        }
        assert!(!p.exists(), "pid file must be removed on drop");
    }

    /// F-338: macOS-specific end-to-end coverage of the pid-file
    /// lifecycle on the cross-platform starttime path. The generic
    /// lifecycle tests above already run on macOS now (the unit gate
    /// from #333 is lifted), but this test pins the macOS-specific DoD
    /// item: "returns a usable record on macOS". Asserts the recorded
    /// start-time is a plausible microseconds-since-epoch value from
    /// libproc (non-zero, > 10^15 so it can't collide with a Linux
    /// clock-ticks value that's always < 10^12 even after a century
    /// uptime). Keeps the invariant readable for future maintainers.
    #[cfg(target_os = "macos")]
    #[test]
    fn create_records_macos_style_starttime() {
        let td = TempDir::new().unwrap();
        let p = td.path().join("session.pid");
        let _owned = OwnedPidFile::create(&p).expect("create");
        let raw = std::fs::read_to_string(&p).expect("read");
        let mut lines = raw.lines();
        let _pid = lines.next().expect("pid");
        let st: u64 = lines
            .next()
            .expect("starttime")
            .parse()
            .expect("starttime int");
        // libproc returns microseconds-since-epoch. At 2026-04-21 the
        // value is ~1.77e15 (any live process). 10^15 is the smallest
        // value that cleanly falls outside the Linux /proc/self/stat
        // clock-ticks range (< 10^12 for any realistic uptime × HZ),
        // so this threshold documents the platform-shape difference.
        assert!(
            st > 1_000_000_000_000_000,
            "macOS start-time is microseconds since epoch; got implausibly small value {st}"
        );
    }
}
