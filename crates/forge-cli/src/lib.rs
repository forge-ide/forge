pub mod display;
pub mod socket;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "forge", about = "Forge IDE command-line interface")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Manage forge sessions.
    Session {
        #[command(subcommand)]
        cmd: SessionCommands,
    },
    /// One-shot ephemeral agent run.
    Run {
        #[command(subcommand)]
        cmd: RunCommands,
    },
}

#[derive(Subcommand, Debug)]
pub enum SessionCommands {
    /// Start a new session.
    New {
        #[command(subcommand)]
        kind: SessionNewKind,
    },
    /// List known sessions and their state.
    List,
    /// Stream events from a session to stdout.
    Tail {
        /// Session ID to tail.
        id: String,
    },
    /// Send SIGTERM to a session process.
    Kill {
        /// Session ID to kill.
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum SessionNewKind {
    /// Start an agent session.
    Agent {
        /// Agent name (must exist in .agents/).
        name: String,
        /// Workspace root directory.
        #[arg(long)]
        workspace: Option<PathBuf>,
        /// Provider spec (e.g. "ollama:qwen2.5:0.5b" or "mock"). Falls back
        /// to FORGE_PROVIDER env, then MockProvider.
        #[arg(long)]
        provider: Option<String>,
    },
    /// Start a bare provider session.
    Provider {
        /// Provider spec string.
        spec: String,
        /// Workspace root directory.
        #[arg(long)]
        workspace: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum RunCommands {
    /// Run an agent with a single input message and exit.
    Agent {
        /// Agent name.
        name: String,
        /// Input source: "-" reads from stdin, otherwise a file path.
        #[arg(long, default_value = "-")]
        input: String,
    },
}
