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
//! path at all when a per-user runtime directory cannot be established.
//! `forged` will fail to start with a clear error naming the fix. On Linux
//! with systemd `XDG_RUNTIME_DIR` is always set to a per-user `0o700` tmpfs
//! mount; anything else is a misconfiguration that we surface loudly rather
//! than mask with a shared-tmp fallback.
//!
//! # F-339: macOS fallback
//!
//! launchd does not export `XDG_RUNTIME_DIR`, so out-of-the-box `cargo run
//! -p forge-shell` on macOS used to error out at startup. The shared helper
//! in [`forge_core::runtime_dir`] now resolves
//! `$HOME/Library/Application Support/Forge/run` on macOS when
//! `XDG_RUNTIME_DIR` is unset — creating it with mode `0o700` and rejecting
//! any pre-existing directory with looser perms. The F-044 invariant
//! (per-user `0o700`) is preserved, just via a natively-appropriate path.
//!
//! See `server.rs` for the post-bind `chmod` defense-in-depth that runs
//! irrespective of this resolver's input.

use std::path::PathBuf;

use anyhow::Result;

/// Resolve the UDS socket path for a session under the per-user runtime dir.
///
/// Delegates the runtime-dir policy to [`forge_core::runtime_dir::runtime_dir`]
/// so `forged`, the CLI, and the Tauri shell all agree on the same rules:
///   - honor `XDG_RUNTIME_DIR` when set (Linux priority preserved);
///   - on macOS, fall back to `$HOME/Library/Application Support/Forge/run`
///     with `0o700` enforcement (F-339);
///   - on Linux with `XDG_RUNTIME_DIR` unset, error rather than fall back
///     to a shared `/tmp` directory (F-044 / H8).
pub fn resolve_socket_path(session_id: &str) -> Result<PathBuf> {
    Ok(forge_core::runtime_dir::runtime_dir()?
        .join("forge/sessions")
        .join(format!("{session_id}.sock")))
}
