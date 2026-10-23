//! The opt-in tier with no isolation. Only the environment allowlist applies.

use std::path::Path;

use forge_api::model::SandboxTier;
use tokio::process::Command;

use crate::SessionSandbox;

/// Runs commands as normal user processes, with the allowlisted environment, the real `HOME`,
/// and the daemon's `TMPDIR`.
#[derive(Clone, Debug)]
pub struct Unsandboxed {
    env: Vec<(String, String)>,
}

impl Unsandboxed {
    pub fn new() -> Self {
        let mut env = crate::env::from_daemon();
        for name in ["HOME", "TMPDIR"] {
            if let Ok(value) = std::env::var(name) {
                env.push((name.to_owned(), value));
            }
        }
        Self { env }
    }
}

impl Default for Unsandboxed {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionSandbox for Unsandboxed {
    fn tier(&self) -> SandboxTier {
        SandboxTier::Unsandboxed
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> Command {
        let mut cmd = Command::new(program);
        cmd.args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .envs(env.iter().map(|(k, v)| (k, v)));
        cmd
    }
}
