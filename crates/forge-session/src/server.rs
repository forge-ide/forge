use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use forge_core::{read_since, SessionId};
use forge_ipc::{HelloAck, IpcEvent, IpcMessage, PROTO_VERSION, SCHEMA_VERSION};
use forge_providers::{MockProvider, Provider};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{broadcast, Mutex};

use crate::orchestrator::{run_turn, PendingApprovals};
use crate::session::Session;

/// Start a session server using the default `MockProvider`.
pub async fn serve(path: &Path) -> Result<()> {
    let log_path = std::env::temp_dir()
        .join(format!("forge-session-{}", SessionId::new()))
        .join("events.jsonl");
    let session = Arc::new(Session::create(log_path).await?);
    let provider = Arc::new(MockProvider::with_default_path());
    serve_with_session(path, session, provider).await
}

/// Start a session server with an explicit provider.
pub async fn serve_with_session<P: Provider + 'static>(
    path: &Path,
    session: Arc<Session>,
    provider: Arc<P>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }
    let listener = UnixListener::bind(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let session = Arc::clone(&session);
        let provider = Arc::clone(&provider);
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, session, provider).await {
                eprintln!("connection error: {e}");
            }
        });
    }
}

async fn handle_connection<P: Provider + 'static>(
    mut stream: UnixStream,
    session: Arc<Session>,
    provider: Arc<P>,
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
        session_id: SessionId::new().to_string(),
        workspace: String::new(),
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
                        let frame = IpcMessage::Event(IpcEvent {
                            seq,
                            event: serde_json::to_value(&event)?,
                        });
                        forge_ipc::write_frame(&mut writer, &frame).await?;
                        last_sent = seq;
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
                            // TODO(F-013): thread AgentDef.allowed_paths once agent
                            // context is passed through the server connection.
                            // Using ["**"] (allow all) until then.
                            if let Err(e) = run_turn(session, provider, m.text, approvals, vec!["**".to_string()]).await {
                                eprintln!("turn error: {e}");
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

    Ok(())
}
