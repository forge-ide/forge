//! Commits the daemon makes on a session's branch for its agent (`git_commit`).
//!
//! Two steps, so whoever commits can check exactly what will be committed in between:
//! [`stage`] writes the worktree's changes into a tree, using an index the daemon owns, and
//! [`commit`] records that tree on the branch. Git runs with the repo's shared git dir and
//! explicit refs, never the worktree's `.git` link, git dir, or index, which whatever runs in
//! the worktree can rewrite; and the repo's hooks never run.

use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::worktree::{GitDirs, GitError};

/// Most diff text kept for the secret scan; a larger change is reported as cut.
const MAX_DIFF: usize = 4 * 1024 * 1024;

/// What a commit would record.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Staged {
    /// The branch tip it would go on top of.
    pub parent: String,
    pub tree: String,
    /// Paths it changes, relative to the repo, with forward slashes.
    pub changed: Vec<String>,
    /// The lines it adds, for the secret scan.
    pub added: String,
    /// Whether `added` was cut short.
    pub cut: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum CommitError {
    #[error("there is nothing to commit")]
    NothingToCommit,
    #[error("path {0:?} must be relative to the working directory and stay inside it")]
    BadPath(String),
    #[error("the branch {0} moved while committing; try again")]
    Moved(String),
    #[error(transparent)]
    Git(#[from] GitError),
}

/// Stage the changes in `worktree` (every change when `paths` is `None`, new files included)
/// on top of `branch`'s tip, in an index file at `index` that only the daemon writes.
pub fn stage(
    dirs: &GitDirs,
    worktree: &Path,
    branch: &str,
    paths: Option<&[String]>,
    index: &Path,
) -> Result<Staged, CommitError> {
    let pathspecs = match paths {
        None => vec![".".to_owned()],
        Some([]) => return Err(CommitError::NothingToCommit),
        Some(paths) => paths
            .iter()
            .map(|p| checked_path(p))
            .collect::<Result<_, _>>()?,
    };
    let _ = std::fs::remove_file(index);
    let run = |args: &[&str]| run(dirs, worktree, index, args, None);
    let parent = run(&[
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("refs/heads/{branch}^{{commit}}"),
    ])?
    .trim()
    .to_owned();
    run(&["read-tree", &parent])?;
    let mut add = vec!["add", "--all", "--"];
    add.extend(pathspecs.iter().map(String::as_str));
    run(&add)?;
    let tree = run(&["write-tree"])?.trim().to_owned();
    let changed: Vec<String> = run(&[
        "diff-tree",
        "-r",
        "-z",
        "--no-renames",
        "--name-only",
        &parent,
        &tree,
    ])?
    .split('\0')
    .filter(|p| !p.is_empty())
    .map(str::to_owned)
    .collect();
    if changed.is_empty() {
        return Err(CommitError::NothingToCommit);
    }
    let diff = run(&[
        "diff-tree",
        "-p",
        "-U0",
        "--no-renames",
        "--no-ext-diff",
        "--no-textconv",
        "--binary",
        &parent,
        &tree,
    ])?;
    let mut added = String::new();
    let mut cut = false;
    for line in diff.lines() {
        if let Some(text) = line.strip_prefix('+')
            && !line.starts_with("+++ ")
        {
            if added.len() + text.len() + 1 > MAX_DIFF {
                cut = true;
                break;
            }
            added.push_str(text);
            added.push('\n');
        }
    }
    Ok(Staged {
        parent,
        tree,
        changed,
        added,
        cut,
    })
}

/// Record `staged` on `branch` with `message`, if the branch is still where it was staged on.
/// Then the worktree's own index is reset to the new commit, so the agent's `git status`
/// shows nothing left to commit. Returns the new commit.
pub fn commit(
    dirs: &GitDirs,
    worktree: &Path,
    branch: &str,
    staged: &Staged,
    message: &str,
    index: &Path,
) -> Result<String, CommitError> {
    let commit = run(
        dirs,
        worktree,
        index,
        &["commit-tree", &staged.tree, "-p", &staged.parent, "-F", "-"],
        Some(message.as_bytes()),
    )?
    .trim()
    .to_owned();
    let reference = format!("refs/heads/{branch}");
    run(
        dirs,
        worktree,
        index,
        &[
            "update-ref",
            "-m",
            "forge: git_commit",
            &reference,
            &commit,
            &staged.parent,
        ],
        None,
    )
    .map_err(|_| CommitError::Moved(branch.to_owned()))?;
    let _ = std::fs::remove_file(index);
    // The worktree's index lives in its git dir, which the agent can write. Git replaces it
    // through a lock file it creates exclusively and renames over it, so a symlink planted
    // there is replaced, not followed; if the agent holds the lock, its index just stays stale.
    let own = dirs.admin.join("index");
    if run(dirs, worktree, &own, &["read-tree", &commit], None).is_ok() {
        let _ = run(
            dirs,
            worktree,
            &own,
            &["update-index", "-q", "--refresh"],
            None,
        );
    }
    Ok(commit)
}

/// `path` as a literal pathspec, if it is relative and stays inside the working directory.
fn checked_path(path: &str) -> Result<String, CommitError> {
    let p = Path::new(path);
    let ok = !path.is_empty()
        && p.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    if ok {
        Ok(path.to_owned())
    } else {
        Err(CommitError::BadPath(path.to_owned()))
    }
}

/// Run git on `worktree` with the repo's shared git dir and `index`; its stdout on success.
fn run(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<String, GitError> {
    let mut cmd = Command::new("git");
    cmd.args([
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
    ])
    .args(args)
    .current_dir(worktree)
    .env("GIT_DIR", &dirs.common)
    .env("GIT_WORK_TREE", worktree)
    .env("GIT_INDEX_FILE", index)
    .env("GIT_LITERAL_PATHSPECS", "1")
    .env("GIT_TERMINAL_PROMPT", "0")
    .env_remove("GIT_COMMON_DIR")
    .env_remove("GIT_OBJECT_DIRECTORY")
    .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
    .env_remove("GIT_NAMESPACE")
    .env_remove("GIT_CONFIG_PARAMETERS")
    .env_remove("GIT_CONFIG_COUNT")
    .stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|source| GitError::Io {
        context: "running git".into(),
        source,
    })?;
    if let (Some(input), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(input).map_err(|source| GitError::Io {
            context: "writing to git".into(),
            source,
        })?;
    }
    let output = child.wait_with_output().map_err(|source| GitError::Io {
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

/// Where a session keeps the index its commits are staged in: a daemon-owned dir no sandbox
/// can write.
pub fn index_path(private_dir: &Path) -> PathBuf {
    private_dir.join("git_commit.index")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{CreateMode, GitCli, Worktrees};

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    /// A repo on `main` with a session worktree on `feat/x`, and a private dir for the index.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf, GitDirs, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        git(&repo, &["config", "user.name", "T"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "a.txt"]);
        git(&repo, &["commit", "--quiet", "-m", "initial"]);
        let wt = GitCli
            .create(&repo, "feat/x", "main", CreateMode::New)
            .unwrap();
        let dirs = GitCli.git_dirs(&repo, &wt).unwrap();
        let private = root.join("private");
        std::fs::create_dir(&private).unwrap();
        (dir, repo, wt, dirs, private)
    }

    #[test]
    fn commits_every_change_on_the_branch_and_leaves_the_worktree_clean() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        std::fs::write(wt.join("new.rs"), "fn f() {}\n").unwrap();
        // The agent's own staging is ignored: the daemon stages from the files.
        git(&wt, &["add", "a.txt"]);
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        assert_eq!(staged.changed, ["a.txt", "new.rs"]);
        assert!(staged.added.contains("fn f() {}"), "{}", staged.added);
        assert!(!staged.cut);
        let commit = commit(&dirs, &wt, "feat/x", &staged, "add f\n\nbody", &index).unwrap();
        assert_eq!(git(&repo, &["rev-parse", "feat/x"]).trim(), commit);
        assert_eq!(
            git(&repo, &["log", "-1", "--format=%s", "feat/x"]).trim(),
            "add f"
        );
        // main didn't move, and the worktree has nothing left to commit.
        assert_ne!(git(&repo, &["rev-parse", "main"]).trim(), commit);
        assert_eq!(git(&wt, &["status", "--porcelain"]), "");
        assert!(!index.exists());
        // Committing again has nothing to do.
        assert!(matches!(
            stage(&dirs, &wt, "feat/x", None, &index),
            Err(CommitError::NothingToCommit)
        ));
    }

    #[test]
    fn commits_only_named_paths_and_refuses_paths_that_leave_the_worktree() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        std::fs::write(wt.join("b.txt"), "b\n").unwrap();
        let paths = vec!["b.txt".to_owned()];
        let staged = stage(&dirs, &wt, "feat/x", Some(&paths), &index).unwrap();
        assert_eq!(staged.changed, ["b.txt"]);
        commit(&dirs, &wt, "feat/x", &staged, "b", &index).unwrap();
        assert_eq!(
            git(&repo, &["show", "--name-only", "--format=", "feat/x"]).trim(),
            "b.txt"
        );
        assert_eq!(git(&wt, &["status", "--porcelain"]).trim(), "M a.txt");
        for bad in ["../x", "/etc/passwd", "a/../../x", ""] {
            assert!(
                matches!(
                    stage(&dirs, &wt, "feat/x", Some(&[bad.to_owned()]), &index),
                    Err(CommitError::BadPath(_))
                ),
                "{bad:?}"
            );
        }
        // A pathspec is literal: a glob names a file called `*`, not everything.
        assert!(stage(&dirs, &wt, "feat/x", Some(&["*".to_owned()]), &index).is_err());
    }

    #[test]
    fn a_branch_that_moved_after_staging_is_not_overwritten() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        let main = git(&repo, &["rev-parse", "main"]);
        let other = git(
            &repo,
            &[
                "commit-tree",
                "-m",
                "other",
                "-p",
                main.trim(),
                &format!("{}^{{tree}}", main.trim()),
            ],
        );
        git(&repo, &["update-ref", "refs/heads/feat/x", other.trim()]);
        assert!(matches!(
            commit(&dirs, &wt, "feat/x", &staged, "m", &index),
            Err(CommitError::Moved(_))
        ));
        assert_eq!(git(&repo, &["rev-parse", "feat/x"]).trim(), other.trim());
    }

    #[test]
    fn the_worktrees_git_link_and_hooks_are_never_used() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        // A hook that would leave a mark if anything ran it.
        let hooks = repo.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let mark = private.join("hook-ran");
        for hook in ["pre-commit", "post-commit", "reference-transaction"] {
            let path = hooks.join(hook);
            std::fs::write(&path, format!("#!/bin/sh\ntouch {}\n", mark.display())).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // The agent points its .git link at another repo.
        let other = private.join("other");
        std::fs::create_dir(&other).unwrap();
        git(&other, &["init", "--quiet"]);
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}/.git\n", other.display()),
        )
        .unwrap();
        std::fs::write(wt.join("c.txt"), "c\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        let commit = commit(&dirs, &wt, "feat/x", &staged, "c", &index).unwrap();
        assert_eq!(git(&repo, &["rev-parse", "feat/x"]).trim(), commit);
        assert!(!mark.exists(), "a repo hook ran");
    }
}
