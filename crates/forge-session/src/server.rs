use std::collections::HashMap;
use std::io::ErrorKind;
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use forge_core::{
    meta::{write_meta, SessionMeta},
    read_since, SessionId, SessionPersistence, SessionState, WorkspaceId,
};
use forge_ipc::{HelloAck, IpcEvent, IpcMessage, PROTO_VERSION, SCHEMA_VERSION};
use forge_providers::{MockProvider, Provider};
use tokio::net::{UnixListener, UnixStream};
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{broadcast, Mutex};

use crate::archive::archive_or_purge;
use crate::orchestrator::{run_turn, PendingApprovals};
use crate::sandbox::ChildRegistry;
use crate::session::Session;

/// Compute the session's `allowed_paths` glob list from its workspace root.
///
/// Returns `[format!("{}/**", canonical_ws)]` when `workspace` points at an
/// existing directory that canonicalizes successfully. Returns `vec![]`
/// otherwise (no workspace, or canonicalization failure). An empty list causes
/// every `fs.*` tool call to be rejected by
/// `forge_fs::validate_against_globs`, which is the intended fail-closed
/// behaviour when the session has no workspace to scope filesystem access to.
pub(crate) fn compute_allowed_paths(workspace: Option<&Path>) -> Vec<String> {
    let Some(ws) = workspace else {
        return Vec::new();
    };
    match std::fs::canonicalize(ws) {
        Ok(canonical) => vec![format!("{}/**", canonical.display())],
        Err(_) => Vec::new(),
    }
}

/// Resolves the events.jsonl path for a daemon session.
///
/// When `workspace` is provided, the log lives under
/// `<workspace>/.forge/sessions/<session_id>/events.jsonl`, which causes
/// `forge_core::workspace::ensure_gitignore` to bootstrap the workspace's
/// `.forge/.gitignore` on first session. Otherwise falls back to
/// `<temp_dir>/forge-session-<session_id>/events.jsonl` for tests and ad-hoc runs.
pub fn event_log_path(session_id: &str, workspace: Option<&Path>) -> PathBuf {
    match workspace {
        Some(ws) => ws
            .join(".forge")
            .join("sessions")
            .join(session_id)
            .join("events.jsonl"),
        None => std::env::temp_dir()
            .join(format!("forge-session-{session_id}"))
            .join("events.jsonl"),
    }
}

/// Start a session server using the default `MockProvider`.
pub async fn serve(path: &Path, auto_approve: bool, ephemeral: bool) -> Result<()> {
    let log_path = event_log_path(&SessionId::new().to_string(), None);
    let session = Arc::new(Session::create(log_path).await?);
    let provider = Arc::new(MockProvider::with_default_path());
    serve_with_session(path, session, provider, auto_approve, ephemeral, None, None).await
}

/// Timeout for the post-`EADDRINUSE` liveness probe. Short enough that a
/// genuinely orphaned socket doesn't stall daemon startup, long enough to let
/// a slow local daemon reply. The probe is `connect(2)` only — no handshake —
/// so the common-case round-trip is a few hundred microseconds.
const LIVENESS_PROBE_TIMEOUT: Duration = Duration::from_millis(200);

/// Bind a `UnixListener` at `path` without the classic pre-unlink TOCTOU.
///
/// F-056 (T6): the previous `if path.exists() { remove_file } ; bind` sequence
/// let an attacker with write access to the parent directory plant a symlink
/// and watch the daemon blindly unlink it. F-044 (H8) already closed the
/// practical `/tmp/forge-0/` window by refusing the shared-directory fallback
/// and chmod'ing the parent to 0o700, but the TOCTOU remains relevant as
/// defense-in-depth for any future path configuration (e.g. an operator
/// pointing `FORGE_SOCKET_PATH` into a shared location).
///
/// Protocol:
/// 1. Try `bind` first — never pre-unlink.
/// 2. On `EADDRINUSE`, probe with a short `UnixStream::connect`. If the probe
///    succeeds, another live daemon is listening: bail out. **Do not** unlink
///    another daemon's socket out from under it.
/// 3. On probe failure (connection refused or timeout), the entry is likely
///    an orphan. Confirm via `symlink_metadata` — which does **not** follow
///    symlinks — that the entry is a real socket file. Symlinks and regular
///    files are rejected outright: neither is a legitimate orphan, and
///    unlinking either would be the exact exploit path this fix closes.
/// 4. Unlink the confirmed orphan and retry `bind` exactly once.
async fn bind_uds_safely(path: &Path) -> Result<UnixListener> {
    match UnixListener::bind(path) {
        Ok(listener) => Ok(listener),
        Err(e) if e.kind() == ErrorKind::AddrInUse => {
            // Address in use — either a live daemon is already serving or a
            // prior daemon crashed leaving an orphan socket behind.
            let probe =
                tokio::time::timeout(LIVENESS_PROBE_TIMEOUT, UnixStream::connect(path)).await;
            if let Ok(Ok(_stream)) = probe {
                anyhow::bail!(
                    "refusing to bind at {}: another daemon is already listening",
                    path.display()
                );
            }

            // Probe failed (refused or timed out). Check the entry type
            // without following symlinks — `metadata()` would follow, which
            // is itself exploitable.
            let meta = tokio::fs::symlink_metadata(path).await.with_context(|| {
                format!(
                    "failed to stat {} while recovering from EADDRINUSE",
                    path.display()
                )
            })?;
            if !meta.file_type().is_socket() {
                anyhow::bail!(
                    "refusing to unlink {}: entry is not a socket (type={:?}). Remove it \
                     manually after verifying it is not attacker-planted.",
                    path.display(),
                    meta.file_type()
                );
            }

            tokio::fs::remove_file(path)
                .await
                .with_context(|| format!("failed to unlink orphan socket at {}", path.display()))?;
            UnixListener::bind(path)
                .with_context(|| format!("retry bind failed at {}", path.display()))
        }
        Err(e) => Err(e).with_context(|| format!("bind failed at {}", path.display())),
    }
}

/// Start a session server with an explicit provider.
///
/// `workspace` is reported back to clients via `HelloAck.workspace` (empty when `None`).
/// `session_id` is reported back to clients via `HelloAck.session_id` and identifies
/// this daemon's persistent session; when `None`, a fresh id is generated for the lifetime
/// of this server (so all connections to the same server still see the same value).
pub async fn serve_with_session<P: Provider + 'static>(
    path: &Path,
    session: Arc<Session>,
    provider: Arc<P>,
    auto_approve: bool,
    ephemeral: bool,
    workspace: Option<PathBuf>,
    session_id: Option<String>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
        // F-044 (H8): tighten the socket's parent dir to 0o700 regardless of
        // whether we created it or found it pre-existing. `XDG_RUNTIME_DIR`
        // itself is 0o700 per systemd.exec(5), but the `forge/sessions`
        // subdir sits inside and inherits `0o777 & ~umask` (typically 0o755)
        // on first creation. Applying 0o700 explicitly closes the tiny window
        // where the subdir is created loose and means the defense-in-depth
        // survives if a future operator runs forged with `XDG_RUNTIME_DIR`
        // pointed somewhere with a permissive parent. Best-effort: if the
        // chmod fails (e.g. parent owned by another user under FORGE_SOCKET_PATH
        // in a test), the 0o600 mode on the socket itself is the real defense.
        let _ = tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).await;
    }
    let listener = bind_uds_safely(path).await?;
    // F-044 (H8): chmod the socket to 0o600 the moment it exists. bind(2)
    // creates the socket file with `0o777 & ~umask`, typically 0o755 —
    // world-connectable, which in Phase 1 means any local user can drive the
    // session. There is a brief post-bind TOCTOU window between `bind` and
    // this chmod; it is a distinct concern from F-056 (which closed the
    // *pre-bind* `remove_file`→`bind` race via `bind_uds_safely`) and remains
    // bounded by the 0o700 parent-dir mode set above. We refuse to proceed if
    // the chmod fails rather than serve on a permissive socket.
    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).await?;
    let workspace_path: Option<PathBuf> = workspace.clone();
    // F-043: Derive `allowed_paths` from the workspace root so `fs.*` tools can
    // only touch files inside the session's workspace. Fail-closed — if no
    // workspace is configured, or canonicalization fails (e.g. the workspace
    // path doesn't exist), the session stays usable but every `fs.*` call is
    // rejected by `forge_fs::validate_against_globs` (empty list matches
    // nothing). Previously this was `vec!["**"]`, which matched every absolute
    // path including `/etc/passwd` and `~/.ssh/id_rsa`.
    let allowed_paths: Arc<Vec<String>> =
        Arc::new(compute_allowed_paths(workspace_path.as_deref()));
    let workspace = Arc::new(
        workspace
            .map(|w| w.display().to_string())
            .unwrap_or_default(),
    );
    let session_id = Arc::new(session_id.unwrap_or_else(|| SessionId::new().to_string()));

    let socket_path = Arc::new(path.to_path_buf());
    // Session-scoped registry of sandboxed child process groups. Killed on
    // session shutdown so tool subprocesses (e.g. `shell.exec`) cannot outlive
    // the daemon.
    let child_registry = ChildRegistry::new();

    if ephemeral {
        // Accept exactly one connection, serve it to completion, then exit.
        // `handle_connection` performs the session-scoped process-group
        // cleanup in its ephemeral branch.
        let (stream, _) = listener.accept().await?;
        return handle_connection(
            stream,
            session,
            provider,
            auto_approve,
            true,
            workspace,
            session_id,
            socket_path,
            workspace_path,
            allowed_paths,
            child_registry,
        )
        .await;
    }

    // Persistent mode: write the initial meta.toml so `archive_or_purge`'s
    // `update_meta_to_archived` call has something to update on shutdown.
    // F-031's archive feature only updates an existing meta file; without
    // this write, the archived directory ships with no meta.toml at all.
    if let Some(session_dir) = session.log_path.parent() {
        let meta_path = session_dir.join("meta.toml");
        let meta = SessionMeta {
            id: SessionId::from_string((*session_id).clone()),
            // TODO: derive workspace_id from the workspace path so sessions
            // sharing a workspace correlate. Today no production code groups
            // on workspace_id (dashboard reads `workspace` string), so a
            // synthetic per-session id is acceptable as a placeholder.
            workspace_id: WorkspaceId::new(),
            name: format!("session-{}", *session_id),
            agent: None,
            provider_id: None,
            model: None,
            state: SessionState::Active,
            persistence: SessionPersistence::Persist,
            started_at: Utc::now(),
            ended_at: None,
            tokens_in: 0,
            tokens_out: 0,
            cost_usd: 0.0,
            pid: std::process::id(),
            socket_path: (*socket_path).clone(),
        };
        write_meta(&meta_path, &meta).await?;
    }

    // Persistent mode: race the accept loop against SIGTERM/SIGINT so the
    // daemon can run `archive_or_purge` on its own session dir before exit.
    // Without this, `forge session kill` SIGTERMs an unhandled signal and
    // the live session dir is left behind under `sessions/<id>/`.
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, _) = accept?;
                let session = Arc::clone(&session);
                let provider = Arc::clone(&provider);
                let workspace = Arc::clone(&workspace);
                let session_id = Arc::clone(&session_id);
                let socket_path = Arc::clone(&socket_path);
                let workspace_path = workspace_path.clone();
                let allowed_paths = Arc::clone(&allowed_paths);
                let child_registry = child_registry.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(
                        stream,
                        session,
                        provider,
                        auto_approve,
                        false,
                        workspace,
                        session_id,
                        socket_path,
                        workspace_path,
                        allowed_paths,
                        child_registry,
                    )
                    .await
                    {
                        eprintln!("connection error: {e}");
                    }
                });
            }
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
        }
    }

    // Graceful shutdown for persistent mode: kill any sandboxed children,
    // then archive the session dir + remove the socket. Errors propagate so
    // `forged` exits non-zero on archive failure (so callers like a future
    // `forge session kill --wait` can surface the failure).
    //
    // In-flight `tokio::spawn`'d connection tasks (above) are not joined —
    // they're orphaned and the runtime drops them on process exit. On the
    // normal (same-filesystem) archive path, `rename` preserves open file
    // descriptors so any late EventLog write still lands on the correct
    // inode; meta.toml writes are atomic via temp+rename in
    // `forge_core::meta::write_meta`, so a torn-meta hazard is not possible.
    // The EXDEV fallback in `archive_or_purge` (cross-filesystem copy +
    // remove_dir_all) is theoretically unreachable here because the source
    // and destination both live under `<workspace>/.forge/sessions/`. If
    // shutdown ever needs to await in-flight turns, replace the spawn with
    // a JoinSet drained here with a timeout.
    child_registry.kill_all();
    if let Some(session_dir) = session.log_path.parent() {
        archive_or_purge(session_dir, SessionPersistence::Persist, &socket_path).await?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn handle_connection<P: Provider + 'static>(
    mut stream: UnixStream,
    session: Arc<Session>,
    provider: Arc<P>,
    auto_approve: bool,
    ephemeral: bool,
    workspace: Arc<String>,
    session_id: Arc<String>,
    socket_path: Arc<PathBuf>,
    workspace_path: Option<PathBuf>,
    allowed_paths: Arc<Vec<String>>,
    child_registry: ChildRegistry,
) -> Result<()> {
    // ── Handshake ──────────────────────────────────────────────────────────────
    let msg = forge_ipc::read_frame(&mut stream).await?;
    let IpcMessage::Hello(hello) = msg else {
        anyhow::bail!("expected Hello, got unexpected message type");
    };
    if hello.proto != PROTO_VERSION {
        anyhow::bail!("unsupported protocol version: {}", hello.proto);
    }

    let ack = IpcMessage::HelloAck(HelloAck {
        session_id: (*session_id).clone(),
        workspace: (*workspace).clone(),
        started_at: chrono::Utc::now().to_rfc3339(),
        event_seq: session.current_seq().await,
        schema_version: SCHEMA_VERSION,
    });
    forge_ipc::write_frame(&mut stream, &ack).await?;

    // ── Subscribe + history replay ─────────────────────────────────────────────
    let msg = forge_ipc::read_frame(&mut stream).await?;
    let IpcMessage::Subscribe(sub) = msg else {
        anyhow::bail!("expected Subscribe after HelloAck");
    };

    // Subscribe to live broadcast BEFORE reading history to avoid missing events.
    let mut live_rx = session.event_tx.subscribe();

    let history = read_since(&session.log_path, sub.since).await?;
    let mut last_sent = sub.since;

    // Split stream so we can read and write concurrently.
    let (mut reader, mut writer) = stream.into_split();

    for (seq, event) in history {
        let frame = IpcMessage::Event(IpcEvent {
            seq,
            event: serde_json::to_value(&event)?,
        });
        forge_ipc::write_frame(&mut writer, &frame).await?;
        last_sent = seq;
    }

    // ── Bidirectional loop ─────────────────────────────────────────────────────
    // Pending tool call approvals shared between this loop and spawned turn tasks.
    let pending_approvals: PendingApprovals = Arc::new(Mutex::new(HashMap::new()));

    // Channel for commands arriving from the client reader.
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<IpcMessage>(32);

    // Spawn a task that forwards client frames onto the command channel.
    tokio::spawn(async move {
        while let Ok(msg) = forge_ipc::read_frame(&mut reader).await {
            if cmd_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    loop {
        tokio::select! {
            // Live events → forward to client.
            result = live_rx.recv() => {
                match result {
                    Ok((seq, event)) if seq > last_sent => {
                        let is_session_ended = matches!(event, forge_core::Event::SessionEnded { .. });
                        let frame = IpcMessage::Event(IpcEvent {
                            seq,
                            event: serde_json::to_value(&event)?,
                        });
                        forge_ipc::write_frame(&mut writer, &frame).await?;
                        last_sent = seq;
                        if is_session_ended {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Closed) => break,
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        eprintln!("subscriber dropped {n} events; closing connection");
                        break;
                    }
                }
            }

            // Commands from client → dispatch.
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(IpcMessage::SendUserMessage(m)) => {
                        let session = Arc::clone(&session);
                        let provider = Arc::clone(&provider);
                        let approvals = Arc::clone(&pending_approvals);
                        let workspace_path = workspace_path.clone();
                        let allowed_paths = Arc::clone(&allowed_paths);
                        let child_registry = child_registry.clone();
                        tokio::spawn(async move {
                            let result = run_turn(
                                Arc::clone(&session),
                                provider,
                                m.text,
                                approvals,
                                (*allowed_paths).clone(),
                                auto_approve,
                                workspace_path,
                                Some(child_registry),
                            ).await;
                            if let Err(e) = &result {
                                eprintln!("turn error: {e}");
                            }
                            if ephemeral {
                                let reason = match result {
                                    Ok(()) => forge_core::EndReason::Completed,
                                    Err(e) => forge_core::EndReason::Error(e.to_string()),
                                };
                                if let Err(e) = session.emit(forge_core::Event::SessionEnded {
                                    at: chrono::Utc::now(),
                                    reason,
                                    archived: false,
                                }).await {
                                    eprintln!("failed to emit SessionEnded: {e}");
                                }
                            }
                        });
                    }

                    Some(IpcMessage::ToolCallApproved(a)) => {
                        let mut map = pending_approvals.lock().await;
                        if let Some(tx) = map.remove(&a.id) {
                            let _ = tx.send(true);
                        }
                    }

                    Some(IpcMessage::ToolCallRejected(r)) => {
                        let mut map = pending_approvals.lock().await;
                        if let Some(tx) = map.remove(&r.id) {
                            let _ = tx.send(false);
                        }
                    }

                    Some(_) => {} // ignore other messages
                    None => break,
                }
            }
        }
    }

    // In ephemeral mode the daemon exits after this connection, so we kill
    // any still-live sandboxed process groups here. For long-running
    // sessions, per-connection cleanup is handled by `SandboxedChild::drop`
    // (which both killpg's and deregisters from the shared registry);
    // the final `kill_all` runs when the daemon itself shuts down.
    if ephemeral {
        child_registry.kill_all();

        if let Some(session_dir) = session.log_path.parent() {
            if let Err(e) =
                archive_or_purge(session_dir, SessionPersistence::Ephemeral, &socket_path).await
            {
                eprintln!("archive_or_purge failed: {e}");
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_log_path_uses_workspace_when_set() {
        let ws = Path::new("/tmp/ws");
        let p = event_log_path("abc123", Some(ws));
        assert_eq!(p, Path::new("/tmp/ws/.forge/sessions/abc123/events.jsonl"));
    }

    #[test]
    fn event_log_path_falls_back_to_temp_when_unset() {
        let p = event_log_path("abc123", None);
        let expected = std::env::temp_dir()
            .join("forge-session-abc123")
            .join("events.jsonl");
        assert_eq!(p, expected);
    }

    #[test]
    fn compute_allowed_paths_returns_empty_when_workspace_absent() {
        assert!(compute_allowed_paths(None).is_empty());
    }

    #[test]
    fn compute_allowed_paths_returns_workspace_glob_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let canonical = std::fs::canonicalize(dir.path()).unwrap();
        let expected = format!("{}/**", canonical.display());
        assert_eq!(compute_allowed_paths(Some(dir.path())), vec![expected]);
    }

    #[test]
    fn compute_allowed_paths_returns_empty_when_workspace_does_not_exist() {
        // Fail-closed: a non-existent path can't be canonicalized, so the
        // session rejects every `fs.*` call rather than guessing at semantics.
        let missing = Path::new("/nonexistent/forge-f043-test-path");
        assert!(compute_allowed_paths(Some(missing)).is_empty());
    }

    // F-043 regression: derived `allowed_paths` must not permit reads outside
    // the session workspace. These tests exercise the *composition* of
    // `compute_allowed_paths` with `forge_fs::read_file` — a helper returning
    // `vec![]` is only safe if the enforcement layer it feeds actually denies
    // on empty. Likewise a workspace of `/tmp/ws` must not match `/etc/passwd`.
    //
    // `forge_fs::read_file` returns `Result<_, FsError>` (F-061); we assert
    // on the `Display` rendering of the typed `NotAllowed` variant, which
    // still formats as "…not allowed by allowed_paths".
    #[test]
    fn fs_read_etc_passwd_denied_when_no_workspace() {
        let allowed = compute_allowed_paths(None);
        let err = forge_fs::read_file("/etc/passwd", &allowed, &forge_fs::Limits::default())
            .expect_err("reading /etc/passwd without a workspace must fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not allowed by allowed_paths"),
            "expected path-denied error, got: {msg}"
        );
    }

    #[test]
    fn fs_read_etc_passwd_denied_when_workspace_is_tmp_ws() {
        // Use a real temp directory so canonicalization succeeds and we get a
        // non-empty allow-list. `/etc/passwd` must still be outside it.
        let dir = tempfile::tempdir().unwrap();
        let allowed = compute_allowed_paths(Some(dir.path()));
        assert!(
            !allowed.is_empty(),
            "workspace present → allow-list should not be empty"
        );
        let err = forge_fs::read_file("/etc/passwd", &allowed, &forge_fs::Limits::default())
            .expect_err("reading /etc/passwd with a scoped workspace must fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("not allowed by allowed_paths"),
            "expected path-denied error, got: {msg}"
        );
    }
}
