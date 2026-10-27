//! The daemon's own git, confined to the paths a command needs (see `forge_git::confine`):
//! on Linux through the helper's `confine` mode (Landlock file rules), on macOS through a
//! Seatbelt profile. Either way the rules apply to the resolved path a file is opened at, so a
//! symlink swapped into a worktree leads only to paths the command could reach anyway.
//!
//! Every confined git can read the system dirs and git itself (found once, when the daemon
//! starts), write `/dev/null`, and gets only the daemon's allowlisted environment and `HOME`.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Unavailable;

/// Starts git confined. Probed once; see [`GitConfinement::probe`].
#[derive(Debug)]
pub struct GitConfinement {
    /// The git the daemon runs, resolved through `PATH` at probe time.
    git: PathBuf,
    /// What every confined git may read.
    base: Vec<PathBuf>,
    kind: Kind,
}

#[derive(Debug)]
enum Kind {
    #[cfg(target_os = "linux")]
    Landlock { helper: PathBuf },
    #[cfg(target_os = "macos")]
    Seatbelt,
}

impl GitConfinement {
    /// Whether this machine can confine git, and what every confined git may read. `helper`
    /// is the `forge-sandbox-exec` binary. Blocking: runs git and a probe.
    pub fn probe(helper: PathBuf) -> Result<Self, Unavailable> {
        let unavailable = |reason: String, fix: &str| Unavailable {
            reason,
            fix: fix.to_owned(),
        };
        let git = forge_git::confine::git_program()
            .map_err(|_| unavailable("git is not on PATH".into(), "install git"))?
            .to_path_buf();
        let kind = probe_kind(helper).map_err(|(reason, fix)| unavailable(reason, fix))?;
        let base = base_paths(&git);
        Ok(Self { git, base, kind })
    }

    /// A command that runs git able to read `read`, read and write `write`, and otherwise
    /// only what every confined git may. Arguments and environment for git go on after.
    pub fn command(&self, read: &[PathBuf], write: &[PathBuf]) -> Command {
        let mut cmd = self.launcher(read, write);
        git_env(&mut cmd);
        cmd
    }

    fn launcher(&self, read: &[PathBuf], write: &[PathBuf]) -> Command {
        let mut readable = self.base.clone();
        readable.extend(read.iter().filter_map(|p| canonical(p)));
        let mut writable: Vec<PathBuf> = write.iter().filter_map(|p| canonical(p)).collect();
        writable.push(PathBuf::from("/dev/null"));
        match &self.kind {
            #[cfg(target_os = "linux")]
            Kind::Landlock { helper } => {
                let policy = crate::policy::Policy {
                    mode: crate::policy::Mode::Confine,
                    read: readable,
                    write: writable,
                    connect_ports: vec![],
                    forward: None,
                };
                let mut cmd = Command::new(helper);
                cmd.arg("confine")
                    .arg(policy.to_json())
                    .arg("--")
                    .arg(&self.git);
                cmd
            }
            #[cfg(target_os = "macos")]
            Kind::Seatbelt => {
                let mut cmd = Command::new(crate::macos::SANDBOX_EXEC);
                cmd.arg("-p")
                    .arg(crate::macos::git_profile(&readable, &writable))
                    .arg(&self.git);
                cmd
            }
        }
    }
}

/// The daemon's git, unconfined, for sessions that run unsandboxed, whose agent can reach
/// whatever it could: no paths are enforced, but it runs with the same environment as
/// confined git.
#[derive(Debug)]
pub struct PlainGit {
    git: PathBuf,
}

impl PlainGit {
    pub fn new() -> Result<Self, Unavailable> {
        let git = forge_git::confine::git_program()
            .map_err(|_| Unavailable {
                reason: "git is not on PATH".into(),
                fix: "install git".into(),
            })?
            .to_path_buf();
        Ok(Self { git })
    }
}

impl forge_git::Confine for PlainGit {
    fn git(&self, _read: &[PathBuf], _write: &[PathBuf]) -> Result<Command, String> {
        let mut cmd = Command::new(&self.git);
        git_env(&mut cmd);
        Ok(cmd)
    }
}

/// Give `cmd` only the daemon's git environment: no `GIT_TRACE` file, `GIT_SSH_COMMAND`,
/// agent socket, or token the user's shell exported before starting the daemon, and no
/// relative `PATH` entry by which a filter would run a program from the worktree.
fn git_env(cmd: &mut Command) {
    cmd.env_clear().envs(crate::env::for_git(std::env::vars()));
}

impl forge_git::Confine for GitConfinement {
    fn git(&self, read: &[PathBuf], write: &[PathBuf]) -> Result<Command, String> {
        Ok(self.command(read, write))
    }
}

#[cfg(target_os = "linux")]
fn probe_kind(helper: PathBuf) -> Result<Kind, (String, &'static str)> {
    let fix = "Forge confines its own git with Landlock ABI 2 or newer (Linux 5.19+) enabled \
               at boot (\"landlock\" in the kernel's lsm= list); until then only unsandboxed \
               sessions can commit or checkpoint";
    if !helper.is_file() {
        return Err((format!("{} is missing", helper.display()), fix));
    }
    let out = Command::new(&helper)
        .arg("check-confine")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| (format!("running {}: {e}", helper.display()), fix))?;
    if out.status.success() {
        Ok(Kind::Landlock { helper })
    } else {
        Err((
            format!(
                "Landlock could not confine git ({})",
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            fix,
        ))
    }
}

#[cfg(target_os = "macos")]
fn probe_kind(_helper: PathBuf) -> Result<Kind, (String, &'static str)> {
    let works = Command::new(crate::macos::SANDBOX_EXEC)
        .args(["-p", "(version 1)(allow default)", "/usr/bin/true"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if works {
        Ok(Kind::Seatbelt)
    } else {
        Err((
            format!(
                "{} could not apply a Seatbelt profile",
                crate::macos::SANDBOX_EXEC
            ),
            "check that sandbox-exec exists and works on this version of macOS",
        ))
    }
}

/// What every confined git may read: the system dirs, and git, its helper programs, and Git
/// LFS where they live outside them. Never the user's git config: confined git reads a
/// snapshot of the settings it needs instead (`forge_git::confine::snapshot`), since files
/// beside the config (`~/.config/git/credentials`) and in it (a token in a URL) hold secrets.
fn base_paths(git: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = crate::SYSTEM_READ_ONLY
        .iter()
        .filter_map(|p| canonical(Path::new(p)))
        .collect();
    let in_system = |p: &Path| paths_contain(&paths, p);
    let mut extra = vec![git.to_path_buf()];
    let exec_path = Command::new(git)
        .arg("--exec-path")
        .stdin(Stdio::null())
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
    extra.extend(exec_path.and_then(|p| canonical(Path::new(&p))));
    extra.extend(which("git-lfs"));
    extra.extend(developer_dirs());
    let extra: Vec<PathBuf> = extra.into_iter().filter(|p| !in_system(p)).collect();
    paths.extend(extra);
    paths.sort();
    paths.dedup();
    paths
}

fn paths_contain(paths: &[PathBuf], path: &Path) -> bool {
    paths.iter().any(|p| path.starts_with(p))
}

/// On macOS, `/usr/bin/git` runs the git of the selected developer dir, which may be an Xcode
/// app outside the system dirs.
#[cfg(target_os = "macos")]
fn developer_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = canonical(Path::new("/private/var/db/xcode_select_link"))
        .into_iter()
        .collect();
    if let Ok(out) = Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .stdin(Stdio::null())
        .output()
        && out.status.success()
    {
        let selected = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        dirs.extend(canonical(Path::new(&selected)));
    }
    dirs
}

#[cfg(not(target_os = "macos"))]
fn developer_dirs() -> Vec<PathBuf> {
    Vec::new()
}

fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

/// `name` on the daemon's `PATH`, by an absolute entry only, as it resolves.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .and_then(|p| canonical(&p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_confined_git_reads_the_system_and_git_but_nothing_of_the_home() {
        let git = which("git").unwrap();
        let base = base_paths(&git);
        assert!(paths_contain(&base, &git), "{base:?}");
        if let Some(home) = std::env::var_os("HOME").and_then(|h| canonical(Path::new(&h))) {
            // Git itself may live under the home; nothing else there is granted.
            for path in base.iter().filter(|p| p.starts_with(&home)) {
                assert!(
                    path == &git || git.starts_with(path) || path.ends_with("git-lfs"),
                    "{path:?} in {base:?}"
                );
                assert!(!path.ends_with(".gitconfig") && !path.ends_with(".config/git"));
            }
        }
    }
}
