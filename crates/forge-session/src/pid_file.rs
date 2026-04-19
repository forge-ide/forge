//! `forged`'s pid-file lifecycle (F-049).
//!
//! Persistent-mode `forged` owns the pid file pointed at by
//! `$FORGE_PID_FILE`. It writes `"<pid>\n<start_time>\n"` atomically via
//! `O_EXCL` (so a leftover file from a crashed prior run cannot be
//! silently clobbered), and removes the file on clean shutdown.
//!
//! The start-time field is `/proc/self/stat` field 22, which `forge-cli`'s
//! `session_kill` uses to detect kernel PID reuse before signalling.
//! See `forge_cli::socket::{parse_pid_file_record, kill_session_from_pid_file}`.
//!
//! Ephemeral `forged` does not write a pid file — it's spawned by
//! `forge run agent` whose wait is synchronous, so there is no external
//! caller that would need to locate its pid.

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
        let start_time =
            read_self_starttime().context("failed to read /proc/self/stat for pid-file record")?;
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

/// Read `/proc/self/stat` and return field 22 (start-time in clock ticks
/// since boot). Separate from `forge_cli::socket::read_process_starttime`
/// because that helper targets an arbitrary pid; `forged` can take the
/// fast path of `/proc/self/stat` and avoid re-parsing its own pid.
fn read_self_starttime() -> Result<u64> {
    let contents = std::fs::read_to_string("/proc/self/stat").context("read /proc/self/stat")?;
    parse_proc_stat_starttime(&contents)
}

/// Extract field 22 (`starttime`) from a `/proc/<pid>/stat` line.
///
/// Mirrors `forge_cli::socket::parse_proc_stat_starttime`. Duplicated
/// here to avoid a forge-cli -> forge-session dep in production code;
/// the two implementations are tested against each other in the
/// `pid_file_lifecycle` integration test.
fn parse_proc_stat_starttime(line: &str) -> Result<u64> {
    let close = line
        .rfind(')')
        .ok_or_else(|| anyhow::anyhow!("malformed /proc/self/stat: no closing paren"))?;
    let tail = &line[close + 1..];
    let st = tail
        .split_ascii_whitespace()
        .nth(19)
        .ok_or_else(|| anyhow::anyhow!("malformed /proc/self/stat: not enough fields"))?;
    st.parse::<u64>()
        .map_err(|_| anyhow::anyhow!("invalid starttime field: {st:?}"))
}

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

    #[test]
    fn parses_self_stat_field_22() {
        let line = "1 (init) S 0 1 1 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 7 0 0 0 0 0 0 0";
        let st = parse_proc_stat_starttime(line).expect("parses");
        assert_eq!(st, 7);
    }

    #[test]
    fn parses_comm_with_spaces() {
        let line =
            "1234 (weird (comm) name) S 1 0 0 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 98765 0 0 0 0 0";
        let st = parse_proc_stat_starttime(line).expect("parses comm with parens");
        assert_eq!(st, 98765);
    }
}
