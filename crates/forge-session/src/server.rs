use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use forge_core::{read_since, SessionId};
use forge_ipc::{HelloAck, IpcEvent, IpcMessage, PROTO_VERSION, SCHEMA_VERSION};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::broadcast;

use crate::session::Session;

pub async fn serve(path: &Path) -> Result<()> {
    let log_path = std::env::temp_dir()
        .join(format!("forge-session-{}", SessionId::new()))
        .join("events.jsonl");
    let session = Arc::new(Session::create(log_path).await?);
    serve_with_session(path, session).await
}

pub async fn serve_with_session(path: &Path, session: Arc<Session>) -> Result<()> {
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
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, session).await {
                eprintln!("connection error: {e}");
            }
        });
    }
}

async fn handle_connection(mut stream: UnixStream, session: Arc<Session>) -> Result<()> {
    let msg = forge_ipc::read_frame(&mut stream).await?;
    let IpcMessage::Hello(hello) = msg else {
        anyhow::bail!("expected Hello, got unexpected message type");
    };
    if hello.proto != PROTO_VERSION {
        anyhow::bail!("unsupported protocol version: {}", hello.proto);
    }

    let session_id = SessionId::new();
    let ack = IpcMessage::HelloAck(HelloAck {
        session_id: session_id.to_string(),
        workspace: String::new(),
        started_at: chrono::Utc::now().to_rfc3339(),
        event_seq: session.current_seq().await,
        schema_version: SCHEMA_VERSION,
    });
    forge_ipc::write_frame(&mut stream, &ack).await?;

    let msg = forge_ipc::read_frame(&mut stream).await?;
    let IpcMessage::Subscribe(sub) = msg else {
        anyhow::bail!("expected Subscribe after HelloAck");
    };

    // Subscribe to live broadcast BEFORE reading history to avoid missing events.
    let mut live_rx = session.event_tx.subscribe();

    let history = read_since(&session.log_path, sub.since).await?;
    let mut last_sent = sub.since;
    for (seq, event) in history {
        let frame = IpcMessage::Event(IpcEvent {
            seq,
            event: serde_json::to_value(&event)?,
        });
        forge_ipc::write_frame(&mut stream, &frame).await?;
        last_sent = seq;
    }

    loop {
        match live_rx.recv().await {
            Ok((seq, event)) if seq > last_sent => {
                let frame = IpcMessage::Event(IpcEvent {
                    seq,
                    event: serde_json::to_value(&event)?,
                });
                forge_ipc::write_frame(&mut stream, &frame).await?;
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

    Ok(())
}
