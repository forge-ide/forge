use std::path::PathBuf;

/// Resolve the Unix socket path for a session, using the same logic as `forged`.
pub fn socket_path(session_id: &str) -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let uid = std::env::var("UID").unwrap_or_else(|_| "0".to_string());
            PathBuf::from(format!("/tmp/forge-{uid}"))
        });
    base.join("forge/sessions")
        .join(format!("{session_id}.sock"))
}

/// Return the directory containing all session sockets.
pub fn sessions_socket_dir() -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let uid = std::env::var("UID").unwrap_or_else(|_| "0".to_string());
            PathBuf::from(format!("/tmp/forge-{uid}"))
        });
    base.join("forge/sessions")
}

/// Resolve the PID file path for a session.
pub fn pid_path(session_id: &str) -> PathBuf {
    socket_path(session_id).with_extension("pid")
}

/// Parse and validate the contents of a session pid file.
///
/// Returns an error if the contents do not parse as an integer, or if the
/// resulting pid is less than or equal to zero. POSIX `kill(2)` treats
/// `pid == 0` as "signal every process in the caller's process group" and
/// `pid == -1` as "signal every process the user may signal"; both would
/// detonate far outside the intended target, so they must never reach
/// `libc::kill`.
pub fn parse_session_pid(raw: &str) -> anyhow::Result<libc::pid_t> {
    let trimmed = raw.trim();
    let pid: libc::pid_t = trimmed
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid pid file contents: {trimmed:?}"))?;
    anyhow::ensure!(pid > 0, "refusing to signal non-positive pid {pid}");
    Ok(pid)
}

/// Render the contents to write to a session pid file, given the result of
/// `std::process::Child::id()` / `tokio::process::Child::id()`.
///
/// Rejects `None` — a missing pid means the child was already reaped before
/// we could record it, and writing `"0"` to the pid file would later cause
/// `libc::kill(0, SIGTERM)` to signal the caller's entire process group.
/// Also rejects `Some(0)` defensively so that neither helper in this module
/// can ever emit a non-positive pid for `libc::kill`.
pub fn pid_file_contents(pid: Option<u32>) -> anyhow::Result<String> {
    let pid =
        pid.ok_or_else(|| anyhow::anyhow!("forged child exited before its pid could be recorded"))?;
    anyhow::ensure!(pid > 0, "refusing to write non-positive pid {pid}");
    Ok(pid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn socket_path_uses_xdg_runtime_dir() {
        // Temporarily set XDG_RUNTIME_DIR to a known value.
        // Use a fixed path so the test is deterministic.
        // We can't really set env vars safely in parallel tests, so just verify
        // the path structure with the fallback UID approach.
        let path = socket_path("deadbeefcafebabe");
        let path_str = path.to_string_lossy();
        assert!(
            path_str.contains("forge/sessions"),
            "expected forge/sessions in path: {path_str}"
        );
        assert!(
            path_str.ends_with("deadbeefcafebabe.sock"),
            "expected .sock extension: {path_str}"
        );
    }

    #[test]
    fn pid_path_has_pid_extension() {
        let path = pid_path("abc123");
        assert!(
            path.to_string_lossy().ends_with("abc123.pid"),
            "expected .pid extension: {}",
            path.display()
        );
    }

    #[test]
    fn sessions_socket_dir_contains_forge_sessions() {
        let dir = sessions_socket_dir();
        assert!(
            dir.to_string_lossy().contains("forge/sessions"),
            "got: {}",
            dir.display()
        );
    }

    #[test]
    fn parse_session_pid_accepts_positive() {
        let pid = parse_session_pid("4242").expect("positive pid should parse");
        assert_eq!(pid, 4242);
    }

    #[test]
    fn parse_session_pid_trims_whitespace_and_newlines() {
        let pid = parse_session_pid("  1234\n").expect("trimmed pid should parse");
        assert_eq!(pid, 1234);
    }

    #[test]
    fn parse_session_pid_rejects_zero() {
        let err = parse_session_pid("0").expect_err("pid 0 must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("non-positive") && msg.contains('0'),
            "expected non-positive rejection mentioning 0, got: {msg}"
        );
    }

    #[test]
    fn parse_session_pid_rejects_negative_one() {
        let err = parse_session_pid("-1").expect_err("pid -1 must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("non-positive") && msg.contains("-1"),
            "expected non-positive rejection mentioning -1, got: {msg}"
        );
    }

    #[test]
    fn parse_session_pid_rejects_negative_group() {
        let err = parse_session_pid("-42").expect_err("negative pid must be rejected");
        assert!(err.to_string().contains("non-positive"));
    }

    #[test]
    fn parse_session_pid_rejects_garbage() {
        let err = parse_session_pid("not-a-number").expect_err("garbage must be rejected");
        assert!(err.to_string().contains("invalid"));
    }

    #[test]
    fn pid_file_contents_returns_string_for_known_pid() {
        let s = pid_file_contents(Some(4242)).expect("known pid should produce contents");
        assert_eq!(s, "4242");
    }

    #[test]
    fn pid_file_contents_rejects_none() {
        let err = pid_file_contents(None).expect_err("None pid must be rejected");
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("pid"),
            "expected message to mention pid, got: {msg}"
        );
    }

    #[test]
    fn pid_file_contents_rejects_zero_defensively() {
        let err = pid_file_contents(Some(0)).expect_err("pid 0 must be rejected");
        assert!(err.to_string().contains("non-positive"));
    }
}
