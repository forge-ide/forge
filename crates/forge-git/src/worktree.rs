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

/// Whether a file name, as a tree or index holds it, may resolve to [`WORKTREES_DIR`] on some
/// filesystem: compared ignoring case (as macOS and Linux's casefold dirs do, Kelvin sign and
/// long s included) and the code points HFS+ ignores, as git's own `.git` check does.
pub fn is_worktrees_dir(name: &[u8]) -> bool {
    let name = String::from_utf8_lossy(name);
    let folded: String = name
        .chars()
        .filter(|c| !hfs_ignorable(*c))
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect();
    folded == WORKTREES_DIR
}

/// Whether any component of `path` (git's bytes, `/`-separated) is [`is_worktrees_dir`]:
/// every session worktree lives under one, in this repo or in a repo nested inside it.
pub fn in_worktrees_dir(path: &[u8]) -> bool {
    path.split(|b| *b == b'/').any(is_worktrees_dir)
}

/// Code points HFS+ drops when comparing names (git's `next_hfs_char`).
fn hfs_ignorable(c: char) -> bool {
    matches!(
        c,
        '\u{200C}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{206A}'..='\u{206F}' | '\u{FEFF}'
    )
}

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
    #[error("base branch {0:?} is not a valid branch name")]
    InvalidBase(String),
    #[error("base branch {0:?} has no commits to start a worktree from")]
    NoBase(String),
    #[error("branch {branch:?} is checked out at {path}")]
    CheckedOut { branch: String, path: PathBuf },
    #[error("{path} is not a worktree of branch {branch:?}")]
    NotAWorktree { path: PathBuf, branch: String },
    #[error("{path} is not a link to one of the repo's worktrees: {reason}")]
    BadWorktreeLink { path: PathBuf, reason: String },
    #[error("Forge doesn't run git on a worktree's files unconfined: {0}")]
    NotConfined(String),
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

/// The git dirs a worktree uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitDirs {
    /// The repo's shared git dir, `<repo>/.git`.
    pub common: PathBuf,
    /// The worktree's own git dir, `<common>/worktrees/<name>`.
    pub admin: PathBuf,
    /// How git commands that open the worktree's files run. Unset, they don't.
    pub launch: crate::confine::Launch,
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
    /// The git dirs of `worktree`, a worktree of `repo`. The worktree's `.git` file is
    /// writable by whatever runs in it, so it is believed only when it names a dir under the
    /// repo's own `worktrees/` whose `gitdir` file points back at it.
    fn git_dirs(&self, repo: &Path, worktree: &Path) -> Result<GitDirs, GitError>;
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
        if is_branch_name(repo, branch) {
            Ok(())
        } else {
            Err(GitError::InvalidBranch(branch.to_owned()))
        }
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
        if mode == CreateMode::New {
            // The base comes from forge.toml, which a cloned repo controls.
            if !is_branch_name(repo, base) {
                return Err(GitError::InvalidBase(base.to_owned()));
            }
            let commit = format!("{base}^{{commit}}");
            let args = [
                "rev-parse",
                "--verify",
                "--quiet",
                "--end-of-options",
                &commit,
            ];
            if git(repo, &args).is_err() {
                return Err(GitError::NoBase(base.to_owned()));
            }
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
            CreateMode::New => git(
                repo,
                &[
                    "worktree",
                    "add",
                    "-b",
                    branch,
                    "--end-of-options",
                    &target,
                    base,
                ],
            ),
            _ => git(
                repo,
                &["worktree", "add", "--end-of-options", &target, branch],
            ),
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

    fn git_dirs(&self, repo: &Path, worktree: &Path) -> Result<GitDirs, GitError> {
        let common = git(repo, &["rev-parse", "--git-common-dir"])?;
        let common = canonical(&repo.join(common.trim()))?;
        let link = worktree.join(".git");
        let bad = |reason: &str| GitError::BadWorktreeLink {
            path: link.clone(),
            reason: reason.to_owned(),
        };
        let meta = link.symlink_metadata().map_err(|_| bad("it is missing"))?;
        if !meta.is_file() {
            return Err(bad("it is not a regular file"));
        }
        let text = std::fs::read_to_string(&link).map_err(|_| bad("it can't be read"))?;
        let target = text
            .strip_prefix("gitdir:")
            .map(str::trim)
            .ok_or_else(|| bad("it has no gitdir line"))?;
        let admin = std::fs::canonicalize(worktree.join(target))
            .map_err(|_| bad("its gitdir does not exist"))?;
        if admin.parent() != Some(common.join("worktrees").as_path()) {
            return Err(bad("its gitdir is not under the repo's worktrees dir"));
        }
        let back = std::fs::read_to_string(admin.join("gitdir"))
            .map_err(|_| bad("its gitdir has no back link"))?;
        let back = std::fs::canonicalize(admin.join(back.trim()))
            .map_err(|_| bad("its gitdir's back link does not exist"))?;
        if back != canonical(&link)? {
            return Err(bad("its gitdir belongs to another worktree"));
        }
        Ok(GitDirs {
            common,
            admin,
            launch: Default::default(),
        })
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

/// A name git accepts for a branch that can't be mistaken for an option or a hidden path.
fn is_branch_name(repo: &Path, name: &str) -> bool {
    !name.starts_with('-')
        && !name.split('/').any(|c| c.starts_with('.'))
        && git(repo, &["check-ref-format", &format!("refs/heads/{name}")]).is_ok()
}

fn branch_exists(repo: &Path, branch: &str) -> Result<bool, GitError> {
    let mut cmd = command(repo)?;
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

/// Git on `repo`, through [`crate::confine::plain`]: git and the filters a checkout runs
/// (in the new worktree) are found through absolute `PATH` entries only. Unconfined: it runs
/// on the branch the human chose, before any agent has the worktree, as the user's own git
/// would, Git LFS content included.
fn command(repo: &Path) -> Result<Command, GitError> {
    let mut cmd = crate::confine::plain()?;
    cmd.arg("-C")
        .arg(repo)
        // Never run the repo's hooks; /dev/null is not a directory, so no hook is found.
        .args(["-c", "core.hooksPath=/dev/null"])
        // Inherited repository overrides would point git at some other repo.
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .stdin(std::process::Stdio::null());
    Ok(cmd)
}

/// Run git in `repo`; its stdout on success.
fn git(repo: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = command(repo)?
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

    #[test]
    fn the_worktrees_dir_is_recognized_in_any_spelling_a_filesystem_folds() {
        for name in [
            ".worktrees",
            ".WORKTREES",
            ".Worktrees",
            ".wor\u{212A}trees",
            ".work\u{200D}trees",
            "\u{FEFF}.worktrees",
            ".worktree\u{17F}",
        ] {
            assert!(is_worktrees_dir(name.as_bytes()), "{name:?}");
        }
        for name in [".worktrees2", "worktrees", ".worktree", ".worktrees/x", ""] {
            assert!(!is_worktrees_dir(name.as_bytes()), "{name:?}");
        }
        for path in [
            ".worktrees/x",
            "a/.WORKTREES/b",
            "./.worktrees",
            "front/end/.worktrees",
        ] {
            assert!(in_worktrees_dir(path.as_bytes()), "{path:?}");
        }
        for path in ["a/worktrees/b", "a.worktrees", ".worktrees2/x", ""] {
            assert!(!in_worktrees_dir(path.as_bytes()), "{path:?}");
        }
    }

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
    fn base_branches_cannot_smuggle_options() {
        let (_dir, repo) = repo();
        for base in ["--orphan", "-f", "main --force", ""] {
            let result = GitCli.create(&repo, "feat/opt", base, CreateMode::New);
            assert!(
                matches!(result, Err(GitError::InvalidBase(_))),
                "{base:?}: {result:?}"
            );
            assert!(!worktree_path(&repo, "feat/opt").exists());
            assert!(!GitCli.inspect(&repo, "feat/opt").unwrap().branch_exists);
        }
        GitCli
            .create(&repo, "feat/opt", "main", CreateMode::New)
            .unwrap();
    }

    #[test]
    fn git_dirs_follow_the_worktree_link_only_when_it_points_back() {
        let (_dir, repo) = repo();
        let git = GitCli;
        let mine = git
            .create(&repo, "feat/mine", "main", CreateMode::New)
            .unwrap();
        let other = git
            .create(&repo, "feat/other", "main", CreateMode::New)
            .unwrap();
        let dirs = git.git_dirs(&repo, &mine).unwrap();
        assert_eq!(dirs.common, repo.join(".git"));
        assert_eq!(dirs.admin.parent().unwrap(), repo.join(".git/worktrees"));
        let other_admin = git.git_dirs(&repo, &other).unwrap().admin;
        assert_ne!(dirs.admin, other_admin);

        // The worktree's .git file is the agent's to rewrite. Pointing it at another
        // session's git dir, or anywhere else, must not be believed.
        let link = mine.join(".git");
        std::fs::write(&link, format!("gitdir: {}\n", other_admin.display())).unwrap();
        assert!(matches!(
            git.git_dirs(&repo, &mine),
            Err(GitError::BadWorktreeLink { .. })
        ));
        let fake = mine.join("fake");
        std::fs::create_dir(&fake).unwrap();
        std::fs::write(fake.join("gitdir"), format!("{}\n", link.display())).unwrap();
        std::fs::write(&link, format!("gitdir: {}\n", fake.display())).unwrap();
        assert!(matches!(
            git.git_dirs(&repo, &mine),
            Err(GitError::BadWorktreeLink { .. })
        ));
        std::fs::remove_file(&link).unwrap();
        std::fs::create_dir(&link).unwrap();
        assert!(matches!(
            git.git_dirs(&repo, &mine),
            Err(GitError::BadWorktreeLink { .. })
        ));
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
