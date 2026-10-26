//! The filesystem a sandboxed process sees, resolved from a [`Spec`]. Every backend expresses
//! the same view in its own terms: Landlock rules, bubblewrap mounts, or a Seatbelt profile.

use std::path::{Path, PathBuf};

use crate::{HomeLock, SandboxError, Spec, WorktreeMount};

/// The only writable dirs of the sandbox home, whose root is read-only: the agents' own
/// config dirs, caches, and the package managers' homes, none of which anything runs from by
/// itself. Not `.config` (fish and git read it), `.cargo` (its `config.toml` names runners),
/// `.gradle` (its `init.d` runs on every build), or the root's dotfiles, which shells and git
/// read. Programs in `.local` (Python's user site) are kept off by `PYTHONNOUSERSITE`.
pub const HOME_WRITABLE: &[&str] = &[
    ".claude",
    ".codex",
    ".gemini",
    ".cache",
    ".local",
    ".npm",
    ".bun",
    ".deno",
    ".yarn",
    ".pnpm-store",
    "go",
    ".m2",
];

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
    /// The sandbox's private home and temp dirs, canonical. Only the home's
    /// [`HOME_WRITABLE`] dirs are writable.
    pub scratch_home: PathBuf,
    pub scratch_tmp: PathBuf,
    /// Paths inside those that stay read-only, relative to the home.
    pub home_locked: Vec<HomeLock>,
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
        prepare_home(&scratch_home, &spec.home_locked)?;
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
            home_locked: spec.home_locked.clone(),
        })
    }

    /// Everything read-only: system, toolchains, extras, each repo's git dir, and the
    /// sandbox home.
    pub fn read_only(&self) -> Vec<PathBuf> {
        let mut paths: Vec<PathBuf> = self.system.clone();
        paths.push(self.scratch_home.clone());
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

    /// Everything read-write: worktrees, their own git dirs, the home's writable dirs, and the
    /// temp dir.
    pub fn read_write(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for w in &self.worktrees {
            paths.push(w.path.clone());
            paths.push(w.git_admin.clone());
        }
        paths.extend(HOME_WRITABLE.iter().map(|d| self.scratch_home.join(d)));
        paths.push(self.scratch_tmp.clone());
        paths.extend(self.extra_rw.iter().cloned());
        paths
    }

    /// The home's locked paths, where the scratch home is.
    pub fn locked_in_home(&self) -> Vec<(PathBuf, bool)> {
        self.home_locked
            .iter()
            .map(|l| (self.scratch_home.join(&l.path), l.dir))
            .collect()
    }

    /// `path` (a `PATH` value) without the entries a sandboxed process could have written
    /// to: relative or empty ones (the working directory), and those inside a writable path,
    /// as named or as they resolve on the host, wherever a tier shows it (Standard shows the
    /// home's writable dirs at the real home's path, and the temp dir at `/tmp`). Entries that
    /// don't exist, and those under `/proc`, `/dev`, or `/sys` (whose meaning inside the
    /// sandbox the host can't resolve), are dropped too. A program found in a dropped entry
    /// could run instead of the system one Forge decided. Blocking: resolves each entry.
    pub fn safe_path(&self, path: &str) -> String {
        self.safe_path_by(path, |p| std::fs::canonicalize(p).ok())
    }

    fn safe_path_by(&self, path: &str, resolve: impl Fn(&Path) -> Option<PathBuf>) -> String {
        let mut writable = self.read_write();
        writable.extend(HOME_WRITABLE.iter().map(|d| self.home.join(d)));
        writable.push(PathBuf::from("/tmp"));
        let inside = |p: &Path| writable.iter().any(|w| p.starts_with(w));
        path.split(':')
            .filter(|entry| {
                let entry = Path::new(entry);
                let plain = entry.is_absolute()
                    && !entry
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                    && !["/proc", "/dev", "/sys"]
                        .iter()
                        .any(|p| entry.starts_with(p));
                plain && !inside(entry) && resolve(entry).is_some_and(|real| !inside(&real))
            })
            .collect::<Vec<_>>()
            .join(":")
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

/// Make the sandbox home hold only its writable dirs, each a real directory, and the locked
/// paths inside them, fresh and empty. What an earlier launch left at the root (a profile, a
/// link where a writable dir should be) is removed; nothing is followed. Nothing may run in
/// the session meanwhile.
fn prepare_home(home: &Path, locked: &[HomeLock]) -> Result<(), SandboxError> {
    let failed = |what: &Path, e| SandboxError::io(format!("preparing {}", what.display()), e);
    let entries = std::fs::read_dir(home).map_err(|e| failed(home, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| failed(home, e))?;
        let keep = HOME_WRITABLE.iter().any(|d| entry.file_name() == *d)
            && entry.file_type().is_ok_and(|t| t.is_dir());
        if !keep {
            remove(&entry.path()).map_err(|e| failed(&entry.path(), e))?;
        }
    }
    for dir in HOME_WRITABLE {
        private_dir(&home.join(dir))?;
    }
    for lock in locked {
        let mut parts = lock.path.components();
        let inside = parts.next().is_some_and(|first| {
            HOME_WRITABLE
                .iter()
                .any(|d| first == std::path::Component::Normal(d.as_ref()))
        }) && parts.clone().next().is_some()
            && parts.all(|c| matches!(c, std::path::Component::Normal(_)));
        if !inside {
            return Err(SandboxError::Invalid(format!(
                "{} is not inside a writable dir of the sandbox home",
                lock.path.display()
            )));
        }
        let path = home.join(&lock.path);
        // Each parent a real directory, made if missing.
        let mut dir = home.to_path_buf();
        for part in lock.path.parent().into_iter().flat_map(Path::components) {
            dir.push(part);
            match std::fs::symlink_metadata(&dir) {
                Ok(meta) if meta.is_dir() => {}
                Ok(_) => {
                    remove(&dir).map_err(|e| failed(&dir, e))?;
                    private_dir(&dir)?;
                }
                Err(_) => {
                    private_dir(&dir)?;
                }
            }
        }
        if std::fs::symlink_metadata(&path).is_ok() {
            remove(&path).map_err(|e| failed(&path, e))?;
        }
        if lock.dir {
            private_dir(&path)?;
        } else {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
                .map_err(|e| failed(&path, e))?;
        }
    }
    Ok(())
}

/// Remove `path`, a directory with what is in it or anything else; never follows a link.
fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
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
            home_locked: vec![],
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
    fn path_keeps_nothing_the_sandbox_can_write() {
        let view = View {
            system: vec!["/usr".into()],
            toolchains: vec!["/home/u/.cargo".into()],
            toolchain_env: vec![],
            extra: vec![],
            extra_rw: vec![],
            worktrees: vec![WorktreeMount {
                path: "/home/u/repo/.worktrees/feat/x".into(),
                git_common: "/home/u/repo/.git".into(),
                git_admin: "/home/u/repo/.git/worktrees/x".into(),
            }],
            home: "/home/u".into(),
            scratch_home: "/data/s/home".into(),
            scratch_tmp: "/data/s/tmp".into(),
            home_locked: vec![],
        };
        let home = Path::new("/home/u");
        let path = [
            "/usr/bin".to_owned(),
            format!("{}/.local/bin", home.display()),
            "/data/s/home/.local/bin".to_owned(),
            format!("{}/.cargo/bin", home.display()),
            "/home/u/repo/.worktrees/feat/x/bin".to_owned(),
            "/tmp/bin".to_owned(),
            "/usr/../tmp".to_owned(),
            "node_modules/.bin".to_owned(),
            String::new(),
            "/opt/tool/bin".to_owned(),
        ]
        .join(":");
        // Entries resolve as named, but for a link into a writable dir and one that's gone.
        let resolve = |p: &Path| match p.to_str() {
            Some("/opt/link/bin") => Some(PathBuf::from("/data/s/home/.local/bin")),
            Some("/opt/gone/bin") => None,
            _ => Some(p.to_path_buf()),
        };
        let path = format!(
            "{path}:/opt/link/bin:/opt/gone/bin:/proc/self/root/usr/bin:/tmpx/bin:/devtools/bin"
        );
        assert_eq!(
            view.safe_path_by(&path, resolve),
            format!(
                "/usr/bin:{}/.cargo/bin:/opt/tool/bin:/tmpx/bin:/devtools/bin",
                home.display()
            )
        );
    }

    #[test]
    fn the_home_keeps_only_its_writable_dirs_and_fresh_locks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let scratch_home = root.join("scratch/home");
        std::fs::create_dir_all(scratch_home.join(".claude/jobs")).unwrap();
        std::fs::write(scratch_home.join(".claude/jobs/run.sh"), "x").unwrap();
        std::fs::write(scratch_home.join(".profile"), "x").unwrap();
        std::fs::create_dir_all(scratch_home.join(".config/fish")).unwrap();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, scratch_home.join(".cache")).unwrap();
        let mut spec = spec(&root);
        spec.home_locked = vec![
            HomeLock {
                path: ".claude/jobs".into(),
                dir: true,
            },
            HomeLock {
                path: ".claude/daemon.json".into(),
                dir: false,
            },
        ];
        View::new(&spec).unwrap();
        assert!(!scratch_home.join(".profile").exists());
        assert!(!scratch_home.join(".config").exists());
        // A link where a writable dir goes is replaced by a real dir, never followed.
        let cache = std::fs::symlink_metadata(scratch_home.join(".cache")).unwrap();
        assert!(cache.is_dir());
        assert!(outside.exists());
        assert_eq!(
            std::fs::read_dir(scratch_home.join(".claude/jobs"))
                .unwrap()
                .count(),
            0
        );
        assert!(scratch_home.join(".claude/daemon.json").is_file());
        for d in HOME_WRITABLE {
            assert!(scratch_home.join(d).is_dir(), "{d}");
        }
        // A lock must be inside a writable dir.
        for bad in [".profile", ".claude", ".claude/../x", ".config/fish"] {
            spec.home_locked = vec![HomeLock {
                path: bad.into(),
                dir: true,
            }];
            assert!(View::new(&spec).is_err(), "{bad}");
        }
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
