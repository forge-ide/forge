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
        // The app redirects stderr to forged.log; keep color codes out of it.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();

    let result = tokio::runtime::Runtime::new()
        .map_err(anyhow::Error::from)
        .and_then(|rt| {
            rt.block_on(async {
                let paths = Paths::from_env()?;
                // Install the handlers before the socket exists, so a client that sees the
                // socket can always stop the daemon cleanly.
                let shutdown = shutdown_signal()?;
                forged::run(paths, shutdown).await
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

/// Registers SIGINT and SIGTERM handlers now; the future resolves when either arrives.
fn shutdown_signal() -> anyhow::Result<impl std::future::Future<Output = ()>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    Ok(async move {
        tokio::select! {
            _ = interrupt.recv() => {}
            _ = term.recv() => {}
        }
    })
}
