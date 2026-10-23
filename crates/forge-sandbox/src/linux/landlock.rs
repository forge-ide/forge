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
