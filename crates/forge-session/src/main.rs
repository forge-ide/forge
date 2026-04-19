use anyhow::Result;
use forge_providers::{ollama::OllamaProvider, MockProvider};
use forge_session::{
    pid_file::OwnedPidFile,
    provider_spec::{parse_provider_spec, ProviderKind},
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
    let provider_spec = parse_flag(&args, "--provider").or_else(|| {
        std::env::var("FORGE_PROVIDER")
            .ok()
            .filter(|s| !s.is_empty())
    });

    // Allow the CLI to pre-assign the session ID and socket path so it can
    // print the path before forged starts and can track it for `session kill`.
    let session_id = std::env::var("FORGE_SESSION_ID")
        .unwrap_or_else(|_| forge_core::SessionId::new().to_string());
    let socket_path = std::env::var("FORGE_SOCKET_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| resolve_socket_path(&session_id));
    // Normalize FORGE_WORKSPACE to an absolute path so HelloAck.workspace is
    // portable for clients (which may have a different CWD than the daemon).
    // std::path::absolute does not require the path to exist, unlike canonicalize.
    let workspace = std::env::var("FORGE_WORKSPACE")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p));
    eprintln!("forged: listening on {}", socket_path.display());

    // F-049: persistent-mode forged owns the pid-file lifecycle. Created
    // with O_EXCL so a leftover file from a prior crash is not clobbered;
    // removed on drop (SIGTERM, SIGINT, or any exit path) so stale pid
    // files don't outlive the process. Ephemeral mode has no external
    // `session_kill` caller and so does not need a pid file.
    // Held in a binding that must outlive `serve_with_session`.
    let _pid_guard = if !ephemeral {
        std::env::var("FORGE_PID_FILE")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|p| OwnedPidFile::create(PathBuf::from(p)))
            .transpose()?
    } else {
        None
    };

    let log_path = event_log_path(&session_id, workspace.as_deref());
    let session = Arc::new(Session::create(log_path).await?);

    // Provider selection (F-038):
    //   1. explicit `--provider <spec>` flag, OR `FORGE_PROVIDER` env, OR
    //   2. Mock when `FORGE_MOCK_SEQUENCE_FILE` is set, OR
    //   3. Mock from default path (legacy fallback for ad-hoc runs).
    // The Provider trait uses `impl Future` (not object-safe), so we cannot
    // box and dispatch — instead, match here and call `serve_with_session`
    // with the concrete provider type from each branch.
    match provider_spec {
        Some(spec) => match parse_provider_spec(&spec)? {
            ProviderKind::Mock => {
                let provider = build_mock_provider().await?;
                serve_with_session(
                    &socket_path,
                    session,
                    provider,
                    auto_approve,
                    ephemeral,
                    workspace,
                    Some(session_id),
                )
                .await
            }
            ProviderKind::Ollama { model } => {
                // F-058 / M5 (T7): validate `OLLAMA_BASE_URL` before handing
                // it to reqwest. An unvalidated URL (the old `env::var` +
                // `unwrap_or_else` pattern) TLS-dials arbitrary hosts and
                // exfiltrates every chat transcript + tool-result payload.
                // Policy is enforced by `validate_base_url`; see its docs.
                let raw = std::env::var("OLLAMA_BASE_URL").ok();
                let allow_remote_raw =
                    std::env::var(forge_providers::ollama::ALLOW_REMOTE_ENV).ok();
                let allow_remote =
                    forge_providers::ollama::parse_allow_remote(allow_remote_raw.as_deref());
                let url = forge_providers::ollama::validate_base_url(raw.as_deref(), allow_remote)?;
                // Loudly surface the resolved URL so env-var redirection is
                // visible in logs. Remote-opt-in is called out explicitly.
                if allow_remote
                    && !matches!(
                        url.host_str(),
                        Some("127.0.0.1") | Some("localhost") | Some("::1") | Some("[::1]")
                    )
                {
                    eprintln!(
                        "forged: WARN ollama base_url = {} (remote endpoint enabled via {}=1)",
                        url,
                        forge_providers::ollama::ALLOW_REMOTE_ENV
                    );
                } else {
                    eprintln!("forged: ollama base_url = {}", url);
                }
                let provider = Arc::new(OllamaProvider::new(url.as_str(), model));
                serve_with_session(
                    &socket_path,
                    session,
                    provider,
                    auto_approve,
                    ephemeral,
                    workspace,
                    Some(session_id),
                )
                .await
            }
        },
        None => {
            let provider = build_mock_provider().await?;
            serve_with_session(
                &socket_path,
                session,
                provider,
                auto_approve,
                ephemeral,
                workspace,
                Some(session_id),
            )
            .await
        }
    }
}

/// Parse `--flag value` from a flat argv. Returns None if the flag isn't
/// present or if it has no following value. Mirrors the shape of the other
/// argv-walks in this file rather than introducing clap mid-task.
fn parse_flag(args: &[String], flag: &str) -> Option<String> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1).cloned()
}

/// FORGE_MOCK_SEQUENCE_FILE points to a JSON array of NDJSON scripts; each
/// element is consumed in order. Falls back to `with_default_path()` when
/// no file is configured.
async fn build_mock_provider() -> Result<Arc<MockProvider>> {
    if let Ok(seq_file) = std::env::var("FORGE_MOCK_SEQUENCE_FILE") {
        let content = tokio::fs::read_to_string(&seq_file).await?;
        let scripts: Vec<String> = serde_json::from_str(&content)?;
        Ok(Arc::new(MockProvider::from_responses(scripts)?))
    } else {
        Ok(Arc::new(MockProvider::with_default_path()))
    }
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
