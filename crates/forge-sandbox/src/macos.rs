//! macOS backend: one Seatbelt profile, through `sandbox-exec`, for both Light and Standard.
//! Seatbelt has no cheaper, weaker mode worth offering, so the two tiers are the same here.
//!
//! The profile denies by default. It allows mapping executable code (dyld needs it to load any
//! binary; reading is still limited), reading system directories, toolchains, the
//! session's worktrees, and each repo's git dir; writing only the worktrees, their own git dirs,
//! and the session's scratch dirs; signals only within the sandbox; and no network. File
//! metadata stays readable everywhere so path lookups work, but contents outside the allowed
//! paths do not.

use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use forge_api::model::SandboxTier;
use tokio::process::Command;

use crate::{
    Sandbox, SandboxError, SessionSandbox, Spec, TierStatus, Unsandboxed, View, not_implemented,
    require,
};

const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

/// Mach services the system libraries need for user lookups, logging, and notifications.
const MACH_SERVICES: &[&str] = &[
    "com.apple.system.opendirectoryd.libinfo",
    "com.apple.system.opendirectoryd.membership",
    "com.apple.system.logger",
    "com.apple.system.notification_center",
    "com.apple.logd",
    "com.apple.diagnosticd",
    "com.apple.CoreServices.coreservicesd",
    "com.apple.SecurityServer",
];

pub struct Seatbelt {
    statuses: Vec<TierStatus>,
}

impl Seatbelt {
    /// Probe `sandbox-exec` with a trivial profile. Blocking.
    pub fn probe() -> Self {
        let works = std::process::Command::new(SANDBOX_EXEC)
            .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        let status = |tier| {
            if works {
                TierStatus::available(tier)
            } else {
                TierStatus::unavailable(
                    tier,
                    format!("{SANDBOX_EXEC} could not apply a Seatbelt profile"),
                    "check that sandbox-exec exists and works on this version of macOS",
                )
            }
        };
        Self {
            statuses: vec![
                status(SandboxTier::Light),
                status(SandboxTier::Standard),
                not_implemented(SandboxTier::Container),
                TierStatus::available(SandboxTier::Unsandboxed),
            ],
        }
    }
}

impl Sandbox for Seatbelt {
    fn tiers(&self) -> Vec<TierStatus> {
        self.statuses.clone()
    }

    fn prepare(&self, spec: Spec) -> Result<Arc<dyn SessionSandbox>, SandboxError> {
        require(&self.statuses, spec.tier)?;
        match spec.tier {
            SandboxTier::Unsandboxed => Ok(Arc::new(Unsandboxed::new())),
            SandboxTier::Light | SandboxTier::Standard => {
                let view = View::new(&spec)?;
                let mut env = crate::env::from_daemon();
                env.push(("HOME".into(), view.scratch_home.display().to_string()));
                env.push(("TMPDIR".into(), view.scratch_tmp.display().to_string()));
                env.extend(view.toolchain_env.iter().cloned());
                Ok(Arc::new(SeatbeltSession {
                    tier: spec.tier,
                    profile: profile(&view),
                    env,
                }))
            }
            SandboxTier::Container => unreachable!("never available"),
        }
    }
}

struct SeatbeltSession {
    tier: SandboxTier,
    profile: String,
    env: Vec<(String, String)>,
}

impl SessionSandbox for SeatbeltSession {
    fn tier(&self) -> SandboxTier {
        self.tier
    }

    fn command(
        &self,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
    ) -> Command {
        let mut cmd = Command::new(SANDBOX_EXEC);
        cmd.arg("-p")
            .arg(&self.profile)
            .arg(program)
            .args(args)
            .current_dir(cwd)
            .env_clear()
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .envs(env.iter().map(|(k, v)| (k, v)));
        cmd
    }
}

/// The Seatbelt profile for `view`. Paths are canonical already (`/var` is `/private/var`).
fn profile(view: &View) -> String {
    let mut p = String::from(
        "(version 1)\n\
         (deny default)\n\
         (allow process-exec)\n\
         (allow file-map-executable)\n\
         (allow process-fork)\n\
         (allow signal (target same-sandbox))\n\
         (allow process-info* (target same-sandbox))\n\
         (allow sysctl-read)\n\
         (allow user-preference-read)\n\
         (allow file-read-metadata file-test-existence)\n\
         (allow ipc-posix-shm-read-data ipc-posix-shm-read-metadata)\n",
    );
    p.push_str("(allow mach-lookup\n");
    for service in MACH_SERVICES {
        p.push_str(&format!("  (global-name {})\n", quote(service)));
    }
    p.push_str(")\n");

    let devices_rw = ["/dev/null", "/dev/zero"];
    let devices_ro = ["/dev/urandom", "/dev/random", "/dev/fd"];
    p.push_str("(allow file-read*\n");
    for path in view.read_only().iter().chain(view.read_write().iter()) {
        p.push_str(&format!("  (subpath {})\n", quote_path(path)));
    }
    for dev in devices_rw.iter().chain(devices_ro.iter()) {
        p.push_str(&format!("  (subpath {})\n", quote(dev)));
    }
    p.push_str(")\n(allow file-write*\n");
    for path in view.read_write() {
        p.push_str(&format!("  (subpath {})\n", quote_path(&path)));
    }
    p.push_str(")\n(allow file-write-data file-ioctl\n");
    for dev in devices_rw {
        p.push_str(&format!("  (literal {})\n", quote(dev)));
    }
    p.push_str(")\n");
    p
}

fn quote_path(path: &Path) -> String {
    quote(&path.display().to_string())
}

/// A Seatbelt string literal.
fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorktreeMount;

    #[test]
    fn profile_denies_by_default_and_writes_only_session_paths() {
        let view = View {
            system: vec!["/usr".into()],
            toolchains: vec![],
            toolchain_env: vec![],
            extra: vec![],
            worktrees: vec![WorktreeMount {
                path: "/Users/u/repo/.worktrees/feat/x".into(),
                git_common: "/Users/u/repo/.git".into(),
                git_admin: "/Users/u/repo/.git/worktrees/x".into(),
            }],
            home: "/Users/u".into(),
            scratch_home: "/private/var/s/home".into(),
            scratch_tmp: "/private/var/s/tmp".into(),
        };
        let p = profile(&view);
        assert!(p.starts_with("(version 1)\n(deny default)"));
        let writes = &p[p.find("(allow file-write*").unwrap()..];
        let writes = &writes[..writes.find(")\n(").unwrap()];
        assert!(writes.contains("\"/Users/u/repo/.worktrees/feat/x\""));
        assert!(writes.contains("\"/Users/u/repo/.git/worktrees/x\""));
        assert!(!writes.contains("\"/Users/u/repo/.git\""));
        assert!(!p.contains("(subpath \"/Users/u\")"));
        assert!(!p.contains("network"));
    }

    #[test]
    fn quotes_escape_backslashes_and_quotes() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }
}
