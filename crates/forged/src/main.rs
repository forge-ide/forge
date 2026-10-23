//! Forge daemon binary.

use std::process::ExitCode;

use clap::Parser;
use forge_config::Paths;
use tracing_subscriber::EnvFilter;

/// The Forge daemon. Clients connect over the Unix socket in the runtime directory.
#[derive(Parser)]
#[command(version)]
struct Args {}

fn main() -> ExitCode {
    Args::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("forged=info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let result = tokio::runtime::Runtime::new()
        .map_err(anyhow::Error::from)
        .and_then(|rt| {
            rt.block_on(async {
                let paths = Paths::from_env()?;
                forged::run(paths, shutdown_signal()).await
            })
        });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("forged: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Resolves on SIGINT or SIGTERM.
async fn shutdown_signal() {
    use tokio::signal::unix::{SignalKind, signal};
    let Ok(mut term) = signal(SignalKind::terminate()) else {
        let _ = tokio::signal::ctrl_c().await;
        return;
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
