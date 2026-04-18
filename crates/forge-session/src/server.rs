use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use forge_core::{read_since, SessionId, SessionPersistence};
use forge_ipc::{HelloAck, IpcEvent, IpcMessage, PROTO_VERSION, SCHEMA_VERSION};
use forge_providers::{MockProvider, Provider};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Mutex};

use crate::archive::archive_or_purge;
use crate::orchestrator::{run_turn, PendingApprovals};
use crate::session::Session;

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
    }
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }
    let listener = UnixListener::bind(path)?;
    let workspace = Arc::new(
        workspace
            .map(|w| w.display().to_string())
            .unwrap_or_default(),
    );
    let session_id = Arc::new(session_id.unwrap_or_else(|| SessionId::new().to_string()));

    let socket_path = Arc::new(path.to_path_buf());

    if ephemeral {
        // Accept exactly one connection, serve it to completion, then exit.
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
        )
        .await;
    }

    loop {
        let (stream, _) = listener.accept().await?;
        let session = Arc::clone(&session);
        let provider = Arc::clone(&provider);
        let workspace = Arc::clone(&workspace);
        let session_id = Arc::clone(&session_id);
        let socket_path = Arc::clone(&socket_path);
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
            )
            .await
            {
                eprintln!("connection error: {e}");
            }
        });
    }
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
                        tokio::spawn(async move {
                            let result = run_turn(Arc::clone(&session), provider, m.text, approvals, vec!["**".to_string()], auto_approve).await;
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

    if ephemeral {
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
}
