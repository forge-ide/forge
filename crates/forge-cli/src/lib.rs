pub mod display;
pub mod socket;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

/// Clap `value_parser` for `<session-id>` arguments.
///
/// Enforces the canonical `SessionId` wire format (`^[0-9a-f]{16}$`) *at
/// parse time* — before any command body runs — so attacker-controlled ids
/// like `../../tmp/evil` can never reach `socket::pid_path` or
/// `socket::socket_path`. See F-057 (T12a, path-traversal vector for raw-PID
/// SIGTERM).
///
/// Returning `Err(String)` here produces a typed clap error message of the
/// form `invalid value '<id>' for '<session-id>': ...`, which is what the
/// DoD calls a "typed validation error".
fn parse_session_id(raw: &str) -> Result<String, String> {
    if socket::session_id_is_valid(raw) {
        Ok(raw.to_string())
    } else {
        Err(format!(
            "session id must be 16 lowercase hex characters (got {:?})",
            raw
        ))
    }
}

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
        /// Session ID to tail (16 lowercase hex characters).
        #[arg(value_parser = parse_session_id)]
        id: String,
    },
    /// Send SIGTERM to a session process.
    Kill {
        /// Session ID to kill (16 lowercase hex characters).
        #[arg(value_parser = parse_session_id)]
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
