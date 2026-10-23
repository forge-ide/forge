//! Confines the paths an agent asks for to the session's worktrees.
//!
//! Reads resolve symlinks and must land inside a root. Writes must name a path inside a root
//! with no symlink anywhere below the root, and never touch a `.git` at any depth, however it
//! is spelled. On the wire
//! paths are relative to the root, with forward slashes.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PathError {
    #[error("{0} is not an absolute path")]
    NotAbsolute(PathBuf),
    #[error("{0} is outside the session's worktree")]
    Outside(PathBuf),
    #[error("{0} goes through a symlink; write to its target instead")]
    Symlink(PathBuf),
    #[error("{0} is inside .git")]
    GitDir(PathBuf),
    #[error("{path}: {message}")]
    Io { path: PathBuf, message: String },
}

/// A worktree an agent may use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Root {
    /// Repo name, used to prefix wire paths when a session has several roots.
    pub name: String,
    /// Canonical path.
    pub path: PathBuf,
}

/// A path inside the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub path: PathBuf,
    /// Relative to the root, with forward slashes; prefixed by the repo name when there are
    /// several roots. `.` for the root itself.
    pub display: String,
}

/// The session's worktrees and the agent's working directory.
#[derive(Clone, Debug)]
pub struct Workspace {
    roots: Vec<Root>,
    cwd: PathBuf,
}

impl Workspace {
    /// `roots` must be non-empty; each path is canonicalized. The working directory is the
    /// first root.
    pub fn new(roots: Vec<Root>) -> Result<Self, PathError> {
        let roots = roots
            .into_iter()
            .map(|r| {
                let path = std::fs::canonicalize(&r.path).map_err(|e| PathError::Io {
                    path: r.path.clone(),
                    message: e.to_string(),
                })?;
                Ok(Root { name: r.name, path })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let cwd = roots
            .first()
            .map(|r| r.path.clone())
            .ok_or_else(|| PathError::Outside(PathBuf::new()))?;
        Ok(Self { roots, cwd })
    }

    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    pub fn roots(&self) -> &[Root] {
        &self.roots
    }

    /// An existing file or directory to read, after following symlinks.
    pub fn resolve_read(&self, path: &Path) -> Result<Resolved, PathError> {
        require_absolute(path)?;
        let real = std::fs::canonicalize(path).map_err(|e| io(path, e))?;
        self.inside(&real, path)
    }

    /// An existing directory to run a command in, after following symlinks.
    pub fn resolve_dir(&self, path: &Path) -> Result<Resolved, PathError> {
        let resolved = self.resolve_read(path)?;
        if !resolved.path.is_dir() {
            return Err(PathError::Io {
                path: path.to_path_buf(),
                message: "not a directory".into(),
            });
        }
        Ok(resolved)
    }

    /// A file to create or overwrite. Missing parent directories are allowed; the caller
    /// creates them and should open the file without following a final symlink.
    pub fn resolve_write(&self, path: &Path) -> Result<Resolved, PathError> {
        require_absolute(path)?;
        let normal = normalize(path);
        let (root, rel) = self
            .roots
            .iter()
            .find_map(|r| normal.strip_prefix(&r.path).ok().map(|rel| (r, rel)))
            .ok_or_else(|| PathError::Outside(path.to_path_buf()))?;
        if rel.as_os_str().is_empty() {
            return Err(PathError::Outside(path.to_path_buf()));
        }
        if touches_git_dir(rel) {
            return Err(PathError::GitDir(path.to_path_buf()));
        }
        let mut current = root.path.clone();
        for component in rel.components() {
            current.push(component);
            match std::fs::symlink_metadata(&current) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(PathError::Symlink(path.to_path_buf()));
                }
                Ok(_) => {}
                // Everything below a missing directory is missing too.
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => break,
                Err(e) => return Err(io(path, e)),
            }
        }
        Ok(Resolved {
            display: self.display(root, rel),
            path: normal,
        })
    }

    fn inside(&self, real: &Path, asked: &Path) -> Result<Resolved, PathError> {
        let (root, rel) = self
            .roots
            .iter()
            .find_map(|r| real.strip_prefix(&r.path).ok().map(|rel| (r, rel)))
            .ok_or_else(|| PathError::Outside(asked.to_path_buf()))?;
        if touches_git_dir(rel) {
            return Err(PathError::GitDir(asked.to_path_buf()));
        }
        Ok(Resolved {
            display: self.display(root, rel),
            path: real.to_path_buf(),
        })
    }

    fn display(&self, root: &Root, rel: &Path) -> String {
        let rel: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        match (self.roots.len() > 1, rel.is_empty()) {
            (false, true) => ".".into(),
            (false, false) => rel.join("/"),
            (true, true) => root.name.clone(),
            (true, false) => format!("{}/{}", root.name, rel.join("/")),
        }
    }
}

/// Whether any component of `rel` names a `.git` file or directory, including spellings that
/// a case-insensitive filesystem (the macOS default) or HFS+, which ignores some zero-width
/// characters, would resolve to one.
fn touches_git_dir(rel: &Path) -> bool {
    rel.components().any(|c| match c {
        Component::Normal(name) => name.to_str().is_some_and(|name| {
            let visible: String = name.chars().filter(|c| !is_hfs_ignorable(*c)).collect();
            visible.eq_ignore_ascii_case(".git")
        }),
        _ => false,
    })
}

/// Characters HFS+ drops when comparing names (the set git guards against, CVE-2014-9390).
fn is_hfs_ignorable(c: char) -> bool {
    matches!(c, '\u{200c}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{206a}'..='\u{206f}' | '\u{feff}')
}

fn require_absolute(path: &Path) -> Result<(), PathError> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(PathError::NotAbsolute(path.to_path_buf()))
    }
}

/// Resolve `.` and `..` without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn io(path: &Path, e: std::io::Error) -> PathError {
    PathError::Io {
        path: path.to_path_buf(),
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        outside: PathBuf,
        ws: Workspace,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let root = base.join("wt");
        let outside = base.join("outside");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(root.join("src/a.rs"), "a").unwrap();
        std::fs::write(outside.join("secret"), "s").unwrap();
        symlink(&outside, root.join("out-link")).unwrap();
        symlink(root.join("src/a.rs"), root.join("in-link")).unwrap();
        let ws = Workspace::new(vec![Root {
            name: "wt".into(),
            path: root.clone(),
        }])
        .unwrap();
        Fixture {
            _dir: dir,
            root,
            outside,
            ws,
        }
    }

    #[test]
    fn reads_resolve_inside_the_root() {
        let fx = fixture();
        let r = fx.ws.resolve_read(&fx.root.join("src/a.rs")).unwrap();
        assert_eq!(r.display, "src/a.rs");
        // A symlink to a file inside reads as its target.
        assert_eq!(
            fx.ws
                .resolve_read(&fx.root.join("in-link"))
                .unwrap()
                .display,
            "src/a.rs"
        );
        assert_eq!(fx.ws.resolve_dir(&fx.root).unwrap().display, ".");
    }

    #[test]
    fn reads_cannot_escape() {
        let fx = fixture();
        for path in [
            fx.outside.join("secret"),
            fx.root.join("out-link/secret"),
            fx.root.join("../outside/secret"),
            fx.root.join(".git"),
        ] {
            assert!(fx.ws.resolve_read(&path).is_err(), "{}", path.display());
        }
        assert!(matches!(
            fx.ws.resolve_read(Path::new("src/a.rs")),
            Err(PathError::NotAbsolute(_))
        ));
        assert!(fx.ws.resolve_dir(&fx.root.join("src/a.rs")).is_err());
    }

    #[test]
    fn writes_stay_inside_and_never_follow_symlinks() {
        let fx = fixture();
        let r = fx.ws.resolve_write(&fx.root.join("new/dir/b.rs")).unwrap();
        assert_eq!(r.display, "new/dir/b.rs");
        assert_eq!(r.path, fx.root.join("new/dir/b.rs"));
        assert_eq!(
            fx.ws
                .resolve_write(&fx.root.join("src/./x/../a.rs"))
                .unwrap()
                .display,
            "src/a.rs"
        );

        let cases = [
            (fx.outside.join("x"), "outside"),
            (fx.root.join("../outside/x"), "outside"),
            (fx.root.join("out-link/x"), "symlink"),
            (fx.root.join("in-link"), "symlink"),
            (fx.root.join(".git/config"), "git"),
            (fx.root.clone(), "outside"),
        ];
        for (path, why) in cases {
            let err = fx.ws.resolve_write(&path).unwrap_err();
            let ok = match why {
                "outside" => matches!(err, PathError::Outside(_)),
                "symlink" => matches!(err, PathError::Symlink(_)),
                _ => matches!(err, PathError::GitDir(_)),
            };
            assert!(ok, "{}: {err:?}", path.display());
        }
    }

    #[test]
    fn git_dirs_are_blocked_whatever_the_spelling() {
        let fx = fixture();
        // Case-insensitive filesystems (macOS by default) and HFS+, which ignores some
        // zero-width characters, would resolve these to the real `.git`.
        for name in [
            ".GIT/config",
            ".Git/hooks/pre-commit",
            "\u{200c}.git/config",
            ".gi\u{feff}t/x",
        ] {
            let err = fx.ws.resolve_write(&fx.root.join(name)).unwrap_err();
            assert!(matches!(err, PathError::GitDir(_)), "{name:?}: {err:?}");
        }
        // Names that merely contain `.git` are fine.
        fx.ws
            .resolve_write(&fx.root.join(".github/workflows/ci.yml"))
            .unwrap();
        fx.ws
            .resolve_write(&fx.root.join("src/.gitignore"))
            .unwrap();
    }

    #[test]
    fn several_roots_prefix_paths_with_the_repo_name() {
        let fx = fixture();
        let other = fx.outside.clone();
        let ws = Workspace::new(vec![
            Root {
                name: "api".into(),
                path: fx.root.clone(),
            },
            Root {
                name: "web".into(),
                path: other.clone(),
            },
        ])
        .unwrap();
        assert_eq!(ws.cwd(), fx.root);
        assert_eq!(
            ws.resolve_read(&other.join("secret")).unwrap().display,
            "web/secret"
        );
        assert_eq!(
            ws.resolve_write(&fx.root.join("x")).unwrap().display,
            "api/x"
        );
    }
}
