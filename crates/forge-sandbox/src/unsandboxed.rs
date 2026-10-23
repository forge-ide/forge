//! The opt-in tier with no isolation. Only the environment allowlist applies.

use std::path::Path;
use std::sync::Arc;

use forge_api::model::SandboxTier;
use tokio::process::Command;

use crate::SessionSandbox;
use crate::proxy::Proxy;

/// Runs commands as normal user processes, with the allowlisted environment, the real `HOME`,
/// and the daemon's `TMPDIR`. With a proxy, the proxy variables point at it; nothing makes a
/// process use it.
#[derive(Clone, Debug)]
pub struct Unsandboxed {
    env: Vec<(String, String)>,
    _proxy: Option<Arc<Proxy>>,
}

impl Unsandboxed {
    pub fn new() -> Self {
        Self::with_proxy(None)
    }

    pub fn with_proxy(proxy: Option<Arc<Proxy>>) -> Self {
        let mut env = crate::env::from_daemon();
        for name in ["HOME", "TMPDIR"] {
            if let Ok(value) = std::env::var(name) {
                env.push((name.to_owned(), value));
            }
        }
        if let Some(proxy) = &proxy {
            env.extend(proxy.env(&format!("http://127.0.0.1:{}", proxy.port())));
        }
        Self { env, _proxy: proxy }
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
            .envs(crate::env::extra(&self.env, env));
        cmd
    }
}
