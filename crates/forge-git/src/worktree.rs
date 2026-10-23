//! Session worktrees under `<repo>/.worktrees/<branch>/`, through the git CLI.
//!
//! Every call is blocking; async callers run them on the blocking pool. Repo hooks never run:
//! each command points `core.hooksPath` at a directory with no hooks, since checking out a
//! worktree would otherwise run the repo's `post-checkout`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Folder in each repo root that holds session worktrees.
pub const WORKTREES_DIR: &str = ".worktrees";

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("repo path {path:?} resolves to {resolved}, outside the project root")]
    Escapes { path: String, resolved: PathBuf },
    #[error("repo path {path:?}: {source}")]
    Missing {
        path: String,
        source: std::io::Error,
    },
    #[error("{0} is not the root of a git repository")]
    NotARepo(PathBuf),
    #[error("{0:?} is not a valid branch name")]
    InvalidBranch(String),
    #[error("base branch {0:?} has no commits to start a worktree from")]
    NoBase(String),
    #[error("branch {branch:?} is checked out at {path}")]
    CheckedOut { branch: String, path: PathBuf },
    #[error("{path} is not a worktree of branch {branch:?}")]
    NotAWorktree { path: PathBuf, branch: String },
    #[error("git {args}: {stderr}")]
    Command { args: String, stderr: String },
    #[error("{context}: {source}")]
    Io {
        context: String,
        source: std::io::Error,
    },
}

/// How to get a worktree for a branch name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateMode {
    /// Create the branch from the base and a new worktree for it.
    New,
    /// Check out the existing branch in a new worktree.
    ReuseBranch,
    /// Use the existing worktree at the branch's path.
    Adopt,
}

/// What already uses a branch name in one repo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NameUse {
    pub branch_exists: bool,
    /// `<repo>/.worktrees/<branch>` exists on disk.
    pub worktree_exists: bool,
    /// The worktree, if any, that has the branch checked out (possibly the main checkout).
    pub checked_out_at: Option<PathBuf>,
}

impl NameUse {
    pub fn is_free(&self) -> bool {
        !self.branch_exists && !self.worktree_exists && self.checked_out_at.is_none()
    }
}

/// A worktree git knows about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    /// Short branch name, or `None` for a detached HEAD.
    pub branch: Option<String>,
}

/// Worktree and branch management, behind a trait so callers can be tested with fakes.
pub trait Worktrees: Send + Sync {
    /// Resolve `path` (relative, from `forge.toml`) under `project_root`, following symlinks,
    /// and check it is still inside the root and is a repository root.
    fn resolve_repo(&self, project_root: &Path, path: &str) -> Result<PathBuf, GitError>;
    fn validate_branch(&self, repo: &Path, branch: &str) -> Result<(), GitError>;
    fn inspect(&self, repo: &Path, branch: &str) -> Result<NameUse, GitError>;
    /// Returns the worktree's path. On failure, anything this call created is removed.
    fn create(
        &self,
        repo: &Path,
        branch: &str,
        base: &str,
        mode: CreateMode,
    ) -> Result<PathBuf, GitError>;
    fn list(&self, repo: &Path) -> Result<Vec<Worktree>, GitError>;
    fn remove(&self, repo: &Path, path: &Path, force: bool) -> Result<(), GitError>;
    /// Delete a branch, even if it isn't merged.
    fn delete_branch(&self, repo: &Path, branch: &str) -> Result<(), GitError>;
}

/// Where the worktree for `branch` lives in `repo`.
pub fn worktree_path(repo: &Path, branch: &str) -> PathBuf {
    repo.join(WORKTREES_DIR).join(branch)
}

/// [`Worktrees`] backed by the `git` executable.
#[derive(Clone, Debug, Default)]
pub struct GitCli;

impl Worktrees for GitCli {
    fn resolve_repo(&self, project_root: &Path, path: &str) -> Result<PathBuf, GitError> {
        let root = std::fs::canonicalize(project_root).map_err(|source| GitError::Missing {
            path: project_root.display().to_string(),
            source,
        })?;
        let resolved =
            std::fs::canonicalize(root.join(path)).map_err(|source| GitError::Missing {
                path: path.to_owned(),
                source,
            })?;
        if !resolved.starts_with(&root) {
            return Err(GitError::Escapes {
                path: path.to_owned(),
                resolved,
            });
        }
        let toplevel = git(&resolved, &["rev-parse", "--show-toplevel"])
            .map_err(|_| GitError::NotARepo(resolved.clone()))?;
        let toplevel = std::fs::canonicalize(toplevel.trim()).map_err(|e| GitError::Io {
            context: format!("resolving the git toplevel of {}", resolved.display()),
            source: e,
        })?;
        if toplevel != resolved {
            return Err(GitError::NotARepo(resolved));
        }
        Ok(resolved)
    }

    fn validate_branch(&self, repo: &Path, branch: &str) -> Result<(), GitError> {
        let invalid = || GitError::InvalidBranch(branch.to_owned());
        if branch.starts_with('-') || branch.split('/').any(|c| c.starts_with('.')) {
            return Err(invalid());
        }
        git(repo, &["check-ref-format", &format!("refs/heads/{branch}")])
            .map(drop)
            .map_err(|_| invalid())
    }

    fn inspect(&self, repo: &Path, branch: &str) -> Result<NameUse, GitError> {
        let branch_exists = branch_exists(repo, branch)?;
        let worktree_exists = worktree_path(repo, branch).symlink_metadata().is_ok();
        let checked_out_at = self
            .list(repo)?
            .into_iter()
            .find(|w| w.branch.as_deref() == Some(branch))
            .map(|w| w.path);
        Ok(NameUse {
            branch_exists,
            worktree_exists,
            checked_out_at,
        })
    }

    fn create(
        &self,
        repo: &Path,
        branch: &str,
        base: &str,
        mode: CreateMode,
    ) -> Result<PathBuf, GitError> {
        self.validate_branch(repo, branch)?;
        let path = worktree_path(repo, branch);
        if mode == CreateMode::Adopt {
            return self.adopt(repo, branch, &path);
        }
        if mode == CreateMode::New
            && git(
                repo,
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("{base}^{{commit}}"),
                ],
            )
            .is_err()
        {
            return Err(GitError::NoBase(base.to_owned()));
        }
        let used = self.inspect(repo, branch)?;
        if let Some(at) = used.checked_out_at {
            return Err(GitError::CheckedOut {
                branch: branch.to_owned(),
                path: at,
            });
        }
        exclude_worktrees_dir(repo)?;
        let target = path.to_string_lossy().into_owned();
        let result = match mode {
            CreateMode::New => git(repo, &["worktree", "add", "-b", branch, &target, base]),
            _ => git(repo, &["worktree", "add", &target, branch]),
        };
        match result {
            Ok(_) => canonical(&path),
            Err(err) => {
                // Only delete a branch this call created, never one that was already there.
                let created = mode == CreateMode::New && !used.branch_exists;
                self.clean_up(repo, &path, created.then_some(branch));
                Err(err)
            }
        }
    }

    fn list(&self, repo: &Path) -> Result<Vec<Worktree>, GitError> {
        let out = git(repo, &["worktree", "list", "--porcelain"])?;
        let mut worktrees = Vec::new();
        for block in out.split("\n\n") {
            let mut path = None;
            let mut branch = None;
            for line in block.lines() {
                if let Some(p) = line.strip_prefix("worktree ") {
                    path = Some(PathBuf::from(p));
                } else if let Some(b) = line.strip_prefix("branch ") {
                    branch = Some(b.strip_prefix("refs/heads/").unwrap_or(b).to_owned());
                }
            }
            if let Some(path) = path {
                worktrees.push(Worktree { path, branch });
            }
        }
        Ok(worktrees)
    }

    fn remove(&self, repo: &Path, path: &Path, force: bool) -> Result<(), GitError> {
        let target = path.to_string_lossy();
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(&target);
        git(repo, &args).map(drop)
    }

    fn delete_branch(&self, repo: &Path, branch: &str) -> Result<(), GitError> {
        git(repo, &["branch", "-D", "--", branch]).map(drop)
    }
}

impl GitCli {
    fn adopt(&self, repo: &Path, branch: &str, path: &Path) -> Result<PathBuf, GitError> {
        let not_worktree = || GitError::NotAWorktree {
            path: path.to_path_buf(),
            branch: branch.to_owned(),
        };
        let resolved = canonical(path).map_err(|_| not_worktree())?;
        let listed = self.list(repo)?.into_iter().any(|w| {
            w.branch.as_deref() == Some(branch)
                && std::fs::canonicalize(&w.path).is_ok_and(|p| p == resolved)
        });
        if listed {
            Ok(resolved)
        } else {
            Err(not_worktree())
        }
    }

    /// Best-effort removal of a worktree and branch that a failed `create` may have left.
    fn clean_up(&self, repo: &Path, path: &Path, branch: Option<&str>) {
        if path.exists()
            && let Err(e) = self.remove(repo, path, true)
        {
            tracing::debug!(error = %e, "removing partial worktree");
            if let Err(e) = std::fs::remove_dir_all(path) {
                tracing::warn!(path = %path.display(), error = %e, "removing partial worktree");
            }
        }
        let _ = git(repo, &["worktree", "prune"]);
        if let Some(branch) = branch
            && branch_exists(repo, branch).unwrap_or(false)
        {
            let _ = git(repo, &["branch", "-D", branch]);
        }
    }
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool, GitError> {
    let mut cmd = command(repo);
    cmd.args([
        "show-ref",
        "--verify",
        "--quiet",
        &format!("refs/heads/{branch}"),
    ]);
    let status = cmd.status().map_err(|source| GitError::Io {
        context: "running git".into(),
        source,
    })?;
    Ok(status.success())
}

/// Add `/.worktrees/` to the repo's `info/exclude`, so worktrees never show as untracked.
/// It lives in the common git dir, so every worktree of the repo shares it.
fn exclude_worktrees_dir(repo: &Path) -> Result<(), GitError> {
    let common = git(repo, &["rev-parse", "--git-common-dir"])?;
    let common = repo.join(common.trim());
    let info = common.join("info");
    let exclude = info.join("exclude");
    let io = |context: &str| {
        let context = format!("{context} {}", exclude.display());
        move |source| GitError::Io { context, source }
    };
    let existing = match std::fs::read_to_string(&exclude) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io("reading")(e)),
    };
    let pattern = format!("/{WORKTREES_DIR}/");
    let present = existing
        .lines()
        .any(|l| l.trim() == pattern || l.trim() == &pattern[1..]);
    if present {
        return Ok(());
    }
    std::fs::create_dir_all(&info).map_err(io("creating the directory for"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&exclude)
        .map_err(io("opening"))?;
    let sep = if existing.is_empty() || existing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    writeln!(file, "{sep}# Forge session worktrees\n{pattern}").map_err(io("writing"))
}

fn canonical(path: &Path) -> Result<PathBuf, GitError> {
    std::fs::canonicalize(path).map_err(|source| GitError::Io {
        context: format!("resolving {}", path.display()),
        source,
    })
}

fn command(repo: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo)
        // Never run the repo's hooks; /dev/null is not a directory, so no hook is found.
        .args(["-c", "core.hooksPath=/dev/null"])
        .env("GIT_TERMINAL_PROMPT", "0")
        // Inherited repository overrides would point git at some other repo.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(std::process::Stdio::null());
    cmd
}

/// Run git in `repo`; its stdout on success.
fn git(repo: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = command(repo)
        .args(args)
        .output()
        .map_err(|source| GitError::Io {
            context: "running git".into(),
            source,
        })?;
    if !output.status.success() {
        return Err(GitError::Command {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// A temp dir holding `repo/`, a git repo on `main` with one commit.
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        run(&repo, &["init", "--quiet", "--initial-branch=main"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        run(&repo, &["add", "a.txt"]);
        run(&repo, &["commit", "--quiet", "-m", "initial"]);
        let repo = repo.canonicalize().unwrap();
        (dir, repo)
    }

    #[test]
    fn creates_lists_and_removes_worktrees_without_touching_status() {
        let (_dir, repo) = repo();
        let git = GitCli;
        assert!(git.inspect(&repo, "feat/x").unwrap().is_free());

        let path = git
            .create(&repo, "feat/x", "main", CreateMode::New)
            .unwrap();
        assert_eq!(path, repo.join(".worktrees/feat/x"));
        assert_eq!(std::fs::read_to_string(path.join("a.txt")).unwrap(), "a\n");
        assert_eq!(run(&path, &["branch", "--show-current"]).trim(), "feat/x");
        assert_eq!(
            run(&repo, &["status", "--porcelain"]),
            "",
            "worktrees are excluded"
        );

        let listed = git.list(&repo).unwrap();
        assert!(listed.iter().any(|w| w.branch.as_deref() == Some("feat/x")));
        let used = git.inspect(&repo, "feat/x").unwrap();
        assert!(used.branch_exists && used.worktree_exists);
        assert_eq!(used.checked_out_at.as_deref(), Some(path.as_path()));

        // Creating again excludes once.
        git.create(&repo, "feat/y", "main", CreateMode::New)
            .unwrap();
        let exclude = std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap();
        assert_eq!(exclude.matches("/.worktrees/").count(), 1);

        git.remove(&repo, &path, false).unwrap();
        assert!(!path.exists());
        assert!(
            git.inspect(&repo, "feat/x").unwrap().branch_exists,
            "branches stay"
        );
        git.delete_branch(&repo, "feat/x").unwrap();
        assert!(git.inspect(&repo, "feat/x").unwrap().is_free());
    }

    #[test]
    fn reuses_branches_and_adopts_worktrees() {
        let (_dir, repo) = repo();
        let git = GitCli;
        run(&repo, &["branch", "fix/old"]);
        assert!(matches!(
            git.create(&repo, "fix/old", "main", CreateMode::New),
            Err(GitError::Command { .. })
        ));
        assert!(
            !worktree_path(&repo, "fix/old").exists(),
            "partial worktree removed"
        );
        assert!(
            git.inspect(&repo, "fix/old").unwrap().branch_exists,
            "their branch kept"
        );

        let path = git
            .create(&repo, "fix/old", "main", CreateMode::ReuseBranch)
            .unwrap();
        assert_eq!(
            git.create(&repo, "fix/old", "main", CreateMode::Adopt)
                .unwrap(),
            path
        );
        assert!(matches!(
            git.create(&repo, "fix/none", "main", CreateMode::Adopt),
            Err(GitError::NotAWorktree { .. })
        ));
        // A plain directory where the worktree would go can't be adopted.
        std::fs::create_dir_all(worktree_path(&repo, "fix/fake")).unwrap();
        run(&repo, &["branch", "fix/fake"]);
        assert!(matches!(
            git.create(&repo, "fix/fake", "main", CreateMode::Adopt),
            Err(GitError::NotAWorktree { .. })
        ));
    }

    #[test]
    fn refuses_branches_checked_out_elsewhere_and_unborn_bases() {
        let (_dir, repo) = repo();
        let git = GitCli;
        assert!(matches!(
            git.create(&repo, "main", "main", CreateMode::ReuseBranch),
            Err(GitError::CheckedOut { .. })
        ));
        assert!(matches!(
            git.create(&repo, "feat/z", "nope", CreateMode::New),
            Err(GitError::NoBase(_))
        ));

        let empty = repo.parent().unwrap().join("empty");
        std::fs::create_dir(&empty).unwrap();
        run(&empty, &["init", "--quiet", "--initial-branch=main"]);
        assert!(matches!(
            git.create(&empty, "feat/z", "main", CreateMode::New),
            Err(GitError::NoBase(_))
        ));
        assert!(!worktree_path(&empty, "feat/z").exists());
    }

    #[test]
    fn repo_hooks_never_run() {
        let (_dir, repo) = repo();
        let marker = repo.parent().unwrap().join("hook-ran");
        let hook = repo.join(".git/hooks/post-checkout");
        std::fs::write(&hook, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        GitCli
            .create(&repo, "feat/hooks", "main", CreateMode::New)
            .unwrap();
        assert!(!marker.exists(), "post-checkout ran");
    }

    #[test]
    fn branch_names_are_validated() {
        let (_dir, repo) = repo();
        for ok in ["feat/x", "jeff/fix/a-b", "main"] {
            GitCli.validate_branch(&repo, ok).unwrap();
        }
        for bad in [
            "has space",
            "double..dot",
            "-leading",
            "a/.hidden",
            "x.lock",
            "",
        ] {
            assert!(GitCli.validate_branch(&repo, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn repo_paths_must_resolve_inside_the_project() {
        let (dir, repo) = repo();
        let root = dir.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::os::unix::fs::symlink(&repo, root.join("link")).unwrap();
        assert!(matches!(
            GitCli.resolve_repo(&root, "link"),
            Err(GitError::Escapes { .. })
        ));
        assert!(matches!(
            GitCli.resolve_repo(&root, "missing"),
            Err(GitError::Missing { .. })
        ));
        assert!(matches!(
            GitCli.resolve_repo(&root, "."),
            Err(GitError::NotARepo(_))
        ));
        assert_eq!(GitCli.resolve_repo(dir.path(), "repo").unwrap(), repo);
        std::fs::create_dir(repo.join("sub")).unwrap();
        assert!(matches!(
            GitCli.resolve_repo(&repo, "sub"),
            Err(GitError::NotARepo(_))
        ));
    }
}
