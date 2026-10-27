//! Commits the daemon makes on a session's branch for its agent (`git_commit`).
//!
//! Two steps, so whoever commits can check exactly what will be committed in between:
//! [`stage`] writes the worktree's changes into a tree, using an index the daemon owns, and
//! [`commit`] records that tree on the branch. Git runs with the repo's shared git dir and
//! explicit refs, never the worktree's `.git` link, git dir, or index, which whatever runs in
//! the worktree can rewrite; and the repo's hooks never run.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::confine::{self, Launch};
use crate::worktree::{GitDirs, GitError};

/// Most diff text kept for the secret scan; a larger change is reported as cut. Binary files
/// count too, since a file git would call binary can still hold a secret.
const MAX_DIFF: usize = 16 * 1024 * 1024;

/// Most bytes of changed files one commit may hold. Checked before anything is staged, so
/// an agent can't fill the daemon's memory, or the repo's object store, through `git_commit`.
pub const MAX_COMMIT_BYTES: u64 = 64 * 1024 * 1024;

/// Longest any one git command may run.
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

/// Most output read from any one git command. Past it git is killed and the commit refused:
/// the size check before staging is a moment's look, and a file can grow after it.
const MAX_GIT_OUTPUT: u64 = 32 * 1024 * 1024;

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
    #[error(
        "the changes are larger than the {} MiB one commit may hold; commit fewer paths at a time, or leave large files out",
        MAX_COMMIT_BYTES / 1024 / 1024
    )]
    TooLarge,
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
    if let Some(dir) = index.parent() {
        std::fs::create_dir_all(dir).map_err(|source| GitError::Io {
            context: format!("creating {}", dir.display()),
            source,
        })?;
    }
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
    drop_gitlinks(dirs, worktree, index)?;
    // What would be staged, by size on disk, before any of it is copied into the repo.
    let mut status = vec![
        "status",
        "--porcelain=v1",
        "-z",
        "--untracked-files=all",
        "--no-renames",
        "--ignore-submodules=all",
        "--",
    ];
    status.extend(pathspecs.iter().map(String::as_str));
    let mut total: u64 = 0;
    for entry in run(&status)?.split('\0').filter(|e| e.len() > 3) {
        if let Ok(meta) = std::fs::symlink_metadata(worktree.join(&entry[3..]))
            && meta.is_file()
        {
            total = total.saturating_add(meta.len());
            if total > MAX_COMMIT_BYTES {
                return Err(CommitError::TooLarge);
            }
        }
    }
    let mut add = vec!["add", "--all", "--"];
    add.extend(pathspecs.iter().map(String::as_str));
    run(&add)?;
    pin_gitlinks(dirs, worktree, index, &parent)?;
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
    // Every added line, as text: neither git's binary guess (a NUL byte near the start) nor an
    // attribute (`-diff`, `binary`) may keep one from the scan. Attributes come from the
    // branch tip, so this never reads the worktree.
    let diff = run_env(
        dirs,
        worktree,
        index,
        &[
            "diff-tree",
            "-p",
            "-U0",
            "--text",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            &parent,
            &tree,
        ],
        None,
        &[("GIT_ATTR_SOURCE", &parent)],
    )?;
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

/// Paths `index` holds as gitlinks: nested repositories, submodules or not.
fn gitlinks(dirs: &GitDirs, worktree: &Path, index: &Path) -> Result<Vec<String>, GitError> {
    Ok(run(dirs, worktree, index, &["ls-files", "-s", "-z"], None)?
        .split('\0')
        .filter(|e| e.starts_with("160000 "))
        .filter_map(|e| e.split_once('\t').map(|(_, path)| path.to_owned()))
        .collect())
}

/// Take every gitlink out of `index`. While one is there, `status`, `add`, and `diff-files`
/// look inside that nested repo with a git of their own, which reads the nested repo's config,
/// filters included; and inside a worktree, that config is the agent's to write.
fn drop_gitlinks(dirs: &GitDirs, worktree: &Path, index: &Path) -> Result<(), GitError> {
    let links = gitlinks(dirs, worktree, index)?;
    if links.is_empty() {
        return Ok(());
    }
    let mut input = links.join("\0");
    input.push('\0');
    run(
        dirs,
        worktree,
        index,
        &["update-index", "-z", "--force-remove", "--stdin"],
        Some(input.as_bytes()),
    )
    .map(drop)
}

/// Make `index`'s gitlinks exactly `parent`'s: nested repos are committed as the branch has
/// them, whatever the worktree holds, since the daemon never looks inside one. Only reads and
/// writes the index.
fn pin_gitlinks(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    parent: &str,
) -> Result<(), GitError> {
    drop_gitlinks(dirs, worktree, index)?;
    let listed = run(
        dirs,
        worktree,
        index,
        &["ls-tree", "-r", "-z", parent],
        None,
    )?;
    let pinned: String = listed
        .split('\0')
        .filter(|e| e.starts_with("160000 commit "))
        .map(|e| format!("{e}\0"))
        .collect();
    if pinned.is_empty() {
        return Ok(());
    }
    run(
        dirs,
        worktree,
        index,
        &["update-index", "-z", "--index-info"],
        Some(pinned.as_bytes()),
    )
    .map(drop)
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

/// The command that starts git for `args` on `worktree`: confined, for a command that opens
/// the worktree's files, as `dirs.launch` says.
fn launcher(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    args: &[&str],
) -> Result<Command, GitError> {
    let reach = confine::reach(args);
    // Never a bare `git`, which a relative `PATH` entry would find in the worktree.
    let plain = confine::plain;
    if reach == confine::Reach::Repo {
        // Out of the worktree too: it never opens its files, so it never runs in it.
        let mut cmd = plain()?;
        cmd.current_dir(&dirs.common);
        return Ok(cmd);
    }
    let mut cmd = match &dirs.launch {
        Launch::Unconfined => plain(),
        Launch::Plain(plain) => plain.git(&[], &[]).map_err(GitError::NotConfined),
        Launch::Confined(confined) => {
            let (mut read, write) = confine::paths(reach, dirs, worktree, index);
            read.extend(confined.read.iter().cloned());
            read.push(confined.config.clone());
            let mut cmd = confined
                .confine
                .git(&read, &write)
                .map_err(GitError::NotConfined)?;
            // The user's own config, as far as confined git may see it; and no default ignore or
            // attributes file of theirs, which it couldn't read: the snapshot names the ones in
            // use.
            cmd.env("GIT_CONFIG_GLOBAL", &confined.config)
                .env("XDG_CONFIG_HOME", confined.config.with_extension("none"));
            Ok(cmd)
        }
        Launch::Refused(why) => Err(GitError::NotConfined(why.clone())),
        Launch::Unset => Err(GitError::NotConfined(
            "no confinement was chosen for this worktree".into(),
        )),
    }?;
    cmd.current_dir(worktree);
    Ok(cmd)
}

/// Run git on `worktree` with the repo's shared git dir and `index`; its stdout on success.
fn run(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
) -> Result<String, GitError> {
    run_env(dirs, worktree, index, args, stdin, &[])
}

/// [`run`], with `env` added to git's environment.
fn run_env(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    env: &[(&str, &str)],
) -> Result<String, GitError> {
    let mut cmd = launcher(dirs, worktree, index, args)?;
    cmd.envs(env.iter().copied());
    // Nothing recurses into nested repos (their config is the agent's to write), and Git LFS
    // never downloads on the daemon's behalf because of an attribute the agent wrote.
    cmd.args([
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.fsmonitor=false",
        "-c",
        "submodule.recurse=false",
        "-c",
        "advice.addEmbeddedRepo=false",
    ])
    .env("GIT_LFS_SKIP_SMUDGE", "1")
    .args(args)
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
    // Read both pipes off this thread, so a full pipe can't stall the wait, and kill git if
    // it runs too long.
    let over = Arc::new(AtomicBool::new(false));
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        let over = over.clone();
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(pipe) = pipe {
                let _ = pipe.take(MAX_GIT_OUTPUT + 1).read_to_end(&mut out);
            }
            if out.len() as u64 > MAX_GIT_OUTPUT {
                over.store(true, Ordering::SeqCst);
            }
            out
        })
    };
    let boxed = |p: Option<Box<dyn Read + Send>>| p;
    let stdout = reader(boxed(child.stdout.take().map(|p| Box::new(p) as _)));
    let stderr = reader(boxed(child.stderr.take().map(|p| Box::new(p) as _)));
    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            // Past the cap nobody reads its output, so git would wait forever.
            Ok(None) if over.load(Ordering::SeqCst) => {
                let _ = child.kill();
                break child.wait().map_err(|source| GitError::Io {
                    context: "running git".into(),
                    source,
                })?;
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(GitError::Command {
                    args: args.join(" "),
                    stderr: format!("timed out after {} s", GIT_TIMEOUT.as_secs()),
                });
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
            Err(source) => {
                return Err(GitError::Io {
                    context: "running git".into(),
                    source,
                });
            }
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if over.load(Ordering::SeqCst) {
        return Err(GitError::Command {
            args: args.join(" "),
            stderr: format!(
                "output over {} MiB, too large to check; commit fewer paths at a time",
                MAX_GIT_OUTPUT / 1024 / 1024
            ),
        });
    }
    if !status.success() {
        return Err(GitError::Command {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&stdout).into_owned())
}

/// The largest committed file [`committed_file`] reads.
const MAX_COMMITTED_FILE: u64 = 1024 * 1024;

/// What the tip of `branch` holds at `path`: a regular file's contents, `None` when the tree
/// has nothing there, and an error for anything else there (a symlink, a directory, a file
/// over 1 MiB). Read from the repo's objects, never from the worktree, which the agent can
/// write.
pub fn committed_file(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    branch: &str,
    path: &str,
) -> Result<Option<String>, GitError> {
    let rev = format!("refs/heads/{branch}");
    let listed = run(
        dirs,
        worktree,
        index,
        &["ls-tree", "-z", &rev, "--", path],
        None,
    )?;
    let Some(entry) = listed.split('\0').find(|e| !e.is_empty()) else {
        return Ok(None);
    };
    let odd = |why: &str| GitError::Command {
        args: format!("ls-tree {rev} -- {path}"),
        stderr: format!("{path} on {branch} {why}"),
    };
    let (meta, _) = entry.split_once('\t').ok_or_else(|| odd("is unreadable"))?;
    let mut fields = meta.split(' ');
    let (mode, kind, object) = (fields.next(), fields.next(), fields.next());
    let object = match (mode, kind, object) {
        (Some("100644" | "100755"), Some("blob"), Some(object)) => object,
        _ => return Err(odd("is not a regular file")),
    };
    let size: u64 = run(dirs, worktree, index, &["cat-file", "-s", object], None)?
        .trim()
        .parse()
        .map_err(|_| odd("has no size"))?;
    if size > MAX_COMMITTED_FILE {
        return Err(odd("is over 1 MiB"));
    }
    run(dirs, worktree, index, &["cat-file", "blob", object], None).map(Some)
}

/// Where a session keeps the index its commits are staged in: a dir of its own in the
/// session's daemon-owned dir, which no sandbox can write, and the only part of it that the
/// daemon's confined git may (see [`crate::confine`]).
pub fn index_path(private_dir: &Path) -> PathBuf {
    private_dir.join("git").join("git_commit.index")
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
        let mut dirs = GitCli.git_dirs(&repo, &wt).unwrap();
        dirs.launch = Launch::Unconfined;
        let private = root.join("private");
        std::fs::create_dir(&private).unwrap();
        (dir, repo, wt, dirs, private)
    }

    #[test]
    fn committed_files_come_from_the_branch_not_the_worktree() {
        let (_dir, _repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        let read = |path: &str| committed_file(&dirs, &wt, &index, "feat/x", path);
        assert_eq!(read("a.txt").unwrap().as_deref(), Some("a\n"));
        assert_eq!(read(".claude/settings.json").unwrap(), None);
        // What the agent writes in the worktree doesn't count until it is committed.
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        std::fs::create_dir_all(wt.join(".claude")).unwrap();
        std::fs::write(wt.join(".claude/settings.json"), "{}").unwrap();
        assert_eq!(read("a.txt").unwrap().as_deref(), Some("a\n"));
        assert_eq!(read(".claude/settings.json").unwrap(), None);
        // A committed symlink or directory is refused, not followed.
        std::os::unix::fs::symlink("/etc/passwd", wt.join("link")).unwrap();
        git(&wt, &["add", "link", ".claude"]);
        git(&wt, &["commit", "--quiet", "-m", "link"]);
        assert!(read("link").is_err());
        assert!(read(".claude").is_err());
        assert_eq!(
            read(".claude/settings.json").unwrap().as_deref(),
            Some("{}")
        );
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
    fn changes_too_large_for_one_commit_are_refused_before_staging() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        let big = std::fs::File::create(wt.join("big.bin")).unwrap();
        big.set_len(MAX_COMMIT_BYTES + 1).unwrap();
        assert!(matches!(
            stage(&dirs, &wt, "feat/x", None, &index),
            Err(CommitError::TooLarge)
        ));
        // Nothing of it reached the repo's object store.
        let objects = git(&repo, &["count-objects", "-v"]);
        let size: u64 = objects
            .lines()
            .find_map(|l| l.strip_prefix("size: "))
            .unwrap()
            .parse()
            .unwrap();
        assert!(size < 1024, "{objects}");
        // A file at a path the commit leaves out doesn't count.
        std::fs::write(wt.join("small.txt"), "s\n").unwrap();
        let staged = stage(
            &dirs,
            &wt,
            "feat/x",
            Some(&["small.txt".to_owned()]),
            &index,
        )
        .unwrap();
        assert_eq!(staged.changed, ["small.txt"]);
    }

    #[test]
    fn git_output_past_the_cap_refuses_the_commit_instead_of_filling_memory() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        // Under the size check, but its diff is more than the daemon reads.
        let line = "x".repeat(1023) + "\n";
        std::fs::write(wt.join("big.txt"), line.repeat(40 * 1024)).unwrap();
        let started = Instant::now();
        match stage(&dirs, &wt, "feat/x", None, &index) {
            Err(CommitError::Git(GitError::Command { stderr, .. })) => {
                assert!(stderr.contains("output over"), "{stderr}");
            }
            other => panic!("{other:?}"),
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "git was killed, not waited out"
        );
        assert_eq!(
            git(&repo, &["rev-parse", "feat/x"]),
            git(&repo, &["rev-parse", "main"])
        );
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

    /// A filter in `repo`'s own config that leaves `mark` when anything runs it, and a file it
    /// applies to.
    fn plant_filter(repo: &Path, mark: &Path) {
        git(
            repo,
            &[
                "config",
                "filter.x.clean",
                &format!("touch {}; cat", mark.display()),
            ],
        );
        std::fs::write(repo.join(".gitattributes"), "* filter=x\n").unwrap();
    }

    /// A repo at `dir` with one commit.
    fn nested_repo(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "--quiet"]);
        std::fs::write(dir.join("f"), "x\n").unwrap();
        git(dir, &[&ID[..], &["add", "f"]].concat());
        git(dir, &[&ID[..], &["commit", "--quiet", "-m", "f"]].concat());
    }

    const ID: [&str; 4] = ["-c", "user.name=A", "-c", "user.email=a@x"];

    #[test]
    fn a_nested_repo_the_agent_made_never_makes_the_daemon_run_its_config() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        let mark = private.join("filter-ran");
        let sub = wt.join("sub");
        nested_repo(&sub);
        plant_filter(&sub, &mark);
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        // The branch has no nested repo there, so the commit doesn't either.
        assert_eq!(staged.changed, ["a.txt"]);
        commit(&dirs, &wt, "feat/x", &staged, "m", &index).unwrap();
        // Same size, new contents: stat-dirty, so git would look inside again.
        std::fs::write(sub.join("f"), "y\n").unwrap();
        std::fs::write(wt.join("a.txt"), "again\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        assert_eq!(staged.changed, ["a.txt"]);
        assert!(!mark.exists(), "the nested repo's filter ran");
        assert!(
            !git(&repo, &["ls-tree", "-r", "feat/x"]).contains("sub"),
            "a nested repo reached the branch"
        );
    }

    #[test]
    fn a_nested_repo_the_branch_holds_stays_where_the_branch_has_it() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        let mark = private.join("filter-ran");
        let sub = wt.join("sub");
        nested_repo(&sub);
        // Committed on the branch by hand as an embedded repo: a gitlink, no .gitmodules.
        git(&wt, &[&ID[..], &["add", "sub"]].concat());
        git(
            &wt,
            &[&ID[..], &["commit", "--quiet", "-m", "sub"]].concat(),
        );
        let pinned = git(&repo, &["rev-parse", "feat/x:sub"]);
        // The agent moves the nested repo on, plants a filter, and dirties a file.
        std::fs::write(sub.join("g"), "g\n").unwrap();
        git(&sub, &[&ID[..], &["add", "g"]].concat());
        git(&sub, &[&ID[..], &["commit", "--quiet", "-m", "g"]].concat());
        plant_filter(&sub, &mark);
        std::fs::write(sub.join("f"), "y\n").unwrap();
        // Nothing but the nested repo changed: nothing to commit.
        assert!(matches!(
            stage(&dirs, &wt, "feat/x", None, &index),
            Err(CommitError::NothingToCommit)
        ));
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        assert_eq!(staged.changed, ["a.txt"]);
        commit(&dirs, &wt, "feat/x", &staged, "m", &index).unwrap();
        std::fs::write(sub.join("f"), "z\n").unwrap();
        std::fs::write(wt.join("a.txt"), "again\n").unwrap();
        stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        assert!(!mark.exists(), "the nested repo's filter ran");
        assert_eq!(git(&repo, &["rev-parse", "feat/x:sub"]), pinned);
    }

    #[test]
    fn a_submodule_stays_where_the_branch_has_it() {
        let (_dir, repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        let mark = private.join("filter-ran");
        let lib = private.join("lib");
        nested_repo(&lib);
        let url = lib.display().to_string();
        git(
            &wt,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "--quiet",
                &url,
                "lib",
            ],
        );
        git(
            &wt,
            &[&ID[..], &["commit", "--quiet", "-m", "lib"]].concat(),
        );
        let pinned = git(&repo, &["rev-parse", "feat/x:lib"]);
        let sub = wt.join("lib");
        std::fs::write(sub.join("g"), "g\n").unwrap();
        git(&sub, &[&ID[..], &["add", "g"]].concat());
        git(&sub, &[&ID[..], &["commit", "--quiet", "-m", "g"]].concat());
        plant_filter(&sub, &mark);
        std::fs::write(sub.join("f"), "y\n").unwrap();
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        assert_eq!(staged.changed, ["a.txt"]);
        commit(&dirs, &wt, "feat/x", &staged, "m", &index).unwrap();
        std::fs::write(sub.join("f"), "z\n").unwrap();
        std::fs::write(wt.join("a.txt"), "again\n").unwrap();
        stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        assert!(!mark.exists(), "the submodule's filter ran");
        assert_eq!(git(&repo, &["rev-parse", "feat/x:lib"]), pinned);
    }

    #[test]
    fn without_a_confinement_nothing_opens_the_worktree() {
        for launch in [Launch::Unset, Launch::Refused("no Landlock".into())] {
            let (_dir, repo, wt, mut dirs, private) = setup();
            dirs.launch = launch;
            let index = index_path(&private);
            std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
            match stage(&dirs, &wt, "feat/x", None, &index) {
                Err(CommitError::Git(GitError::NotConfined(_))) => {}
                other => panic!("{other:?}"),
            }
            assert_eq!(
                git(&repo, &["rev-parse", "feat/x"]),
                git(&repo, &["rev-parse", "main"])
            );
            // What only reads the repo's objects still works.
            assert_eq!(
                committed_file(&dirs, &wt, &index, "feat/x", "a.txt")
                    .unwrap()
                    .as_deref(),
                Some("a\n")
            );
        }
    }

    /// Runs git plainly, recording what each command was allowed.
    #[derive(Default)]
    struct Recording(std::sync::Mutex<Vec<(Vec<PathBuf>, Vec<PathBuf>)>>);

    impl crate::Confine for Recording {
        fn git(&self, read: &[PathBuf], write: &[PathBuf]) -> Result<Command, String> {
            self.0.lock().unwrap().push((read.to_vec(), write.to_vec()));
            Ok(Command::new("git"))
        }
    }

    #[test]
    fn commands_that_open_the_worktree_go_through_the_confinement() {
        let (_dir, repo, wt, mut dirs, private) = setup();
        let recording = std::sync::Arc::new(Recording::default());
        let config = private.join("git-config");
        crate::confine::write_snapshot(&Default::default(), &config).unwrap();
        dirs.launch = Launch::Confined(crate::confine::Confined {
            confine: recording.clone(),
            config: config.clone(),
            read: vec![],
        });
        let index = index_path(&private);
        std::fs::write(wt.join("a.txt"), "changed\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        commit(&dirs, &wt, "feat/x", &staged, "m", &index).unwrap();
        let calls = recording.0.lock().unwrap().clone();
        // Staging's read-tree, status, add, and write-tree, then the worktree's own read-tree
        // and refresh: everything but reading objects and refs, and moving the branch.
        assert_eq!(calls.len(), 6, "{calls:?}");
        // Git's default ignore and attributes files are looked for where nothing is.
        let launched = launcher(&dirs, &wt, &index, &["status"]).unwrap();
        let xdg = launched
            .get_envs()
            .find(|(k, _)| *k == "XDG_CONFIG_HOME")
            .and_then(|(_, v)| v)
            .map(PathBuf::from)
            .unwrap();
        assert!(!xdg.exists(), "{xdg:?}");
        let objects = dirs.common.join("objects");
        assert!(calls.iter().any(|(_, write)| write.contains(&objects)));
        for (read, write) in calls.iter() {
            assert!(read.contains(&wt) && read.contains(&config));
            assert!(write.contains(&private.join("git")) || write.contains(&dirs.admin));
            assert!(!write.contains(&dirs.common) && !write.contains(&private));
            assert!(!write.iter().any(|w| w.starts_with(repo.join(".git/refs"))));
        }
    }

    #[test]
    fn worktree_attributes_dont_hide_added_lines_from_the_scan() {
        let (_dir, _repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        // Not committed, so not among the changed paths: only the attributes git would read.
        std::fs::write(wt.join(".gitattributes"), "* -diff binary\n").unwrap();
        std::fs::write(wt.join("a.txt"), "token = sk-live-1234\n").unwrap();
        let paths = vec!["a.txt".to_owned()];
        let staged = stage(&dirs, &wt, "feat/x", Some(&paths), &index).unwrap();
        assert_eq!(staged.changed, ["a.txt"]);
        assert!(staged.added.contains("sk-live-1234"), "{staged:?}");
    }

    #[test]
    fn a_file_git_would_call_binary_still_reaches_the_scan() {
        let (_dir, _repo, wt, dirs, private) = setup();
        let index = index_path(&private);
        std::fs::write(wt.join("blob.bin"), "\0token = sk-live-5678\n").unwrap();
        let staged = stage(&dirs, &wt, "feat/x", None, &index).unwrap();
        assert!(staged.added.contains("sk-live-5678"), "{staged:?}");
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
