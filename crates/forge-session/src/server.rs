use anyhow::Result;
use forge_ipc::{HelloAck, IpcMessage, PROTO_VERSION, SCHEMA_VERSION};
use std::path::Path;
use tokio::net::{UnixListener, UnixStream};

pub async fn serve(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    if path.exists() {
        tokio::fs::remove_file(path).await?;
    }
    let listener = UnixListener::bind(path)?;
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream).await {
                eprintln!("connection error: {e}");
            }
        });
    }
}

async fn handle_connection(mut stream: UnixStream) -> Result<()> {
    let msg = forge_ipc::read_frame(&mut stream).await?;

    let IpcMessage::Hello(hello) = msg else {
        anyhow::bail!("expected Hello, got unexpected message type");
    };

    if hello.proto != PROTO_VERSION {
        anyhow::bail!("unsupported protocol version: {}", hello.proto);
    }

    let session_id = forge_core::SessionId::new();
    let ack = IpcMessage::HelloAck(HelloAck {
        session_id: session_id.to_string(),
        workspace: String::new(),
        started_at: chrono::Utc::now().to_rfc3339(),
        event_seq: 0,
        schema_version: SCHEMA_VERSION,
    });

    forge_ipc::write_frame(&mut stream, &ack).await?;
    Ok(())
}
