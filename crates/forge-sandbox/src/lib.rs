//! Isolation backends behind one Sandbox trait.
//!
//! A [`Sandbox`] knows which tiers this machine can run. [`Sandbox::prepare`] turns a session's
//! [`Spec`] (its worktrees, scratch dir, and extra read-only paths) into a [`SessionSandbox`],
//! which wraps every process the session starts, the agent and each terminal command, so they
//! all see the same filesystem view, environment, and network.
//!
//! Backends:
//! - Light on Linux: Landlock and seccomp, applied by the `forge-sandbox-exec` helper to itself
//!   before it execs the command.
//! - Standard on Linux: bubblewrap with user, mount, PID, network, IPC, and UTS namespaces, plus
//!   the helper's seccomp filter inside.
//! - Light and Standard on macOS: one Seatbelt profile through `sandbox-exec`.
//! - Unsandboxed: no isolation; only the environment allowlist applies.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_api::model::SandboxTier;
use tokio::process::Command;

mod env;
mod git;
pub mod policy;
pub mod proxy;
pub mod secrets;
mod unsandboxed;
mod view;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

/// Runs the helper binary's modes. Not for use outside `forge-sandbox-exec`.
#[doc(hidden)]
pub mod helper;

pub use env::{PASSTHROUGH_VARS, STARTUP_FILES_OFF, is_passed_through, is_valid_name};
pub use git::{GitConfinement, PlainGit};
pub use unsandboxed::{SIGN_IN_VARS, Unsandboxed};
pub use view::{HOME_WRITABLE, SYSTEM_READ_ONLY, View, shows_read_only, toolchains};

/// File name of the helper binary, installed next to `forged`.
pub const HELPER: &str = "forge-sandbox-exec";

/// Why a tier can't run on this machine, and what would fix it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unavailable {
    pub reason: String,
    pub fix: String,
}

impl std::fmt::Display for Unavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}; {}", self.reason, self.fix)
    }
}

/// Whether one tier can run here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TierStatus {
    pub tier: SandboxTier,
    /// `None` when the tier works.
    pub unavailable: Option<Unavailable>,
}

impl TierStatus {
    pub fn available(tier: SandboxTier) -> Self {
        Self {
            tier,
            unavailable: None,
        }
    }

    pub fn unavailable(
        tier: SandboxTier,
        reason: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            tier,
            unavailable: Some(Unavailable {
                reason: reason.into(),
                fix: fix.into(),
            }),
        }
    }

    pub fn is_available(&self) -> bool {
        self.unavailable.is_none()
    }
}

/// A session worktree and the git metadata it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorktreeMount {
    /// The worktree; read-write.
    pub path: PathBuf,
    /// The main repo's git dir (`<repo>/.git`); read-only, so the agent can't plant hooks or
    /// config that run later in the user's own git.
    pub git_common: PathBuf,
    /// This worktree's own git dir (`<repo>/.git/worktrees/<name>`); read-write, for its index
    /// and HEAD.
    pub git_admin: PathBuf,
}

/// Everything a session's sandbox is built from.
#[derive(Clone, Debug)]
pub struct Spec {
    pub tier: SandboxTier,
    pub worktrees: Vec<WorktreeMount>,
    /// Session-private directory; the sandbox's home and temp dirs are created inside it.
    pub scratch: PathBuf,
    /// The user's real home directory, hidden inside the sandbox.
    pub home: PathBuf,
    /// Extra paths visible read-only: the agent program and the user config's
    /// `[sandbox] read_only` entries.
    pub read_only: Vec<PathBuf>,
    /// Extra paths visible read-write, at their real paths: an agent's own login dir. The
    /// caller checks they hold nothing else of the user's.
    pub read_write: Vec<PathBuf>,
    /// Paths inside the home's writable dirs ([`HOME_WRITABLE`]) that stay read-only and
    /// empty: where an agent keeps scripts it runs without asking. Only the tiers that can
    /// mount one over the other (Standard, and Seatbelt) keep them so; Light can't.
    pub home_locked: Vec<HomeLock>,
    /// The session's egress proxy, the only way out to the network. Without one a sandboxed
    /// session has no network at all. The sandbox keeps it running while it lives.
    pub proxy: Option<Arc<proxy::Proxy>>,
}

/// A path in the sandbox home, relative to it, kept read-only and empty.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HomeLock {
    pub path: PathBuf,
    /// A directory, or else a file.
    pub dir: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SandboxError {
    #[error("the {tier} sandbox tier is not available: {why}")]
    Unavailable { tier: String, why: Unavailable },
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

impl SandboxError {
    pub(crate) fn io(context: impl Into<String>, source: std::io::Error) -> Self {
        Self::Io {
            context: context.into(),
            source,
        }
    }
}

/// Isolation backends for this machine.
pub trait Sandbox: Send + Sync + 'static {
    /// Every tier this backend knows, with whether it works here.
    fn tiers(&self) -> Vec<TierStatus>;

    /// Build the sandbox for one session. Blocking: creates the session's scratch dirs.
    fn prepare(&self, spec: Spec) -> Result<Arc<dyn SessionSandbox>, SandboxError>;

    /// The daemon's own git, confined (see [`GitConfinement`]), or why this machine can't
    /// confine it.
    fn git_confinement(&self) -> Result<Arc<GitConfinement>, Unavailable>;
}

/// One session's sandbox. Every process the session starts goes through [`Self::command`].
pub trait SessionSandbox: Send + Sync + 'static {
    fn tier(&self) -> SandboxTier;

    /// The value the sandbox gives variable `name` (`HTTP_PROXY`, `HOME`, ...), as a process
    /// inside it sees it.
    fn var(&self, name: &str) -> Option<String>;

    /// A command that runs `program args` in `cwd` inside the sandbox, with the sandbox's
    /// environment plus `env`. `env` (which an agent chooses for its terminals) can't replace
    /// any variable the sandbox sets (`PATH`, `HOME`, `TMPDIR`, the proxy variables, ...), and
    /// entries with an empty name or a name containing `=` or NUL are dropped. Its variables
    /// reach only the sandboxed command, never the launcher that sets the sandbox up, where a
    /// loader variable such as `LD_PRELOAD` would run code before any restriction applies.
    /// The caller sets stdio and the process group.
    fn command(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> Command;
}

/// The wire name of a tier, as in `server.hello`.
pub fn tier_name(tier: SandboxTier) -> &'static str {
    match tier {
        SandboxTier::Light => "light",
        SandboxTier::Standard => "standard",
        SandboxTier::Container => "container",
        SandboxTier::Unsandboxed => "unsandboxed",
    }
}

/// The backends for this platform, with their availability probed once now. `helper` is the
/// `forge-sandbox-exec` binary. Blocking: runs a short probe per tier.
pub fn system(helper: PathBuf) -> Arc<dyn Sandbox> {
    #[cfg(target_os = "linux")]
    {
        Arc::new(linux::Linux::probe(helper))
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(macos::Seatbelt::probe(helper))
    }
}

/// Fails a tier this backend doesn't implement.
pub(crate) fn not_implemented(tier: SandboxTier) -> TierStatus {
    TierStatus::unavailable(
        tier,
        format!("the {} tier is not implemented yet", tier_name(tier)),
        "choose light or standard",
    )
}

/// The status for `tier` in `statuses`, or an error saying why it can't run.
pub(crate) fn require(statuses: &[TierStatus], tier: SandboxTier) -> Result<(), SandboxError> {
    match statuses.iter().find(|s| s.tier == tier) {
        Some(TierStatus {
            unavailable: None, ..
        }) => Ok(()),
        Some(TierStatus {
            unavailable: Some(why),
            ..
        }) => Err(SandboxError::Unavailable {
            tier: tier_name(tier).to_owned(),
            why: why.clone(),
        }),
        None => Err(SandboxError::Unavailable {
            tier: tier_name(tier).to_owned(),
            why: not_implemented(tier).unavailable.expect("unavailable"),
        }),
    }
}
