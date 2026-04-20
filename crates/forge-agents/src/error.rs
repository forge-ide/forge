//! Typed errors for `forge-agents`.
//!
//! A typed error enum rather than bare `anyhow` so the isolation-violation
//! branch can be pattern-matched by runtime callers (sub-agent spawners, IPC
//! layers) without string-matching.

use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    /// A user-authored agent (parsed from `.agents/*.md` or `~/.agents/*.md`,
    /// or constructed programmatically with `AgentScope::User`) declared
    /// `isolation: trusted`. That level is reserved for built-in skills
    /// shipped with Forge itself.
    #[error("isolation: trusted is not allowed for user-defined agents ({name}{location})",
            location = source_hint(path))]
    IsolationViolation { name: String, path: Option<PathBuf> },

    /// Parsing / IO / other non-isolation failures.
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

fn source_hint(p: &Option<PathBuf>) -> String {
    match p {
        Some(path) => format!(" from {}", path.display()),
        None => String::new(),
    }
}

pub type Result<T> = std::result::Result<T, Error>;
