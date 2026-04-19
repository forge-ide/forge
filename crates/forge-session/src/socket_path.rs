//! UDS socket-path resolution for `forged`.
//!
//! # F-044 (H8): rejecting the `/tmp` fallback
//!
//! Phase 1's IPC trust boundary is a Unix domain socket; anyone who can
//! connect to it can drive the session, approve tool calls, and exfiltrate
//! events. Prior to F-044 this resolver fell back to `/tmp/forge-{uid}` when
//! `XDG_RUNTIME_DIR` was unset, and read the UID from `std::env::var("UID")`
//! — a shell variable that child processes don't generally inherit — with a
//! fallback of `"0"`. That combination let multiple local users share a
//! world-accessible `/tmp/forge-0/forge/sessions/` directory.
//!
//! The remediation (option 4 of the H8 finding) is to refuse to resolve a
//! path at all when `XDG_RUNTIME_DIR` is unset. `forged` will fail to start
//! with a clear error naming the missing env var. On Linux with systemd
//! `XDG_RUNTIME_DIR` is always set to a per-user `0o700` tmpfs mount, so the
//! only production path is through `$XDG_RUNTIME_DIR/forge/sessions/<id>.sock`;
//! anything else is a misconfiguration that we surface loudly rather than
//! mask with a shared-tmp fallback.
//!
//! See `server.rs` for the post-bind `chmod` defense-in-depth that runs
//! irrespective of this resolver's input.

use std::path::PathBuf;

use anyhow::{anyhow, Result};

/// Resolve the UDS socket path for a session under `$XDG_RUNTIME_DIR`.
///
/// Returns `Err` when `XDG_RUNTIME_DIR` is unset or empty. The error message
/// names the missing env var so operators reading `forged`'s stderr know the
/// fix. No `/tmp` fallback is attempted.
pub fn resolve_socket_path(session_id: &str) -> Result<PathBuf> {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "forged refuses to start: XDG_RUNTIME_DIR is unset. \
                 This env var must point to a per-user 0o700 directory \
                 (systemd sets it to /run/user/<uid> automatically). \
                 Set it explicitly or use FORGE_SOCKET_PATH to override \
                 the socket location. (F-044 / H8)"
            )
        })?;
    Ok(PathBuf::from(base)
        .join("forge/sessions")
        .join(format!("{session_id}.sock")))
}
