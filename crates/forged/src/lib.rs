//! Forge daemon: owns sessions, worktrees, sandboxes, policy, and the event log.
//!
//! The binary in `main.rs` is a thin wrapper around [`run`], which tests can also call in-process.

use std::future::Future;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;

use anyhow::Context;
use forge_config::{Paths, UserConfig};

mod db;
mod projects;
mod rpc;
mod server;

/// Start the daemon and serve until `shutdown` resolves.
pub async fn run(paths: Paths, shutdown: impl Future<Output = ()>) -> anyhow::Result<()> {
    let user = UserConfig::load(&paths.user_config()).context("loading user config")?;
    tracing::debug!(default_agent = ?user.default_agent, "loaded user config");

    std::fs::create_dir_all(&paths.data_dir)
        .with_context(|| format!("creating {}", paths.data_dir.display()))?;
    std::fs::set_permissions(&paths.data_dir, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("restricting {}", paths.data_dir.display()))?;
    let db = db::Db::open(&paths.database())?;

    let socket = paths.socket();
    let listener = server::bind(&socket)?;
    tracing::info!(socket = %socket.display(), "listening");

    let state = Arc::new(rpc::State {
        db,
        hub: forge_events::Hub::new(),
    });
    server::serve(listener, &socket, state, shutdown).await;

    if let Err(e) = std::fs::remove_file(&socket) {
        tracing::warn!(error = %e, "removing socket on shutdown");
    }
    tracing::info!("stopped");
    Ok(())
}
