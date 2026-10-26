//! The filesystem a sandboxed process sees, resolved from a [`Spec`]. Every backend expresses
//! the same view in its own terms: Landlock rules, bubblewrap mounts, or a Seatbelt profile.

use std::path::{Path, PathBuf};

use crate::{SandboxError, Spec, WorktreeMount};

/// System directories visible read-only, when they exist.
#[cfg(target_os = "linux")]
pub const SYSTEM_READ_ONLY: &[&str] = &[
    "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/libx32", "/etc", "/opt", "/nix",
];

/// System directories visible read-only, when they exist.
#[cfg(target_os = "macos")]
pub const SYSTEM_READ_ONLY: &[&str] = &[
    "/usr",
    "/bin",
    "/sbin",
    "/System",
    "/Library",
    "/private/etc",
    "/opt",
    "/private/var/db/timezone",
    "/private/var/select",
];

/// Toolchain and cache directories visible read-only, when they exist: `CARGO_HOME` (default
/// `~/.cargo`), `RUSTUP_HOME` (default `~/.rustup`), and `~/.npm`.
pub fn toolchains(home: &Path, var: impl Fn(&str) -> Option<String>) -> Toolchains {
    let pick = |name: &str, default: &str| {
        var(name)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(default))
    };
    Toolchains {
        cargo_home: pick("CARGO_HOME", ".cargo"),
        rustup_home: pick("RUSTUP_HOME", ".rustup"),
        npm_cache: home.join(".npm"),
    }
}

/// Whether every tier already shows `path` read-only, as a system or toolchain file, given the
/// user's `home` and the daemon's environment `var`. Both `path` and what it resolves to must
/// be there, so a link can't lead out. Callers use it to keep paths a project names, such as
/// hook programs, inside what the sandbox shows instead of widening it. Blocking.
pub fn shows_read_only(home: &Path, path: &Path, var: impl Fn(&str) -> Option<String>) -> bool {
    let Ok(target) = std::fs::canonicalize(path) else {
        return false;
    };
    let tools = toolchains(home, var);
    let roots: Vec<PathBuf> = SYSTEM_READ_ONLY
        .iter()
        .map(PathBuf::from)
        .chain(tools.all().into_iter().map(Path::to_path_buf))
        .flat_map(|root| {
            let real = canonical(&root);
            std::iter::once(root).chain(real)
        })
        .collect();
    let inside = |p: &Path| p.is_absolute() && roots.iter().any(|root| p.starts_with(root));
    inside(path) && inside(&target)
}

/// Where the toolchains live, whether or not they exist.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toolchains {
    pub cargo_home: PathBuf,
    pub rustup_home: PathBuf,
    pub npm_cache: PathBuf,
}

impl Toolchains {
    fn all(&self) -> [&Path; 3] {
        [&self.cargo_home, &self.rustup_home, &self.npm_cache]
    }
}

/// A resolved, canonical filesystem view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct View {
    /// Existing system directories.
    pub system: Vec<PathBuf>,
    /// Existing toolchain directories, canonical.
    pub toolchains: Vec<PathBuf>,
    /// `CARGO_HOME` and `RUSTUP_HOME` to set, for the toolchains that exist.
    pub toolchain_env: Vec<(String, String)>,
    /// The spec's extra read-only paths that exist, canonical.
    pub extra: Vec<PathBuf>,
    /// The spec's extra read-write paths that exist, canonical.
    pub extra_rw: Vec<PathBuf>,
    /// Canonical worktrees and git dirs.
    pub worktrees: Vec<WorktreeMount>,
    /// The user's real home, canonical when it exists.
    pub home: PathBuf,
    /// The sandbox's private home and temp dirs, canonical.
    pub scratch_home: PathBuf,
    pub scratch_tmp: PathBuf,
}

impl View {
    /// Resolve `spec`, creating the scratch dirs (mode 0700). Blocking.
    pub fn new(spec: &Spec) -> Result<Self, SandboxError> {
        if spec.worktrees.is_empty() {
            return Err(SandboxError::Invalid(
                "a sandbox needs at least one worktree".into(),
            ));
        }
        let scratch_home = private_dir(&spec.scratch.join("home"))?;
        let scratch_tmp = private_dir(&spec.scratch.join("tmp"))?;
        let home = canonical(&spec.home).unwrap_or_else(|| spec.home.clone());
        let tools = toolchains(&home, |name| std::env::var(name).ok());
        let mut toolchain_env = Vec::new();
        for (name, path) in [
            ("CARGO_HOME", &tools.cargo_home),
            ("RUSTUP_HOME", &tools.rustup_home),
        ] {
            if let Some(path) = canonical(path) {
                toolchain_env.push((name.to_owned(), path.display().to_string()));
            }
        }
        let worktrees = spec
            .worktrees
            .iter()
            .map(|w| {
                Ok(WorktreeMount {
                    path: must_exist(&w.path)?,
                    git_common: must_exist(&w.git_common)?,
                    git_admin: must_exist(&w.git_admin)?,
                })
            })
            .collect::<Result<Vec<_>, SandboxError>>()?;
        Ok(Self {
            system: SYSTEM_READ_ONLY
                .iter()
                .map(PathBuf::from)
                .filter(|p| p.exists())
                .collect(),
            toolchains: tools.all().into_iter().filter_map(canonical).collect(),
            toolchain_env,
            extra: spec.read_only.iter().filter_map(|p| canonical(p)).collect(),
            extra_rw: spec
                .read_write
                .iter()
                .filter_map(|p| canonical(p))
                .collect(),
            worktrees,
            home,
            scratch_home,
            scratch_tmp,
        })
    }

    /// Everything read-only: system, toolchains, extras, and each repo's git dir.
    pub fn read_only(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.system.clone();
        paths.extend(self.toolchains.iter().cloned());
        paths.extend(self.extra.iter().cloned());
        for w in &self.worktrees {
            if !paths.contains(&w.git_common) {
                paths.push(w.git_common.clone());
            }
        }
        paths
    }

    /// Extra read-only paths inside an extra read-write one, which must stay read-only: only
    /// a tier that can mount one over the other (Standard) keeps them so.
    pub fn locked(&self) -> Vec<PathBuf> {
        self.extra
            .iter()
            .filter(|p| self.extra_rw.iter().any(|rw| p.starts_with(rw)))
            .cloned()
            .collect()
    }

    /// Everything read-write: worktrees, their own git dirs, and the scratch dirs.
    pub fn read_write(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for w in &self.worktrees {
            paths.push(w.path.clone());
            paths.push(w.git_admin.clone());
        }
        paths.push(self.scratch_home.clone());
        paths.push(self.scratch_tmp.clone());
        paths.extend(self.extra_rw.iter().cloned());
        paths
    }
}

/// Refuse a view with [`View::locked`] paths in a tier whose rules only add up, so a read-only
/// path inside a writable one would be writable.
pub(crate) fn refuse_locked(view: &View, tier: &str) -> Result<(), crate::SandboxError> {
    match view.locked().first() {
        Some(path) => Err(crate::SandboxError::Invalid(format!(
            "the {tier} sandbox can't keep {} read-only inside a writable directory",
            path.display()
        ))),
        None => Ok(()),
    }
}

fn canonical(path: &Path) -> Option<PathBuf> {
    std::fs::canonicalize(path).ok()
}

fn must_exist(path: &Path) -> Result<PathBuf, SandboxError> {
    std::fs::canonicalize(path)
        .map_err(|e| SandboxError::io(format!("resolving {}", path.display()), e))
}

fn private_dir(path: &Path) -> Result<PathBuf, SandboxError> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|e| SandboxError::io(format!("creating {}", path.display()), e))?;
    must_exist(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_api::model::SandboxTier;

    fn spec(root: &Path) -> Spec {
        let wt = root.join("repo/.worktrees/feat/x");
        let admin = root.join("repo/.git/worktrees/x");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir_all(root.join("home/.cargo")).unwrap();
        Spec {
            tier: SandboxTier::Light,
            worktrees: vec![WorktreeMount {
                path: wt,
                git_common: root.join("repo/.git"),
                git_admin: admin,
            }],
            scratch: root.join("scratch"),
            home: root.join("home"),
            read_only: vec![root.join("missing"), root.join("home/.cargo")],
            read_write: vec![],
            proxy: None,
        }
    }

    #[test]
    fn resolves_paths_and_creates_private_scratch_dirs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let view = View::new(&spec(&root)).unwrap();
        assert_eq!(view.scratch_home, root.join("scratch/home"));
        let mode = std::fs::metadata(&view.scratch_tmp)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        // Missing extras are dropped; the git common dir is read-only, the admin dir writable.
        assert_eq!(view.extra, [root.join("home/.cargo")]);
        assert!(view.read_only().contains(&root.join("repo/.git")));
        assert!(!view.read_write().contains(&root.join("repo/.git")));
        assert!(
            view.read_write()
                .contains(&root.join("repo/.git/worktrees/x"))
        );
        // The main checkout itself is neither.
        assert!(!view.read_only().contains(&root.join("repo")));
        assert!(!view.read_write().contains(&root.join("repo")));
    }

    #[test]
    fn a_missing_worktree_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let mut spec = spec(dir.path());
        spec.worktrees[0].path = dir.path().join("gone");
        assert!(View::new(&spec).is_err());
        spec.worktrees.clear();
        assert!(View::new(&spec).is_err());
    }

    #[test]
    fn toolchains_follow_their_variables() {
        let t = toolchains(Path::new("/h"), |name| {
            (name == "CARGO_HOME").then(|| "/opt/cargo".to_owned())
        });
        assert_eq!(t.cargo_home, PathBuf::from("/opt/cargo"));
        assert_eq!(t.rustup_home, PathBuf::from("/h/.rustup"));
        assert_eq!(t.npm_cache, PathBuf::from("/h/.npm"));
    }
}
