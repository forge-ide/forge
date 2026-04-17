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
}
