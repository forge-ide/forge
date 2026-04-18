use anyhow::Result;
use forge_providers::MockProvider;
use forge_session::{
    server::{event_log_path, serve_with_session},
    session::Session,
};
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let auto_approve = args.iter().any(|a| a == "--auto-approve-unsafe");
    let ephemeral = args.iter().any(|a| a == "--ephemeral");

    // Allow the CLI to pre-assign the session ID and socket path so it can
    // print the path before forged starts and can track it for `session kill`.
    let session_id = std::env::var("FORGE_SESSION_ID")
        .unwrap_or_else(|_| forge_core::SessionId::new().to_string());
    let socket_path = std::env::var("FORGE_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| resolve_socket_path(&session_id));
    let workspace = std::env::var("FORGE_WORKSPACE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    eprintln!("forged: listening on {}", socket_path.display());

    let log_path = event_log_path(&session_id, workspace.as_deref());

    // FORGE_MOCK_SEQUENCE_FILE points to a JSON array of NDJSON scripts.
    // Each element is used in order as the response for successive provider calls.
    // Used by integration tests to inject scripted multi-turn responses.
    let provider: Arc<MockProvider> =
        if let Ok(seq_file) = std::env::var("FORGE_MOCK_SEQUENCE_FILE") {
            let content = tokio::fs::read_to_string(&seq_file).await?;
            let scripts: Vec<String> = serde_json::from_str(&content)?;
            Arc::new(MockProvider::from_responses(scripts)?)
        } else {
            Arc::new(MockProvider::with_default_path())
        };

    let session = Arc::new(Session::create(log_path).await?);
    serve_with_session(
        &socket_path,
        session,
        provider,
        auto_approve,
        ephemeral,
        workspace,
    )
    .await
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
