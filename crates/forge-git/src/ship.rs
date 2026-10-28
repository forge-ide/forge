//! What the human does to a session's branch after review: reverting a hunk, and shipping it,
//! by merging it into the base branch locally or pushing it.
//!
//! Each starts from the checkpoint the human reviewed and refuses if the worktree or branch
//! moved on since ([`checkpoint::unchanged`]). What the agent left uncommitted at that
//! checkpoint goes on the branch first, as a commit of its own, so what is reverted on or
//! shipped is exactly what was reviewed. Like [`crate::commit`], git runs with the repo's
//! shared git dir and explicit refs, the repo's hooks never run, and refs move by compare and
//! swap.

use std::path::{Path, PathBuf};

use crate::checkpoint::{self, load, tree_of};
use crate::commit::{self, CommitError};
use crate::worktree::{GitDirs, GitError, WORKTREES_DIR, in_worktrees_dir};

#[derive(Debug, thiserror::Error)]
pub enum ShipError {
    #[error(
        "merging into {base} conflicts in {}{}",
        .files.join(", "),
        if *.more > 0 { format!(" and {} more", .more) } else { String::new() }
    )]
    Conflict {
        base: String,
        files: Vec<String>,
        /// Conflicted files past the ones named.
        more: usize,
    },
    #[error(
        "{0} holds a {WORKTREES_DIR}/ dir, where session worktrees live in the main checkout (or a repo nested in it); Forge doesn't merge it"
    )]
    Reserved(String),
    #[error(
        "{base} is checked out at {}, which isn't the repo's main checkout; ship from there by hand",
        commit::printable(.path.as_os_str().as_encoded_bytes())
    )]
    CheckedOutElsewhere { base: String, path: PathBuf },
    #[error(
        "the checkout of {base} at {} has changes of its own, or a merge or rebase under way; finish or stash them, then ship again",
        .path.display()
    )]
    Dirty { base: String, path: PathBuf },
    #[error(
        "the checkout of {base} at {} couldn't be updated and may be partly updated; the branch is back where it was, and `git reset --hard {from}` there restores the files",
        .path.display()
    )]
    PartlyUpdated {
        base: String,
        path: PathBuf,
        from: String,
    },
    #[error("the remote refused the push: {0}")]
    Rejected(String),
    #[error("{0}")]
    NoRemote(String),
    #[error("{0}")]
    Layout(String),
    #[error(transparent)]
    Commit(#[from] CommitError),
}

impl From<GitError> for ShipError {
    fn from(e: GitError) -> Self {
        ShipError::Commit(CommitError::Git(e))
    }
}

/// The commit holding checkpoint `checkpoint`'s tree on `tip`, the branch tip
/// [`checkpoint::unchanged`] returned: `tip` when it already holds it, else a new commit with
/// `message` (not yet on the branch).
fn settle(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    checkpoint: &str,
    tip: &str,
    message: &str,
) -> Result<String, CommitError> {
    let tree = tree_of(dirs, worktree, index, checkpoint)?;
    if tree_of(dirs, worktree, index, tip)? == tree {
        return Ok(tip.to_owned());
    }
    commit_tree(dirs, worktree, index, &tree, &[tip], message)
}

fn commit_tree(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    tree: &str,
    parents: &[&str],
    message: &str,
) -> Result<String, CommitError> {
    let mut args = vec!["commit-tree", tree];
    for parent in parents {
        args.extend(["-p", parent]);
    }
    args.extend(["-F", "-"]);
    Ok(
        commit::run(dirs, worktree, index, &args, Some(message.as_bytes()))?
            .trim()
            .to_owned(),
    )
}

/// Put `branch` at checkpoint `checkpoint`, which must still match the worktree and branch:
/// what the agent left uncommitted becomes a commit with `message`. Returns the branch tip.
pub fn commit_reviewed(
    dirs: &GitDirs,
    worktree: &Path,
    branch: &str,
    checkpoint: &str,
    index: &Path,
    message: &str,
) -> Result<String, CommitError> {
    let tip = checkpoint::unchanged(dirs, worktree, branch, checkpoint, index)?;
    let commit = settle(dirs, worktree, index, checkpoint, &tip, message)?;
    if commit != tip {
        let reference = format!("refs/heads/{branch}");
        commit::point(
            dirs,
            worktree,
            index,
            &reference,
            &commit,
            &tip,
            "forge: ship",
        )
        .map_err(|_| CommitError::Moved(branch.to_owned()))?;
        commit::sync_own_index(dirs, worktree, &commit);
    }
    Ok(commit)
}

/// What a file becomes.
pub struct FileEdit<'a> {
    /// The path, as git's bytes.
    pub path: &'a [u8],
    /// Its mode (octal, as git prints it) and contents; `None` to delete it.
    pub content: Option<(&'a str, &'a [u8])>,
    /// The commit message.
    pub message: &'a str,
}

/// Make `edit` on `branch` as a commit, on top of checkpoint `checkpoint`
/// (what the agent left uncommitted goes first, as a commit with `settle_message`), and
/// update the worktree to it. Refuses, changing nothing, if the worktree or branch no longer
/// match the checkpoint. Returns the new commit.
pub fn edit_file(
    dirs: &GitDirs,
    worktree: &Path,
    branch: &str,
    checkpoint: &str,
    index: &Path,
    edit: &FileEdit<'_>,
    settle_message: &str,
) -> Result<String, CommitError> {
    let run = |args: &[&str], stdin: Option<&[u8]>| commit::run(dirs, worktree, index, args, stdin);
    if in_worktrees_dir(edit.path) {
        return Err(CommitError::Reserved(
            String::from_utf8_lossy(edit.path).into_owned(),
        ));
    }
    let tip = checkpoint::unchanged(dirs, worktree, branch, checkpoint, index)?;
    let base = settle(dirs, worktree, index, checkpoint, &tip, settle_message)?;
    // The index holds the checkpoint's tree, which is `base`'s.
    let mut entry = Vec::new();
    match edit.content {
        Some((mode, bytes)) => {
            let blob = run(
                &["hash-object", "-w", "--stdin", "--no-filters"],
                Some(bytes),
            )?;
            entry.extend_from_slice(format!("{mode} {}\t", blob.trim()).as_bytes());
            entry.extend_from_slice(edit.path);
            entry.push(0);
            run(&["update-index", "-z", "--index-info"], Some(&entry))?;
        }
        None => {
            entry.extend_from_slice(edit.path);
            entry.push(0);
            run(
                &["update-index", "-z", "--force-remove", "--stdin"],
                Some(&entry),
            )?;
        }
    }
    let tree = run(&["write-tree"], None)?.trim().to_owned();
    let commit = commit_tree(dirs, worktree, index, &tree, &[&base], edit.message)?;
    // Back to the checkpoint's tree, to update the worktree from it; a dry run first, so a
    // file that changed meanwhile refuses before anything moves.
    load(dirs, worktree, index, checkpoint)?;
    let _ = run(&["update-index", "-q", "--refresh"], None);
    run(&["read-tree", "-m", "-u", "-n", checkpoint, &commit], None)
        .map_err(|_| CommitError::WorktreeChanged)?;
    let reference = format!("refs/heads/{branch}");
    commit::point(
        dirs,
        worktree,
        index,
        &reference,
        &commit,
        &tip,
        "forge: edit",
    )
    .map_err(|_| CommitError::Moved(branch.to_owned()))?;
    if let Err(e) = run(&["read-tree", "-m", "-u", checkpoint, &commit], None)
        && let Err(reset) = run(&["read-tree", "--reset", "-u", &commit], None)
    {
        tracing::warn!(error = %e, "updating the worktree stopped partway");
        let _ = commit::point(
            dirs,
            worktree,
            index,
            &reference,
            &tip,
            &commit,
            "forge: edit undone",
        );
        return Err(CommitError::PartlyRewound {
            from: checkpoint.to_owned(),
            reason: reset.to_string(),
        });
    }
    commit::sync_own_index(dirs, worktree, &commit);
    Ok(commit)
}

/// A merge of a session's branch into its base branch, worked out but not yet made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Merge {
    pub base: String,
    /// The base branch's tip now.
    pub old: String,
    /// What it becomes: the session's tip for a fast-forward, else a merge commit.
    pub new: String,
    /// The repo's main checkout, when it has the base branch checked out.
    pub checkout: Option<PathBuf>,
}

/// Work out the merge of `tip` into `base` without touching any ref or checkout: a
/// fast-forward when it can be one, else a merge commit with `message`. Merge settings
/// (attributes such as `merge=union`) come from `base`'s tip, never from the session's files.
/// Refuses on conflicts, naming the files; when `base` is checked out anywhere but the main
/// checkout; and when the main checkout has changes of its own.
pub fn plan_merge(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    into: &MergeInto<'_>,
    tip: &str,
    message: &str,
) -> Result<Merge, ShipError> {
    let base = into.base;
    let run = |args: &[&str]| commit::run(dirs, worktree, index, args, None);
    let reserved = |commit: &str| -> Result<bool, ShipError> {
        let listed = commit::run_bytes(
            dirs,
            worktree,
            index,
            &["ls-tree", "-r", "-z", "--name-only", commit],
            None,
        )?;
        Ok(listed.split(|b| *b == 0).any(in_worktrees_dir))
    };
    let old = run(&[
        "rev-parse",
        "--verify",
        "--end-of-options",
        &format!("refs/heads/{base}^{{commit}}"),
    ])?
    .trim()
    .to_owned();
    for (side, commit) in [
        (base.to_owned(), old.as_str()),
        ("the session's branch".to_owned(), tip),
    ] {
        if reserved(commit)? {
            return Err(ShipError::Reserved(side));
        }
    }
    let checkout = checkout_of(dirs, worktree, index, into)?;
    if let Some(path) = &checkout {
        clean(dirs, path, base)?;
    }
    let ancestor = |a: &str, b: &str| -> Result<bool, ShipError> {
        let (status, _, stderr) = commit::run_status(
            dirs,
            worktree,
            index,
            &["merge-base", "--is-ancestor", a, b],
            &[],
        )?;
        match status.code() {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(GitError::Command {
                args: "merge-base --is-ancestor".into(),
                stderr: stderr.trim().to_owned(),
            }
            .into()),
        }
    };
    let new = if ancestor(&old, tip)? {
        tip.to_owned()
    } else if ancestor(tip, &old)? {
        old.clone()
    } else {
        let (status, out, stderr) = commit::run_status(
            dirs,
            worktree,
            index,
            &[
                "merge-tree",
                "--write-tree",
                "-z",
                "--name-only",
                "--no-messages",
                &old,
                tip,
            ],
            &[("GIT_ATTR_SOURCE", &old)],
        )?;
        let out = String::from_utf8_lossy(&out);
        let mut fields = out.split('\0');
        let tree = fields.next().unwrap_or_default().trim().to_owned();
        match status.code() {
            Some(0) => commit_tree(dirs, worktree, index, &tree, &[&old, tip], message)?,
            Some(1) => {
                let mut files: Vec<String> = fields
                    .take_while(|f| !f.is_empty())
                    .map(|f| crate::review::quote_path(f.as_bytes()))
                    .collect();
                files.dedup();
                let more = files.len().saturating_sub(MAX_CONFLICTS);
                files.truncate(MAX_CONFLICTS);
                return Err(ShipError::Conflict {
                    base: base.to_owned(),
                    files,
                    more,
                });
            }
            _ => {
                return Err(GitError::Command {
                    args: "merge-tree --write-tree".into(),
                    stderr: stderr.trim().to_owned(),
                }
                .into());
            }
        }
    };
    if reserved(&new)? {
        return Err(ShipError::Reserved("the merge".to_owned()));
    }
    Ok(Merge {
        base: base.to_owned(),
        old,
        new,
        checkout,
    })
}

/// Where a merge goes.
pub struct MergeInto<'a> {
    /// The base branch.
    pub base: &'a str,
    /// The repo's main checkout, as the project names it.
    pub repo: &'a Path,
    /// Every session worktree of the repo Forge made, from its own records: their admin dirs
    /// are their agents' to write, so what git lists for them is never believed.
    pub sessions: &'a [PathBuf],
}

/// Move the base branch to `m.new` (compare and swap) and update its checkout to match.
pub fn apply_merge(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    m: &Merge,
) -> Result<(), ShipError> {
    move_base(dirs, worktree, index, m, &m.old, &m.new)
}

/// Undo [`apply_merge`]: the base branch, and its checkout, back to `m.old`.
pub fn undo_merge(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    m: &Merge,
) -> Result<(), ShipError> {
    move_base(dirs, worktree, index, m, &m.new, &m.old)
}

fn move_base(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    m: &Merge,
    from: &str,
    to: &str,
) -> Result<(), ShipError> {
    if from == to {
        return Ok(());
    }
    let reference = format!("refs/heads/{}", m.base);
    commit::point(
        dirs,
        worktree,
        index,
        &reference,
        to,
        from,
        "forge: ship merge-local",
    )
    .map_err(|_| CommitError::Moved(m.base.clone()))?;
    let Some(path) = &m.checkout else {
        return Ok(());
    };
    let (main, main_index) = main_checkout(dirs);
    // Attributes (which filters run, line endings) from the base branch as it was before
    // the merge, both ways: never from what the merge brought in.
    let checkout = |args: &[&str]| {
        commit::run_env(
            &main,
            path,
            &main_index,
            args,
            None,
            &[("GIT_ATTR_SOURCE", m.old.as_str())],
        )
    };
    if let Err(e) = checkout(&["read-tree", "-m", "-u", from, to]) {
        tracing::warn!(error = %e, "updating the base branch's checkout");
        let _ = commit::point(
            dirs,
            worktree,
            index,
            &reference,
            from,
            to,
            "forge: ship undone",
        );
        // It was clean, so putting its tracked files back loses nothing of the user's.
        if let Err(e) = checkout(&["read-tree", "--reset", "-u", from]) {
            tracing::warn!(error = %e, "putting the base branch's checkout back");
            return Err(ShipError::PartlyUpdated {
                base: m.base.clone(),
                path: path.clone(),
                from: from.to_owned(),
            });
        }
        return Err(ShipError::Dirty {
            base: m.base.clone(),
            path: path.clone(),
        });
    }
    Ok(())
}

/// The most conflicted files a refusal names.
const MAX_CONFLICTS: usize = 1000;

/// The repo's main checkout, as git sees it: the shared git dir is its own, and so is the
/// index in it. Its git is confined as the session's worktree git is (`dirs.launch`), to the
/// repo: session worktrees live inside it, so a link an agent swaps in there can't lead a
/// write outside the repo.
fn main_checkout(dirs: &GitDirs) -> (GitDirs, PathBuf) {
    let main = GitDirs {
        common: dirs.common.clone(),
        admin: dirs.common.clone(),
        launch: dirs.launch.clone(),
    };
    (main, dirs.common.join("index"))
}

/// Where `into.base` is checked out: `None` when nowhere, the main checkout when there, and
/// an error when in a worktree of the user's own. The main checkout's HEAD is read from the
/// shared git dir. Linked worktrees' records come from their admin dirs, which a session's
/// agent can write, so the records of Forge's session worktrees are passed over: Forge never
/// checks a base branch out there.
fn checkout_of(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    into: &MergeInto<'_>,
) -> Result<Option<PathBuf>, ShipError> {
    let want = format!("refs/heads/{}", into.base);
    let head = commit::run(
        dirs,
        worktree,
        index,
        &["rev-parse", "--symbolic-full-name", "HEAD"],
        None,
    )
    .unwrap_or_default();
    if head.trim() == want {
        // The checkout to update is the one whose git dir is this repo's.
        let canonical = |p: &Path| std::fs::canonicalize(p).ok();
        if git_dir_of(into.repo).and_then(|d| canonical(&d)) != canonical(&dirs.common) {
            return Err(ShipError::Layout(format!(
                "{} doesn't keep its git dir at {}, so Forge can't tell its checkout of {} is the one to update; merge there by hand",
                into.repo.display(),
                dirs.common.display(),
                into.base
            )));
        }
        return Ok(Some(into.repo.to_path_buf()));
    }
    let listed = commit::run_bytes(
        dirs,
        worktree,
        index,
        &["worktree", "list", "--porcelain", "-z"],
        None,
    )?;
    let canonical = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let sessions: Vec<PathBuf> = into.sessions.iter().map(|p| canonical(p)).collect();
    let mut path: Option<PathBuf> = None;
    let mut first = true;
    for line in listed.split(|b| *b == 0) {
        if line.is_empty() {
            first &= path.is_none();
            path = None;
            continue;
        }
        if let Some(p) = line.strip_prefix(b"worktree ") {
            use std::os::unix::ffi::OsStrExt;
            path = Some(PathBuf::from(std::ffi::OsStr::from_bytes(p)));
        } else if line == format!("branch {want}").as_bytes()
            && let Some(at) = &path
            && !first
            && !sessions.contains(&canonical(at))
        {
            return Err(ShipError::CheckedOutElsewhere {
                base: into.base.to_owned(),
                path: at.clone(),
            });
        }
    }
    Ok(None)
}

/// The git dir `repo`'s checkout uses: its `.git` dir, or the one its `.git` file names.
fn git_dir_of(repo: &Path) -> Option<PathBuf> {
    let dot = repo.join(".git");
    if std::fs::symlink_metadata(&dot).ok()?.is_dir() {
        return Some(dot);
    }
    let text = std::fs::read_to_string(&dot).ok()?;
    Some(repo.join(text.strip_prefix("gitdir:")?.trim()))
}

/// Refuse unless the main checkout at `path` has no changes to tracked files and no merge,
/// rebase, cherry-pick, or revert under way.
fn clean(dirs: &GitDirs, path: &Path, base: &str) -> Result<(), ShipError> {
    let dirty = || ShipError::Dirty {
        base: base.to_owned(),
        path: path.to_path_buf(),
    };
    let busy = [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ];
    if busy.iter().any(|f| dirs.common.join(f).exists()) {
        return Err(dirty());
    }
    let (main, main_index) = main_checkout(dirs);
    let status = commit::run_env(
        &main,
        path,
        &main_index,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=no",
            "--ignore-submodules=all",
        ],
        None,
        &[("GIT_ATTR_SOURCE", "HEAD")],
    )?;
    if !status.is_empty() {
        return Err(dirty());
    }
    Ok(())
}

/// Push `commit` to branch `branch` of the remote `base` pulls from (else `origin`), never
/// forced: a branch there that isn't an ancestor is refused. Runs from the repo's shared git
/// dir with the user's own git config and credentials, which only the daemon has, and without
/// the repo's hooks. Returns the remote's name.
pub fn push(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    base: &str,
    commit: &str,
    branch: &str,
) -> Result<String, ShipError> {
    let get = |key: &str| -> Result<Option<String>, ShipError> {
        let (status, out, stderr) =
            commit::run_status(dirs, worktree, index, &["config", "--get", key], &[])
                .map_err(scrubbed)?;
        match status.code() {
            Some(0) => Ok(Some(String::from_utf8_lossy(&out).trim().to_owned())),
            Some(1) => Ok(None),
            _ => Err(GitError::Command {
                args: format!("config --get {key}"),
                stderr: scrub(stderr.trim()),
            }
            .into()),
        }
    };
    let remote = get(&format!("branch.{base}.remote"))?
        .filter(|r| r != "." && !r.is_empty())
        .unwrap_or_else(|| "origin".to_owned());
    // A URL in place of a name may hold a token; it never reaches git's arguments or an error.
    let named = !remote.contains("://")
        && !remote
            .split('/')
            .next()
            .is_some_and(|host| host.contains('@') || host.contains(':'));
    if !named {
        return Err(ShipError::NoRemote(format!(
            "push needs a named remote; set branch.{base}.remote to one"
        )));
    }
    if remote.starts_with('-')
        || (get(&format!("remote.{remote}.url"))?.is_none()
            && get(&format!("remote.{remote}.pushurl"))?.is_none())
    {
        return Err(ShipError::NoRemote(format!(
            "there's no remote {remote:?} to push to: {base} has no upstream, and the repo no remote by that name"
        )));
    }
    let refspec = format!("{commit}:refs/heads/{branch}");
    let (status, out, stderr) = commit::run_status(
        dirs,
        worktree,
        index,
        &[
            "push",
            "--porcelain",
            "--no-verify",
            "--recurse-submodules=no",
            &remote,
            &refspec,
        ],
        &[],
    )
    .map_err(scrubbed)?;
    if status.success() {
        return Ok(remote);
    }
    let out = String::from_utf8_lossy(&out);
    let rejected: Vec<&str> = out.lines().filter(|l| l.starts_with('!')).collect();
    if !rejected.is_empty() {
        return Err(ShipError::Rejected(scrub(&rejected.join("; "))));
    }
    Err(GitError::Command {
        args: format!("push {remote} {refspec}"),
        stderr: scrub(stderr.trim()),
    }
    .into())
}

/// `e` with any URL credentials in it scrubbed.
fn scrubbed(e: GitError) -> GitError {
    match e {
        GitError::Command { args, stderr } => GitError::Command {
            args: scrub(&args),
            stderr: scrub(&stderr),
        },
        e => e,
    }
}

/// `text` with the user and password of every URL in it replaced by `***`, so a token in a
/// remote's URL never reaches an error or the log.
pub fn scrub(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (head, tail) = rest.split_at(at + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(user) => {
                out.push_str("***");
                out.push_str(&authority[user..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::confine::Launch;
    use crate::worktree::{CreateMode, GitCli, Worktrees};
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "T")
            .env("GIT_AUTHOR_EMAIL", "t@example.com")
            .env("GIT_COMMITTER_NAME", "T")
            .env("GIT_COMMITTER_EMAIL", "t@example.com")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn commit_file(dir: &Path, path: &str, text: &str) {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
        git(dir, &["add", path]);
        git(dir, &["commit", "--quiet", "-m", path]);
    }

    struct Setup {
        dir: tempfile::TempDir,
        repo: PathBuf,
        wt: PathBuf,
        dirs: GitDirs,
        index: PathBuf,
    }

    impl Setup {
        /// A repo on `main` holding `a.txt`, with a session worktree on `feat/x`.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let repo = dir.path().canonicalize().unwrap().join("repo");
            std::fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "--quiet", "--initial-branch=main"]);
            commit_file(&repo, "a.txt", "base\n");
            let wt = GitCli
                .create(&repo, "feat/x", "main", CreateMode::New)
                .unwrap();
            let mut dirs = GitCli.git_dirs(&repo, &wt).unwrap();
            dirs.launch = Launch::Unconfined;
            let index = dir.path().join("private/i.index");
            Setup {
                dir,
                repo,
                wt,
                dirs,
                index,
            }
        }

        fn rev(&self, rev: &str) -> String {
            git(&self.repo, &["rev-parse", rev]).trim().to_owned()
        }

        fn plan(&self) -> Result<Merge, ShipError> {
            let sessions = [self.wt.clone()];
            let into = MergeInto {
                base: "main",
                repo: &self.repo,
                sessions: &sessions,
            };
            plan_merge(
                &self.dirs,
                &self.wt,
                &self.index,
                &into,
                &self.rev("feat/x"),
                "merge",
            )
        }
    }

    /// Merge settings are the base branch's: its `merge=union` settles a conflict that the
    /// session's own attributes file, asking for the same, doesn't.
    #[test]
    fn merges_follow_the_base_branchs_attributes() {
        let s = Setup::new();
        commit_file(&s.wt, "a.txt", "session\n");
        commit_file(&s.wt, ".gitattributes", "a.txt merge=union\n");
        commit_file(&s.repo, "a.txt", "human\n");
        match s.plan() {
            Err(ShipError::Conflict { files, more, .. }) => {
                assert_eq!((files.as_slice(), more), (&["a.txt".to_owned()][..], 0));
            }
            other => panic!("{other:?}"),
        }
        commit_file(&s.repo, ".gitattributes", "a.txt merge=union\n");
        let merge = s.plan().unwrap();
        let merged = git(&s.repo, &["show", &format!("{}:a.txt", merge.new)]);
        assert_eq!(merged, "human\nsession\n");
    }

    /// Session worktrees live under `.worktrees/` in the main checkout, so it never enters a
    /// checkpoint or a commit, however the agent's ignore rules read, and a tree that holds it
    /// is never merged.
    #[test]
    fn the_worktrees_dir_is_never_staged_or_merged() {
        let s = Setup::new();
        let write = |path: &str, text: &str| {
            let path = s.wt.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write(".gitignore", "!/.worktrees/\n");
        write(".worktrees/feat/x/d/payload", "x\n");
        write("b.txt", "b\n");
        let cp = checkpoint::take(
            &s.dirs,
            &s.wt,
            "feat/x",
            None,
            &s.index,
            &checkpoint::checkpoint_ref("ses_1", 1),
            "cp",
        )
        .unwrap();
        let files = git(&s.repo, &["ls-tree", "-r", "--name-only", &cp]);
        assert_eq!(files, ".gitignore\na.txt\nb.txt\n");
        let staged = commit::stage(
            &s.dirs,
            &s.wt,
            "feat/x",
            None,
            &s.dir.path().join("c.index"),
        )
        .unwrap();
        assert_eq!(staged.changed, [".gitignore", "b.txt"]);
        assert!(matches!(
            commit::stage(
                &s.dirs,
                &s.wt,
                "feat/x",
                Some(&["./.worktrees/feat/x".to_owned()]),
                &s.dir.path().join("c.index"),
            ),
            Err(CommitError::Reserved(_))
        ));
        // A base branch that already holds one isn't merged into, and nothing moves.
        let base = s.rev("main");
        std::fs::create_dir_all(s.repo.join(".worktrees/x")).unwrap();
        std::fs::write(s.repo.join(".worktrees/x/f"), "f\n").unwrap();
        git(&s.repo, &["add", "--force", ".worktrees/x/f"]);
        git(&s.repo, &["commit", "--quiet", "-m", "oops"]);
        commit_file(&s.wt, "b.txt", "b\n");
        assert!(matches!(s.plan(), Err(ShipError::Reserved(_))));
        assert_ne!(s.rev("main"), base);
        let edit = FileEdit {
            path: b".worktrees/x/f",
            content: None,
            message: "m",
        };
        assert!(matches!(
            edit_file(&s.dirs, &s.wt, "feat/x", &cp, &s.index, &edit, "m"),
            Err(CommitError::Reserved(_))
        ));
    }

    /// Names any filesystem could resolve to `.worktrees` count as it: on macOS, or in a
    /// casefold dir, `.WORKTREES/<branch>/` is the agent's own worktree.
    #[test]
    fn spellings_of_the_worktrees_dir_are_never_staged_or_merged() {
        // The last: a repo nested in this one keeps its session worktrees in its own.
        for name in [
            ".WORKTREES",
            ".wor\u{212A}trees",
            ".work\u{200D}trees",
            "frontend/.worktrees",
        ] {
            let s = Setup::new();
            std::fs::write(s.wt.join(".gitignore"), format!("!/{name}/\n")).unwrap();
            commit_file(&s.wt, "b.txt", "b\n");
            std::fs::create_dir_all(s.wt.join(name).join("feat/x")).unwrap();
            std::fs::write(s.wt.join(name).join("feat/x/f"), "x\n").unwrap();
            let cp = checkpoint::take(
                &s.dirs,
                &s.wt,
                "feat/x",
                None,
                &s.index,
                &checkpoint::checkpoint_ref("ses_1", 1),
                "cp",
            )
            .unwrap();
            let files = git(&s.repo, &["ls-tree", "-r", "--name-only", &cp]);
            assert!(!files.contains("trees"), "{name:?}: {files}");
            // Committed by other means, it still never merges.
            git(&s.wt, &["add", "--force", "--", name]);
            git(&s.wt, &["commit", "--quiet", "-m", "sneak"]);
            let base = s.rev("main");
            assert!(matches!(s.plan(), Err(ShipError::Reserved(_))), "{name:?}");
            assert_eq!(s.rev("main"), base);
        }
    }

    /// Files the branch already tracks under a `.worktrees/` dir stay as it has them: never
    /// changed by the agent, and never lost.
    #[test]
    fn files_the_branch_already_tracks_there_are_kept_as_they_were() {
        let s = Setup::new();
        std::fs::create_dir_all(s.repo.join("docs/.worktrees")).unwrap();
        std::fs::write(s.repo.join("docs/.worktrees/keep"), "kept\n").unwrap();
        git(&s.repo, &["add", "--force", "docs/.worktrees/keep"]);
        git(&s.repo, &["commit", "--quiet", "-m", "tracked"]);
        git(&s.wt, &["merge", "--quiet", "--ff-only", "main"]);
        std::fs::write(s.wt.join("docs/.worktrees/keep"), "changed\n").unwrap();
        std::fs::write(s.wt.join("docs/.worktrees/new"), "new\n").unwrap();
        std::fs::write(s.wt.join("b.txt"), "b\n").unwrap();
        let cp = checkpoint::take(
            &s.dirs,
            &s.wt,
            "feat/x",
            None,
            &s.index,
            &checkpoint::checkpoint_ref("ses_1", 1),
            "cp",
        )
        .unwrap();
        let files = git(&s.repo, &["ls-tree", "-r", "--name-only", &cp]);
        assert_eq!(files, "a.txt\nb.txt\ndocs/.worktrees/keep\n");
        assert_eq!(
            git(&s.repo, &["show", &format!("{cp}:docs/.worktrees/keep")]),
            "kept\n"
        );
        let staged = commit::stage(
            &s.dirs,
            &s.wt,
            "feat/x",
            None,
            &s.dir.path().join("c.index"),
        )
        .unwrap();
        assert_eq!(staged.changed, ["b.txt"]);
    }

    /// A repo whose git dir lives elsewhere (`--separate-git-dir`) still ships into its own
    /// checkout.
    #[test]
    fn a_repo_with_its_git_dir_elsewhere_ships_into_its_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        let gd = root.join("gd");
        git(
            &repo,
            &[
                "init",
                "--quiet",
                "--initial-branch=main",
                &format!("--separate-git-dir={}", gd.display()),
            ],
        );
        commit_file(&repo, "a.txt", "a\n");
        let wt = GitCli
            .create(&repo, "feat/x", "main", CreateMode::New)
            .unwrap();
        commit_file(&wt, "b.txt", "b\n");
        let mut dirs = GitCli.git_dirs(&repo, &wt).unwrap();
        dirs.launch = Launch::Unconfined;
        let index = root.join("private/i.index");
        let tip = git(&repo, &["rev-parse", "feat/x"]).trim().to_owned();
        let sessions = [wt.clone()];
        let into = MergeInto {
            base: "main",
            repo: &repo,
            sessions: &sessions,
        };
        let merge = plan_merge(&dirs, &wt, &index, &into, &tip, "merge").unwrap();
        assert_eq!(merge.checkout.as_deref(), Some(repo.as_path()));
        apply_merge(&dirs, &wt, &index, &merge).unwrap();
        assert_eq!(std::fs::read_to_string(repo.join("b.txt")).unwrap(), "b\n");
        // A repo path whose git dir isn't this one is refused.
        let other = root.join("other");
        std::fs::create_dir(&other).unwrap();
        git(&other, &["init", "--quiet"]);
        let into = MergeInto {
            base: "main",
            repo: &other,
            sessions: &sessions,
        };
        assert!(matches!(
            plan_merge(&dirs, &wt, &index, &into, &tip, "merge"),
            Err(ShipError::Layout(_))
        ));
    }

    /// Which worktree has the base branch checked out is read from the main checkout's own
    /// HEAD; a session's admin dir, which its agent can write, can't block a ship.
    #[test]
    fn a_session_claiming_the_base_branch_is_ignored() {
        let s = Setup::new();
        commit_file(&s.wt, "b.txt", "b\n");
        std::fs::write(s.dirs.admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let merge = s.plan().unwrap();
        assert_eq!(merge.checkout.as_deref(), Some(s.repo.as_path()));
        // The user's own linked worktree on the base branch is respected.
        git(&s.repo, &["checkout", "--quiet", "--detach"]);
        assert_eq!(s.plan().unwrap().checkout, None);
        let elsewhere = s.dir.path().join("elsewhere");
        git(
            &s.repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "--force",
                elsewhere.to_str().unwrap(),
                "main",
            ],
        );
        assert!(matches!(
            s.plan(),
            Err(ShipError::CheckedOutElsewhere { .. })
        ));
    }

    /// A remote given as a URL may hold a token, which never shows in the error.
    #[test]
    fn a_remote_url_with_a_token_is_never_echoed() {
        let s = Setup::new();
        git(
            &s.repo,
            &[
                "config",
                "branch.main.remote",
                "https://user:tok3n@example.invalid/r.git",
            ],
        );
        let tip = s.rev("feat/x");
        let err = push(&s.dirs, &s.wt, &s.index, "main", &tip, "feat/x").unwrap_err();
        assert!(matches!(err, ShipError::NoRemote(_)), "{err:?}");
        assert!(!format!("{err} {err:?}").contains("tok3n"), "{err:?}");
    }

    /// A merge that can't update the checkout leaves the base branch, and the checkout's
    /// tracked files, as they were.
    #[test]
    fn a_checkout_update_that_fails_moves_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let s = Setup::new();
        commit_file(&s.repo, "d/keep", "k\n");
        let base = s.rev("main");
        git(&s.wt, &["merge", "--quiet", "--ff-only", "main"]);
        commit_file(&s.wt, "d/new", "n\n");
        let merge = s.plan().unwrap();
        let d = s.repo.join("d");
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(d.join("probe"), "").is_ok() {
            // Running as root: permissions don't stop anything.
            return;
        }
        let result = apply_merge(&s.dirs, &s.wt, &s.index, &merge);
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(matches!(result, Err(ShipError::Dirty { .. })), "{result:?}");
        assert_eq!(s.rev("main"), base);
        assert_eq!(
            git(&s.repo, &["status", "--porcelain", "--untracked-files=no"]),
            ""
        );
    }

    #[test]
    fn git_errors_show_control_characters_escaped() {
        assert_eq!(
            commit::printable(b"fatal: bad \x1b[31mname\x07\n\tnext line\n"),
            "fatal: bad \\u{1b}[31mname\\u{7}\\u{a}\\u{9}next line"
        );
        assert_eq!(
            commit::printable("a\u{202E}b\u{2028}c".as_bytes()),
            "a\\u{202e}b\\u{2028}c"
        );
        assert!(commit::printable(&[b'x'; 10_000]).len() < 4200);
    }

    #[test]
    fn credentials_in_urls_are_scrubbed() {
        assert_eq!(
            scrub("fatal: unable to access 'https://x-token:ghp_secret@github.com/o/r.git/': 403"),
            "fatal: unable to access 'https://***@github.com/o/r.git/': 403"
        );
        assert_eq!(
            scrub("https://user@host/a and ssh://git@h:22/r http://plain/x"),
            "https://***@host/a and ssh://***@h:22/r http://plain/x"
        );
        assert_eq!(scrub("no url here"), "no url here");
        assert_eq!(scrub("https://a:b@c"), "https://***@c");
    }
}
