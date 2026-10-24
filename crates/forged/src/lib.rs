//! Forge daemon: owns sessions, worktrees, sandboxes, policy, and the event log.
//!
//! The binary in `main.rs` is a thin wrapper around [`run`], which tests can also call in-process.

use std::future::Future;
use std::sync::Arc;

use anyhow::Context;
use forge_config::{Paths, UserConfig};

mod db;
mod dirs;
mod projects;
mod rpc;
mod server;
mod sessions;

/// Start the daemon and serve until `shutdown` resolves.
pub async fn run(paths: Paths, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
    let user = UserConfig::load(&paths.user_config()).context("loading user config")?;
    tracing::debug!(default_agent = ?user.default_agent, "loaded user config");

    dirs::ensure_private(&paths.data_dir)?;

    // Bind before opening the database, so a second daemon stops at "already running"
    // without touching the live daemon's database.
    let socket = paths.socket();
    let listener = server::bind(&socket)?;
    let db = match db::Db::open(&paths.database()) {
        Ok(db) => db,
        Err(e) => {
            let _ = std::fs::remove_file(&socket);
            return Err(e);
        }
    };
    tracing::info!(socket = %socket.display(), "listening");

    let state = match open_state(db, paths).await {
        Ok(state) => state,
        Err(e) => {
            let _ = std::fs::remove_file(&socket);
            return Err(e);
        }
    };
    server::serve(
        listener,
        &socket,
        state.clone(),
        server::Limits::default(),
        shutdown,
    )
    .await;
    state.sessions.shutdown().await;

    if let Err(e) = std::fs::remove_file(&socket) {
        tracing::warn!(error = %e, "removing socket on shutdown");
    }
    tracing::info!("stopped");
    Ok(())
}

/// The event log and session manager over an open database, with interrupted sessions
/// recovered.
async fn open_state(db: db::Db, paths: Paths) -> anyhow::Result<Arc<rpc::State>> {
    // Every event is redacted before it is written or published.
    let redactor = forge_session::SecretRedactor::new();
    let log = Arc::new(
        forge_events::EventLog::open(
            db.shared(),
            Arc::new(forge_events::Hub::new()),
            Box::new(redactor.clone()),
        )
        .context("opening the event log")?,
    );
    let helper = std::env::current_exe()
        .context("locating forged")?
        .with_file_name(forge_sandbox::HELPER);
    let sandbox = tokio::task::spawn_blocking(move || forge_sandbox::system(helper))
        .await
        .context("probing sandbox tiers")?;
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| paths.data_dir.clone());
    let sessions = forge_session::SessionManager::new(
        Arc::new(db.clone()),
        log.clone(),
        Arc::new(forge_git::GitCli),
        Arc::new(sessions::ProfileResolver {
            db: db.clone(),
            sandbox: sandbox.clone(),
            paths: paths.clone(),
            home: home.clone(),
            secrets: Arc::new(forge_sandbox::secrets::Keyring::system()),
        }),
        sandbox.clone(),
        forge_session::SandboxDirs {
            sessions: paths.data_dir.join("sessions"),
            home,
        },
        redactor,
    );
    sessions
        .recover()
        .await
        .map_err(|e| anyhow::anyhow!("recovering sessions: {e}"))?;
    Ok(Arc::new(rpc::State {
        db,
        log,
        sessions,
        sandbox,
        paths,
    }))
}
