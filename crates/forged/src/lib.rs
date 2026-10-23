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
