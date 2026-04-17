use anyhow::Result;
use forge_session::server::serve;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> Result<()> {
    let session_id = forge_core::SessionId::new();
    let socket_path = resolve_socket_path(&session_id.to_string());
    eprintln!("forged: listening on {}", socket_path.display());
    serve(&socket_path).await
}

fn resolve_socket_path(session_id: &str) -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let uid = std::env::var("UID").unwrap_or_else(|_| "0".to_string());
            PathBuf::from(format!("/tmp/forge-{uid}"))
        });
    base.join("forge/sessions")
        .join(format!("{session_id}.sock"))
}
