//! Landlock rules for the Light tier, applied by the helper to itself.
//!
//! ABI 6 (Linux 6.12) is the floor: it adds scoping, so the sandbox can't signal processes
//! outside it or connect to abstract Unix sockets created outside it. Newer rights, such as
//! ABI 9's control over connecting to pathname Unix sockets, are used when the kernel has them.

use landlock::{
    ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, NetPort, PathBeneath, PathFd,
    Ruleset, RulesetAttr, RulesetCreatedAttr, RulesetStatus, Scope,
};

use crate::policy::Policy;

/// The oldest ABI Light accepts.
const FLOOR: ABI = ABI::V6;
/// The oldest ABI the daemon's confined git accepts: ABI 2 lets git rename its lock and
/// temporary files into place across directories. What newer kernels add, it uses when they
/// have it: ABI 3 truncation by path (git unlinks and creates instead), ABI 4 TCP, ABI 6
/// signal and abstract-socket scoping. Nothing git runs here needs any of those.
const CONFINE_FLOOR: ABI = ABI::V2;
/// The newest ABI this code knows; rights beyond the floor are best effort.
const LATEST: ABI = ABI::V9;

/// Restrict this process to `policy`: read (and execute) `policy.read`, read and write
/// `policy.write`, connect over TCP only to `policy.connect_ports`, bind no TCP port, and
/// stay inside its own scope for signals and abstract Unix sockets.
pub fn restrict(policy: &Policy) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("Landlock: {e}");
    let mut ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(FLOOR))
        .map_err(|e| fail(&e))?
        .handle_access(AccessNet::from_all(FLOOR))
        .map_err(|e| fail(&e))?
        .scope(Scope::from_all(FLOOR))
        .map_err(|e| fail(&e))?
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(LATEST))
        .map_err(|e| fail(&e))?
        .create()
        .map_err(|e| fail(&e))?;
    ruleset = add_paths(ruleset, policy)?;
    for port in &policy.connect_ports {
        ruleset = ruleset
            .add_rule(NetPort::new(*port, AccessNet::ConnectTcp))
            .map_err(|e| fail(&e))?;
    }
    let status = ruleset.restrict_self().map_err(|e| fail(&e))?;
    match status.ruleset {
        RulesetStatus::NotEnforced => Err(fail(&"the kernel did not enforce the ruleset")),
        RulesetStatus::FullyEnforced | RulesetStatus::PartiallyEnforced => Ok(()),
    }
}

/// Confine this process to `policy`'s paths, for the daemon's own git: read (and execute)
/// `policy.read`, read and write `policy.write`, and, where the kernel can enforce it, no TCP
/// and no signals or abstract sockets beyond its own scope. No seccomp filter: git is the
/// daemon's, and only which files it can reach matters.
pub fn confine(policy: &Policy) -> Result<(), String> {
    let fail = |e: &dyn std::fmt::Display| format!("Landlock: {e}");
    let ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::HardRequirement)
        .handle_access(AccessFs::from_all(CONFINE_FLOOR))
        .map_err(|e| fail(&e))?
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(LATEST))
        .map_err(|e| fail(&e))?
        .handle_access(AccessNet::from_all(LATEST))
        .map_err(|e| fail(&e))?
        .scope(Scope::from_all(LATEST))
        .map_err(|e| fail(&e))?
        .create()
        .map_err(|e| fail(&e))?;
    let status = add_paths(ruleset, policy)?
        .restrict_self()
        .map_err(|e| fail(&e))?;
    match status.ruleset {
        RulesetStatus::NotEnforced => Err(fail(&"the kernel did not enforce the ruleset")),
        RulesetStatus::FullyEnforced | RulesetStatus::PartiallyEnforced => Ok(()),
    }
}

/// Allow `policy.read` and `policy.write` in `ruleset`.
fn add_paths(
    mut ruleset: landlock::RulesetCreated,
    policy: &Policy,
) -> Result<landlock::RulesetCreated, String> {
    let fail = |e: &dyn std::fmt::Display| format!("Landlock: {e}");
    for (paths, access) in [
        (&policy.read, AccessFs::from_read(LATEST)),
        (&policy.write, AccessFs::from_all(LATEST)),
    ] {
        for path in paths {
            let fd = match PathFd::new(path) {
                Ok(fd) => fd,
                // A path that vanished since the daemon listed it grants nothing.
                Err(_) if !path.exists() => continue,
                Err(e) => return Err(fail(&e)),
            };
            let access = if path.is_dir() {
                access
            } else {
                access & AccessFs::from_file(LATEST)
            };
            ruleset = ruleset
                .add_rule(PathBeneath::new(fd, access))
                .map_err(|e| fail(&e))?;
        }
    }
    Ok(ruleset)
}
