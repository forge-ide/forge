//! Checkpoints: snapshots of a session's worktree, untracked files included, as commits on
//! shadow refs (`refs/forge/<session>/cp/<n>`), and rewinding the worktree to one.
//!
//! Like [`crate::commit`], git runs with the repo's shared git dir, an index only the daemon
//! writes, and explicit refs, never the worktree's `.git` link, git dir, or index; and the
//! repo's hooks never run. Ignored files stay out, as they do for `git_commit`, except a
//! `.gitignore` that is itself ignored: a new `foo/.gitignore` holding `*` would otherwise hide
//! itself and everything beside it from the review.

use std::path::Path;

use crate::commit::{self, CommitError, Record};
use crate::worktree::{GitDirs, GitError, in_worktrees_dir};

/// Who checkpoint commits are by. They live on Forge's own refs, so they don't need the
/// user's identity, and a machine without one can still take them.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "Forge"),
    ("GIT_AUTHOR_EMAIL", "forge@localhost"),
    ("GIT_COMMITTER_NAME", "Forge"),
    ("GIT_COMMITTER_EMAIL", "forge@localhost"),
];

/// The shadow ref of a session's `n`th checkpoint.
pub fn checkpoint_ref(session: &str, n: u32) -> String {
    format!("refs/forge/{session}/cp/{n}")
}

/// Snapshot `worktree` into a commit on top of `branch`'s tip and point `reference`, which
/// must not exist yet, at it. `previous` is the session's latest checkpoint commit: when
/// neither the files nor the branch tip changed since, it is reused rather than copied.
/// The size cap counts only what changed since `previous`. `index` is kept between calls, so
/// git's stat cache spares it re-reading unchanged files. Returns the commit.
pub fn take(
    dirs: &GitDirs,
    worktree: &Path,
    branch: &str,
    previous: Option<&str>,
    index: &Path,
    reference: &str,
    message: &str,
) -> Result<String, CommitError> {
    let parent = tip(dirs, worktree, index, branch)?;
    let tree = snapshot(dirs, worktree, index, previous.unwrap_or(&parent), &parent)?;
    if let Some(previous) = previous
        && tree_of(dirs, worktree, index, previous)? == tree
        && parent_of(dirs, worktree, index, previous)? == parent
    {
        commit::point(
            dirs,
            worktree,
            index,
            reference,
            previous,
            "",
            "forge: checkpoint",
        )?;
        return Ok(previous.to_owned());
    }
    commit::record(
        dirs,
        worktree,
        index,
        Record {
            tree: &tree,
            parent: &parent,
            message,
            reference,
            old: "",
            log: "forge: checkpoint",
            env: &IDENTITY,
        },
    )
}

/// Stage `worktree` as a checkpoint would, into `index` loaded with `base`'s tree, with
/// nested repos pinned as `parent` has them; returns the tree.
fn snapshot(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    base: &str,
    parent: &str,
) -> Result<String, CommitError> {
    let run = |args: &[&str]| commit::run(dirs, worktree, index, args, None);
    load(dirs, worktree, index, base)?;
    commit::drop_gitlinks(dirs, worktree, index)?;
    // Ignored dirs are listed as one entry each, so a `.gitignore` only shows here when it is
    // itself ignored by a rule beside it or below an unignored dir.
    let ignored = run(&[
        "ls-files",
        "-z",
        "--others",
        "--ignored",
        "--exclude-standard",
        "--directory",
    ])?;
    let hiding: Vec<&str> = ignored
        .split('\0')
        .filter(|p| *p == ".gitignore" || p.ends_with("/.gitignore"))
        .filter(|p| !in_worktrees_dir(p.as_bytes()))
        .collect();
    let all = [".".to_owned()];
    commit::check_size(dirs, worktree, index, &all, &hiding)?;
    run(&["add", "--all", "--", "."])?;
    if !hiding.is_empty() {
        let mut add = vec!["add", "--force", "--"];
        add.extend(hiding);
        run(&add)?;
    }
    commit::drop_worktrees_dir(dirs, worktree, index, parent)?;
    commit::pin_gitlinks(dirs, worktree, index, parent)?;
    Ok(run(&["write-tree"])?.trim().to_owned())
}

/// Refuse unless `worktree` and `branch` are still exactly as checkpoint `checkpoint` has
/// them: the branch at its parent, or at a commit on top of it with the checkpoint's own tree
/// (what was uncommitted, committed since by Forge), and a snapshot taken now the same tree.
/// Leaves `index` holding that tree. Returns the branch tip.
pub fn unchanged(
    dirs: &GitDirs,
    worktree: &Path,
    branch: &str,
    checkpoint: &str,
    index: &Path,
) -> Result<String, CommitError> {
    let parent = parent_of(dirs, worktree, index, checkpoint)?;
    let tip = tip(dirs, worktree, index, branch)?;
    if tip != parent
        && (parent_of(dirs, worktree, index, &tip).ok().as_deref() != Some(parent.as_str())
            || tree_of(dirs, worktree, index, &tip)? != tree_of(dirs, worktree, index, checkpoint)?)
    {
        return Err(CommitError::Moved(branch.to_owned()));
    }
    let tree = match snapshot(dirs, worktree, index, checkpoint, &parent) {
        Err(CommitError::TooLarge) => return Err(CommitError::WorktreeChanged),
        tree => tree?,
    };
    if tree != tree_of(dirs, worktree, index, checkpoint)? {
        return Err(CommitError::WorktreeChanged);
    }
    Ok(tip)
}

/// Rewind `worktree` from checkpoint `from`, taken just now, to checkpoint `to`, and move
/// `branch` back to where it was when `to` was taken. Refuses, changing nothing, when the
/// worktree no longer matches `from` or the branch moved since. Ignored files are left alone.
pub fn rewind(
    dirs: &GitDirs,
    worktree: &Path,
    branch: &str,
    from: &str,
    to: &str,
    index: &Path,
) -> Result<(), CommitError> {
    let run = |args: &[&str]| commit::run(dirs, worktree, index, args, None);
    let (from_parent, to_parent) = (
        parent_of(dirs, worktree, index, from)?,
        parent_of(dirs, worktree, index, to)?,
    );
    if tip(dirs, worktree, index, branch)? != from_parent {
        return Err(CommitError::Moved(branch.to_owned()));
    }
    load(dirs, worktree, index, from)?;
    // Anything that changed after the snapshot, tracked or not, would be lost or kept by
    // accident; refuse instead.
    let _ = run(&["update-index", "-q", "--refresh"]);
    let changed = run(&["diff-files", "--name-only", "-z", "--ignore-submodules=all"])?;
    // A nested repo is listed as its dir, with a trailing slash; checkpoints never look inside
    // one, so neither does this.
    let added = run(&["ls-files", "-z", "--others", "--exclude-standard"])?;
    let added = added
        .split('\0')
        .any(|p| !p.is_empty() && !p.ends_with('/'));
    if !changed.is_empty() || added {
        return Err(CommitError::WorktreeChanged);
    }
    run(&["read-tree", "-m", "-u", "-n", from, to]).map_err(|_| CommitError::WorktreeChanged)?;
    let reference = format!("refs/heads/{branch}");
    let moved = from_parent != to_parent;
    if moved {
        commit::point(
            dirs,
            worktree,
            index,
            &reference,
            &to_parent,
            &from_parent,
            "forge: rewind",
        )
        .map_err(|_| CommitError::Moved(branch.to_owned()))?;
    }
    // Git updates the files without following a symlink in their leading path. If it stops
    // partway (a file changed meanwhile, a timeout), the worktree is put at `to` outright,
    // since nothing in it was left unsnapshotted.
    if let Err(e) = run(&["read-tree", "-m", "-u", from, to])
        && let Err(reset) = run(&["read-tree", "--reset", "-u", to])
    {
        tracing::warn!(error = %e, "rewinding the worktree stopped partway");
        if moved {
            let _ = commit::point(
                dirs,
                worktree,
                index,
                &reference,
                &from_parent,
                &to_parent,
                "forge: rewind undone",
            );
        }
        return Err(CommitError::PartlyRewound {
            from: from.to_owned(),
            reason: reset.to_string(),
        });
    }
    commit::sync_own_index(dirs, worktree, &to_parent);
    Ok(())
}

/// Make `index` hold `commit`'s tree, keeping it as it is when it already does, so its stat
/// cache is kept. Callers serialize their uses of `index`.
pub(crate) fn load(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    commit: &str,
) -> Result<(), CommitError> {
    if let Some(dir) = index.parent() {
        std::fs::create_dir_all(dir).map_err(|source| GitError::Io {
            context: format!("creating {}", dir.display()),
            source,
        })?;
    }
    commit::clear_lock(index);
    let run = |args: &[&str]| commit::run(dirs, worktree, index, args, None);
    let want = tree_of(dirs, worktree, index, commit)?;
    let current = if index.exists() {
        run(&["write-tree"]).ok().map(|t| t.trim().to_owned())
    } else {
        None
    };
    if current.as_deref() != Some(want.as_str()) {
        let _ = std::fs::remove_file(index);
        run(&["read-tree", commit])?;
    }
    Ok(())
}

pub(crate) fn tip(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    branch: &str,
) -> Result<String, CommitError> {
    rev(
        dirs,
        worktree,
        index,
        &format!("refs/heads/{branch}^{{commit}}"),
    )
}

pub(crate) fn tree_of(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    commit: &str,
) -> Result<String, CommitError> {
    rev(dirs, worktree, index, &format!("{commit}^{{tree}}"))
}

pub(crate) fn parent_of(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    commit: &str,
) -> Result<String, CommitError> {
    rev(dirs, worktree, index, &format!("{commit}^1"))
}

fn rev(dirs: &GitDirs, worktree: &Path, index: &Path, spec: &str) -> Result<String, CommitError> {
    Ok(commit::run(
        dirs,
        worktree,
        index,
        &["rev-parse", "--verify", "--end-of-options", spec],
        None,
    )?
    .trim()
    .to_owned())
}

/// Where a session keeps the index its checkpoints of `repo` are staged in: beside
/// `git_commit`'s, in the session's daemon-owned `git/` dir, the only part of it the daemon's
/// confined git may write (see `forge_git::confine`). Repo names come from `forge.toml`, so
/// the file is named by the name's bytes in hex.
pub fn index_path(private_dir: &Path, repo: &str) -> std::path::PathBuf {
    let hex: String = repo.bytes().map(|b| format!("{b:02x}")).collect();
    private_dir
        .join("git")
        .join(format!("checkpoint-{hex}.index"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worktree::{CreateMode, GitCli, Worktrees};
    use std::path::PathBuf;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    struct Setup {
        _dir: tempfile::TempDir,
        repo: PathBuf,
        wt: PathBuf,
        dirs: GitDirs,
        private: PathBuf,
        index: PathBuf,
    }

    impl Setup {
        /// A repo on `main` ignoring `target/`, with a session worktree on `feat/x`.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let repo = root.join("repo");
            std::fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "--quiet", "--initial-branch=main"]);
            git(&repo, &["config", "user.name", "T"]);
            git(&repo, &["config", "user.email", "t@example.com"]);
            std::fs::write(repo.join("a.txt"), "a\n").unwrap();
            std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
            git(&repo, &["add", "a.txt", ".gitignore"]);
            git(&repo, &["commit", "--quiet", "-m", "initial"]);
            let wt = GitCli
                .create(&repo, "feat/x", "main", CreateMode::New)
                .unwrap();
            let mut dirs = GitCli.git_dirs(&repo, &wt).unwrap();
            dirs.launch = crate::Launch::Unconfined;
            let private = root.join("private");
            std::fs::create_dir(&private).unwrap();
            let index = index_path(&private, "repo");
            Setup {
                _dir: dir,
                repo,
                wt,
                dirs,
                private,
                index,
            }
        }

        fn take(&self, n: u32, previous: Option<&str>) -> Result<String, CommitError> {
            take(
                &self.dirs,
                &self.wt,
                "feat/x",
                previous,
                &self.index,
                &checkpoint_ref("ses_1", n),
                &format!("checkpoint {n}"),
            )
        }

        fn rewind(&self, from: &str, to: &str) -> Result<(), CommitError> {
            rewind(&self.dirs, &self.wt, "feat/x", from, to, &self.index)
        }

        /// The paths a commit's tree holds.
        fn files(&self, commit: &str) -> Vec<String> {
            git(&self.repo, &["ls-tree", "-r", "--name-only", commit])
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn rev(&self, rev: &str) -> String {
            git(&self.repo, &["rev-parse", rev]).trim().to_owned()
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.wt.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    /// The performance target (quality-and-safety.md): a checkpoint of a 50k-file repo in
    /// under 500 ms, once the session's index has been built by its first one. Slow to set up,
    /// so it runs on demand: `cargo test -p forge-git --release -- --ignored`.
    #[test]
    #[ignore]
    fn a_checkpoint_of_a_50k_file_repo_is_fast() {
        let s = Setup::new();
        for d in 0..500 {
            let dir = s.repo.join(format!("src/d{d}"));
            std::fs::create_dir_all(&dir).unwrap();
            for f in 0..100 {
                std::fs::write(dir.join(format!("f{f}.rs")), format!("// {d} {f}\n")).unwrap();
            }
        }
        git(&s.repo, &["add", "src"]);
        git(&s.repo, &["commit", "--quiet", "-m", "many files"]);
        let main = s.rev("main");
        git(&s.repo, &["update-ref", "refs/heads/feat/x", &main]);
        git(&s.wt, &["reset", "--quiet", "--hard", &main]);
        let started = std::time::Instant::now();
        let mut previous = s.take(1, None).unwrap();
        let first = started.elapsed();
        let mut slowest = std::time::Duration::ZERO;
        for n in 2..=6 {
            s.write(&format!("src/d{n}/f0.rs"), &format!("// turn {n}\n"));
            let started = std::time::Instant::now();
            previous = s.take(n, Some(&previous)).unwrap();
            slowest = slowest.max(started.elapsed());
        }
        eprintln!("first checkpoint {first:?}, slowest later one {slowest:?}");
        assert!(
            slowest < std::time::Duration::from_millis(500),
            "{slowest:?}"
        );
    }

    #[test]
    fn a_checkpoint_holds_changed_new_and_deleted_files_but_not_ignored_ones() {
        let s = Setup::new();
        s.write("a.txt", "changed\n");
        s.write("src/new.rs", "fn f() {}\n");
        s.write("target/out.bin", "build output\n");
        let cp = s.take(1, None).unwrap();
        assert_eq!(s.files(&cp), [".gitignore", "a.txt", "src/new.rs"]);
        assert_eq!(git(&s.repo, &["show", &format!("{cp}:a.txt")]), "changed\n");
        // On its shadow ref, on top of the branch tip, which didn't move.
        assert_eq!(s.rev("refs/forge/ses_1/cp/1"), cp);
        assert_eq!(s.rev(&format!("{cp}^")), s.rev("feat/x"));
        assert_eq!(s.rev("feat/x"), s.rev("main"));
        // The agent's own index and the worktree are as they were.
        assert_eq!(
            git(&s.wt, &["status", "--porcelain"]),
            " M a.txt\n?? src/\n"
        );

        std::fs::remove_file(s.wt.join("a.txt")).unwrap();
        let cp2 = s.take(2, Some(&cp)).unwrap();
        assert_eq!(s.files(&cp2), [".gitignore", "src/new.rs"]);
    }

    #[test]
    fn an_ignore_file_that_hides_itself_is_still_checkpointed() {
        let s = Setup::new();
        s.write("hidden/.gitignore", "*\n");
        s.write("hidden/payload.rs", "evil\n");
        // A .gitignore inside a directory an unchanged rule already ignores stays out.
        s.write("target/.gitignore", "*\n");
        let cp = s.take(1, None).unwrap();
        assert_eq!(s.files(&cp), [".gitignore", "a.txt", "hidden/.gitignore"]);
    }

    #[test]
    fn a_lock_left_by_a_killed_git_doesnt_stop_later_checkpoints() {
        let s = Setup::new();
        s.write("b.txt", "b\n");
        let cp1 = s.take(1, None).unwrap();
        // Git killed past its time or output limit leaves its lock behind.
        let mut lock = s.index.clone().into_os_string();
        lock.push(".lock");
        std::fs::write(&lock, "").unwrap();
        s.write("b.txt", "bb\n");
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        std::fs::write(&lock, "").unwrap();
        s.rewind(&cp2, &cp1).unwrap();
        assert_eq!(std::fs::read_to_string(s.wt.join("b.txt")).unwrap(), "b\n");
    }

    #[test]
    fn an_unchanged_worktree_reuses_the_previous_checkpoint() {
        let s = Setup::new();
        s.write("b.txt", "b\n");
        let cp1 = s.take(1, None).unwrap();
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        assert_eq!(cp2, cp1);
        assert_eq!(s.rev("refs/forge/ses_1/cp/2"), cp1);
        // The same files on a branch that moved are a new checkpoint on the new tip.
        let tree = s.rev(&format!("{cp1}^{{tree}}"));
        let moved = git(
            &s.repo,
            &["commit-tree", &tree, "-p", &s.rev("feat/x"), "-m", "agent"],
        );
        git(&s.repo, &["update-ref", "refs/heads/feat/x", moved.trim()]);
        let cp3 = s.take(3, Some(&cp1)).unwrap();
        assert_ne!(cp3, cp1);
        assert_eq!(s.rev(&format!("{cp3}^")), moved.trim());
    }

    #[test]
    fn an_existing_checkpoint_ref_is_never_overwritten() {
        let s = Setup::new();
        s.write("b.txt", "b\n");
        let cp1 = s.take(1, None).unwrap();
        s.write("b.txt", "bb\n");
        assert!(s.take(1, Some(&cp1)).is_err());
        assert_eq!(s.rev("refs/forge/ses_1/cp/1"), cp1);
    }

    #[test]
    fn the_size_cap_counts_only_what_changed_since_the_previous_checkpoint() {
        let s = Setup::new();
        let size = commit::MAX_COMMIT_BYTES * 5 / 8;
        let file = |name: &str, byte: u8| {
            std::fs::write(s.wt.join(name), vec![byte; size as usize]).unwrap();
        };
        file("one.bin", b'1');
        let cp1 = s.take(1, None).unwrap();
        file("two.bin", b'2');
        // Both files together are over the cap; only the new one counts.
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        assert_eq!(s.files(&cp2), [".gitignore", "a.txt", "one.bin", "two.bin"]);
        let big = std::fs::File::create(s.wt.join("big.bin")).unwrap();
        big.set_len(commit::MAX_COMMIT_BYTES + 1).unwrap();
        assert!(matches!(s.take(3, Some(&cp2)), Err(CommitError::TooLarge)));
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&s.repo)
                .args(["rev-parse", "--verify", "--quiet", "refs/forge/ses_1/cp/3"])
                .output()
                .unwrap()
                .stdout
                .is_empty()
        );
    }

    #[test]
    fn checkpoints_never_run_hooks_or_follow_the_worktrees_git_link() {
        let s = Setup::new();
        let hooks = s.repo.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let mark = s.private.join("hook-ran");
        for hook in ["reference-transaction", "post-checkout", "pre-commit"] {
            let path = hooks.join(hook);
            std::fs::write(&path, format!("#!/bin/sh\ntouch {}\n", mark.display())).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let other = s.private.join("other");
        std::fs::create_dir(&other).unwrap();
        git(&other, &["init", "--quiet"]);
        std::fs::write(
            s.wt.join(".git"),
            format!("gitdir: {}/.git\n", other.display()),
        )
        .unwrap();
        s.write("c.txt", "c\n");
        let cp1 = s.take(1, None).unwrap();
        assert!(s.files(&cp1).contains(&"c.txt".to_owned()));
        s.write("d.txt", "d\n");
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        s.rewind(&cp2, &cp1).unwrap();
        assert!(!mark.exists(), "a repo hook ran");
        assert!(
            git(&other, &["for-each-ref"]).is_empty(),
            "wrote to the repo the .git link names"
        );
    }

    #[test]
    fn a_nested_repo_never_makes_the_daemon_run_its_config() {
        let s = Setup::new();
        let mark = s.private.join("filter-ran");
        let sub = s.wt.join("sub");
        std::fs::create_dir(&sub).unwrap();
        git(&sub, &["init", "--quiet"]);
        std::fs::write(sub.join("f"), "x\n").unwrap();
        git(
            &sub,
            &["-c", "user.name=A", "-c", "user.email=a@x", "add", "f"],
        );
        git(
            &sub,
            &[
                "-c",
                "user.name=A",
                "-c",
                "user.email=a@x",
                "commit",
                "--quiet",
                "-m",
                "f",
            ],
        );
        // The agent's own repo names a filter that would run whatever it says.
        git(
            &sub,
            &[
                "config",
                "filter.x.clean",
                &format!("touch {}; cat", mark.display()),
            ],
        );
        std::fs::write(sub.join(".gitattributes"), "* filter=x\n").unwrap();
        let cp1 = s.take(1, None).unwrap();
        // Same size, new contents: stat-dirty, so git would look inside again.
        std::fs::write(sub.join("f"), "y\n").unwrap();
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        // The nested repo isn't in either checkpoint, and doesn't stop a rewind.
        s.rewind(&cp2, &cp1).unwrap();
        assert!(!mark.exists(), "the nested repo's filter ran");
        // The branch has no nested repo there, so neither does any checkpoint.
        for cp in [&cp1, &cp2] {
            assert!(!s.files(cp).iter().any(|f| f.starts_with("sub")), "{cp}");
        }
    }

    #[test]
    fn a_submodule_stays_where_the_branch_has_it() {
        let s = Setup::new();
        let lib = s.private.join("lib");
        std::fs::create_dir(&lib).unwrap();
        let id = ["-c", "user.name=A", "-c", "user.email=a@x"];
        git(&lib, &["init", "--quiet"]);
        std::fs::write(lib.join("f"), "x\n").unwrap();
        git(&lib, &[&id[..], &["add", "f"]].concat());
        git(&lib, &[&id[..], &["commit", "--quiet", "-m", "f"]].concat());
        let lib_url = lib.display().to_string();
        git(
            &s.wt,
            &[
                "-c",
                "protocol.file.allow=always",
                "submodule",
                "add",
                "--quiet",
                &lib_url,
                "lib",
            ],
        );
        git(
            &s.wt,
            &[&id[..], &["commit", "--quiet", "-m", "lib"]].concat(),
        );
        let pinned = git(&s.repo, &["rev-parse", "feat/x:lib"]).trim().to_owned();
        // The agent moves the submodule, plants a filter in its config, and dirties a file.
        let mark = s.private.join("filter-ran");
        let sub = s.wt.join("lib");
        std::fs::write(sub.join("g"), "g\n").unwrap();
        git(&sub, &[&id[..], &["add", "g"]].concat());
        git(&sub, &[&id[..], &["commit", "--quiet", "-m", "g"]].concat());
        git(
            &sub,
            &[
                "config",
                "filter.x.clean",
                &format!("touch {}; cat", mark.display()),
            ],
        );
        std::fs::write(sub.join(".gitattributes"), "* filter=x\n").unwrap();
        let cp1 = s.take(1, None).unwrap();
        std::fs::write(sub.join("f"), "y\n").unwrap();
        s.write("a.txt", "changed\n");
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        s.rewind(&cp2, &cp1).unwrap();
        let index = commit::index_path(&s.private);
        let staged = commit::stage(&s.dirs, &s.wt, "feat/x", None, &index);
        assert!(!mark.exists(), "the submodule's filter ran");
        for cp in [&cp1, &cp2] {
            assert_eq!(
                git(&s.repo, &["rev-parse", &format!("{cp}:lib")]).trim(),
                pinned
            );
        }
        // Nothing else changed, so there's nothing for git_commit to commit.
        assert!(
            matches!(staged, Err(CommitError::NothingToCommit)),
            "{staged:?}"
        );
    }

    #[test]
    fn a_nested_repo_the_branch_already_holds_never_makes_the_daemon_run_its_config() {
        let s = Setup::new();
        let id = ["-c", "user.name=A", "-c", "user.email=a@x"];
        let sub = s.wt.join("sub");
        std::fs::create_dir(&sub).unwrap();
        git(&sub, &["init", "--quiet"]);
        std::fs::write(sub.join("f"), "x\n").unwrap();
        git(&sub, &[&id[..], &["add", "f"]].concat());
        git(&sub, &[&id[..], &["commit", "--quiet", "-m", "f"]].concat());
        // Committed on the branch as an embedded repo, a gitlink without .gitmodules.
        git(&s.wt, &[&id[..], &["add", "sub"]].concat());
        git(
            &s.wt,
            &[&id[..], &["commit", "--quiet", "-m", "sub"]].concat(),
        );
        let mark = s.private.join("filter-ran");
        git(
            &sub,
            &[
                "config",
                "filter.x.clean",
                &format!("touch {}; cat", mark.display()),
            ],
        );
        std::fs::write(sub.join(".gitattributes"), "* filter=x\n").unwrap();
        let cp1 = s.take(1, None).unwrap();
        std::fs::write(sub.join("f"), "y\n").unwrap();
        s.write("a.txt", "changed\n");
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        s.write("a.txt", "again\n");
        let index = commit::index_path(&s.private);
        let staged = commit::stage(&s.dirs, &s.wt, "feat/x", None, &index).unwrap();
        commit::commit(&s.dirs, &s.wt, "feat/x", &staged, "m", &index).unwrap();
        std::fs::write(sub.join("f"), "z\n").unwrap();
        s.write("a.txt", "once more\n");
        commit::stage(&s.dirs, &s.wt, "feat/x", None, &index).unwrap();
        assert!(!mark.exists(), "the nested repo's filter ran");
        let pinned = git(&s.repo, &["rev-parse", &format!("{cp1}:sub")]);
        assert_eq!(git(&s.repo, &["rev-parse", &format!("{cp2}:sub")]), pinned);
    }

    #[test]
    fn a_git_file_pointing_elsewhere_never_reaches_a_checkpoint() {
        let s = Setup::new();
        s.write(
            "gf/.git",
            &format!("gitdir: {}\n", s.repo.join(".git").display()),
        );
        s.write("gf/x.txt", "x\n");
        let cp = s.take(1, None).unwrap();
        assert_eq!(s.files(&cp), [".gitignore", "a.txt"]);
    }

    #[test]
    fn ignore_files_added_anyway_count_toward_the_size_cap() {
        let s = Setup::new();
        let mut text = String::from("*\n");
        let line = format!("# {}\n", "x".repeat(1021));
        while (text.len() as u64) <= commit::MAX_COMMIT_BYTES {
            text.push_str(&line);
        }
        s.write("big/.gitignore", &text);
        assert!(matches!(s.take(1, None), Err(CommitError::TooLarge)));
        let objects = git(&s.repo, &["count-objects", "-v"]);
        let size: u64 = objects
            .lines()
            .find_map(|l| l.strip_prefix("size: "))
            .unwrap()
            .parse()
            .unwrap();
        assert!(size < 1024, "{objects}");
    }

    #[test]
    fn rewinding_restores_the_files_and_the_branch_and_keeps_ignored_files() {
        let s = Setup::new();
        let main_before = s.rev("main");
        s.write("a.txt", "turn one\n");
        let cp1 = s.take(1, None).unwrap();
        // Turn two: an edit, a new file, a deletion, a commit, and some build output.
        s.write("a.txt", "turn two\n");
        s.write("src/later.rs", "later\n");
        s.write("target/out.bin", "build\n");
        let staged = commit::stage(
            &s.dirs,
            &s.wt,
            "feat/x",
            None,
            &commit::index_path(&s.private),
        )
        .unwrap();
        commit::commit(
            &s.dirs,
            &s.wt,
            "feat/x",
            &staged,
            "agent commit",
            &commit::index_path(&s.private),
        )
        .unwrap();
        s.write("b.txt", "uncommitted\n");
        let cp2 = s.take(2, Some(&cp1)).unwrap();

        s.rewind(&cp2, &cp1).unwrap();
        assert_eq!(
            std::fs::read_to_string(s.wt.join("a.txt")).unwrap(),
            "turn one\n"
        );
        assert!(!s.wt.join("src/later.rs").exists());
        assert!(!s.wt.join("b.txt").exists());
        assert_eq!(
            std::fs::read_to_string(s.wt.join("target/out.bin")).unwrap(),
            "build\n"
        );
        // The branch is back where it was at checkpoint 1; its own status matches.
        assert_eq!(s.rev("feat/x"), s.rev(&format!("{cp1}^")));
        assert_eq!(git(&s.wt, &["status", "--porcelain"]), " M a.txt\n");
        // The main checkout and the base branch never moved.
        assert_eq!(s.rev("main"), main_before);
        assert_eq!(
            std::fs::read_to_string(s.repo.join("a.txt")).unwrap(),
            "a\n"
        );
        // The next checkpoint starts from the rewound state.
        s.write("c.txt", "c\n");
        let cp3 = s.take(3, Some(&cp1)).unwrap();
        assert_eq!(s.files(&cp3), [".gitignore", "a.txt", "c.txt"]);
    }

    #[test]
    fn rewinding_refuses_when_the_branch_moved_though_both_checkpoints_share_a_tip() {
        let s = Setup::new();
        s.write("a.txt", "one\n");
        let cp1 = s.take(1, None).unwrap();
        s.write("a.txt", "two\n");
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        // The worktree still matches checkpoint 2, but the branch got a commit since.
        let tree = s.rev(&format!("{cp2}^{{tree}}"));
        let moved = git(
            &s.repo,
            &["commit-tree", &tree, "-p", &s.rev("feat/x"), "-m", "late"],
        );
        git(&s.repo, &["update-ref", "refs/heads/feat/x", moved.trim()]);
        assert!(matches!(s.rewind(&cp2, &cp1), Err(CommitError::Moved(_))));
        assert_eq!(s.rev("feat/x"), moved.trim());
        assert_eq!(
            std::fs::read_to_string(s.wt.join("a.txt")).unwrap(),
            "two\n"
        );
    }

    #[test]
    fn rewinding_refuses_when_the_worktree_or_branch_changed_since_the_snapshot() {
        let s = Setup::new();
        s.write("a.txt", "one\n");
        let cp1 = s.take(1, None).unwrap();
        s.write("a.txt", "two\n");
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        // A process the agent left running edits the file after the snapshot.
        s.write("a.txt", "three, a bit longer\n");
        assert!(s.rewind(&cp2, &cp1).is_err());
        assert_eq!(
            std::fs::read_to_string(s.wt.join("a.txt")).unwrap(),
            "three, a bit longer\n"
        );

        // So does a file both checkpoints agree on, or a new one: a rewind keeps nothing it
        // didn't snapshot.
        s.write("a.txt", "two\n");
        s.write(".gitignore", "target/\n# edited\n");
        assert!(matches!(
            s.rewind(&cp2, &cp1),
            Err(CommitError::WorktreeChanged)
        ));
        s.write(".gitignore", "target/\n");
        s.write("new.txt", "new\n");
        assert!(matches!(
            s.rewind(&cp2, &cp1),
            Err(CommitError::WorktreeChanged)
        ));
        std::fs::remove_file(s.wt.join("new.txt")).unwrap();

        // A branch that moved after the snapshot isn't moved back either.
        let tip = s.rev("feat/x");
        let moved = git(
            &s.repo,
            &[
                "commit-tree",
                &s.rev(&format!("{tip}^{{tree}}")),
                "-p",
                &tip,
                "-m",
                "moved",
            ],
        );
        git(&s.repo, &["update-ref", "refs/heads/feat/x", moved.trim()]);
        let cp3 = s.take(3, Some(&cp2)).unwrap();
        let other = git(
            &s.repo,
            &[
                "commit-tree",
                &s.rev(&format!("{tip}^{{tree}}")),
                "-p",
                moved.trim(),
                "-m",
                "again",
            ],
        );
        git(&s.repo, &["update-ref", "refs/heads/feat/x", other.trim()]);
        assert!(matches!(s.rewind(&cp3, &cp1), Err(CommitError::Moved(_))));
        assert_eq!(s.rev("feat/x"), other.trim());
        assert_eq!(
            std::fs::read_to_string(s.wt.join("a.txt")).unwrap(),
            "two\n"
        );
    }

    #[test]
    fn rewinding_never_writes_through_a_symlink_the_agent_planted() {
        let s = Setup::new();
        s.write("d/f.txt", "inside\n");
        let cp1 = s.take(1, None).unwrap();
        // The agent swaps the directory for a link to somewhere outside the worktree.
        let outside = s.private.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::remove_dir_all(s.wt.join("d")).unwrap();
        std::os::unix::fs::symlink(&outside, s.wt.join("d")).unwrap();
        let cp2 = s.take(2, Some(&cp1)).unwrap();
        s.rewind(&cp2, &cp1).unwrap();
        assert!(
            std::fs::read_dir(&outside).unwrap().next().is_none(),
            "wrote outside the worktree"
        );
        assert!(!s.wt.join("d").symlink_metadata().unwrap().is_symlink());
        assert_eq!(
            std::fs::read_to_string(s.wt.join("d/f.txt")).unwrap(),
            "inside\n"
        );
    }
}
