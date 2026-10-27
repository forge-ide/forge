//! What review reads from git: which files changed between two commits (renames found), the
//! same for each step along a run of checkpoints, merge-bases, and blobs. Only the repo's
//! objects are read, through its shared git dir as [`crate::commit`] runs git, never the
//! worktree; the diffs themselves are made by `forge-review`.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::commit;
use crate::worktree::{GitDirs, GitError};

/// The largest blob [`blobs`] reads; anything larger is reported by its size alone.
pub const MAX_BLOB: u64 = 16 * 1024 * 1024;

/// How many bytes of blobs one run of git reads, under its output cap.
const BATCH_BYTES: u64 = 24 * 1024 * 1024;

/// How `diff-tree` finds changes: recursively, with renames, NUL-separated.
const RAW: [&str; 4] = ["-r", "-z", "-M", "--raw"];

/// A gitlink's mode: a submodule or nested repo, pinned at a commit of its own.
pub const GITLINK: u32 = 0o160000;

/// How a file changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// One side of a change: where the file was, its mode, and its object. The path is git's
/// bytes: names needn't be UTF-8, and two that differ only in bytes that aren't must stay
/// apart (see [`quote_path`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Side {
    pub path: Vec<u8>,
    pub mode: u32,
    pub id: String,
}

/// A changed file. `old` is absent for an added file, `new` for a deleted one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    pub old: Option<Side>,
    pub new: Option<Side>,
}

impl Change {
    /// Where the file is after the change, or was, if it was deleted.
    pub fn path(&self) -> &[u8] {
        match (&self.new, &self.old) {
            (Some(side), _) | (None, Some(side)) => &side.path,
            (None, None) => b"",
        }
    }
}

/// A blob, or its size when it's over [`MAX_BLOB`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blob {
    Bytes(Vec<u8>),
    TooLarge(u64),
}

/// The best common ancestor of commits `a` and `b`.
pub fn merge_base(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    a: &str,
    b: &str,
) -> Result<String, GitError> {
    let run = |args: &[&str]| commit::run(dirs, worktree, index, args, None);
    Ok(run(&["merge-base", a, b])?.trim().to_owned())
}

/// The files that differ between commits `from` and `to`, in path order.
pub fn changes(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    from: &str,
    to: &str,
) -> Result<Vec<Change>, GitError> {
    let mut args = vec!["diff-tree"];
    args.extend(RAW);
    args.extend([from, to]);
    let out = commit::run_bytes(dirs, worktree, index, &args, None)?;
    let mut tokens = out.split(|b| *b == 0);
    let mut changes = Vec::new();
    while let Some(token) = tokens.next() {
        if let Some(raw) = token.strip_prefix(b":") {
            changes.push(entry(raw, &mut tokens)?);
        }
    }
    Ok(changes)
}

/// The changes of each step along `commits`: one list per consecutive pair, in one run of
/// git.
pub fn steps(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    commits: &[&str],
) -> Result<Vec<Vec<Change>>, GitError> {
    let pairs: Vec<(&str, &str)> = commits.windows(2).map(|w| (w[0], w[1])).collect();
    let mut steps = vec![Vec::new(); pairs.len()];
    // Each line names a commit, then the one to diff it against. A pair that's the same
    // commit is left out; one whose trees match prints nothing, not even its header.
    let input: String = pairs
        .iter()
        .filter(|(from, to)| from != to)
        .map(|(from, to)| format!("{to} {from}\n"))
        .collect();
    if input.is_empty() {
        return Ok(steps);
    }
    let mut args = vec!["diff-tree", "--stdin"];
    args.extend(RAW);
    let out = commit::run_bytes(dirs, worktree, index, &args, Some(input.as_bytes()))?;
    let mut tokens = out.split(|b| *b == 0);
    // The pair the entries being read belong to; headers come in the order of the input. A
    // pair is matched by its target, searching on from the last one matched: the pairs
    // skipped are ones that printed nothing, whose trees are equal, so a later pair with the
    // same target as one of them, with only such pairs between, would have equal trees too and
    // print nothing either.
    let mut at: Option<usize> = None;
    while let Some(token) = tokens.next() {
        if let Some(raw) = token.strip_prefix(b":") {
            let step = at.ok_or_else(|| malformed("an entry before any commit"))?;
            steps[step].push(entry(raw, &mut tokens)?);
            continue;
        }
        let header = std::str::from_utf8(token)
            .map_err(|_| malformed("a commit header that isn't text"))?
            .trim();
        if header.is_empty() {
            continue;
        }
        let from = at.map_or(0, |i| i + 1);
        at = Some(
            (from..pairs.len())
                .find(|&i| pairs[i].0 != pairs[i].1 && pairs[i].1 == header)
                .ok_or_else(|| malformed("a commit it wasn't asked about"))?,
        );
    }
    Ok(steps)
}

/// A raw entry, `raw` being its fields after the `:`, and the paths that follow it.
fn entry<'a>(raw: &[u8], paths: &mut impl Iterator<Item = &'a [u8]>) -> Result<Change, GitError> {
    let raw = std::str::from_utf8(raw).map_err(|_| malformed("a raw entry that isn't text"))?;
    let fields: Vec<&str> = raw.split(' ').collect();
    let [old_mode, new_mode, old_id, new_id, status] = fields[..] else {
        return Err(malformed("a raw entry without five fields"));
    };
    let mode =
        |m: &str| u32::from_str_radix(m, 8).map_err(|_| malformed("a mode that isn't octal"));
    let mut path = || {
        paths
            .next()
            .map(<[u8]>::to_vec)
            .ok_or_else(|| malformed("an entry without its path"))
    };
    let side = |path: Vec<u8>, mode: u32, id: &str| Side {
        path,
        mode,
        id: id.to_owned(),
    };
    let (old_mode, new_mode) = (mode(old_mode)?, mode(new_mode)?);
    Ok(match status.chars().next() {
        Some('A') => Change {
            kind: ChangeKind::Added,
            old: None,
            new: Some(side(path()?, new_mode, new_id)),
        },
        Some('D') => Change {
            kind: ChangeKind::Deleted,
            old: Some(side(path()?, old_mode, old_id)),
            new: None,
        },
        Some('R') => {
            let (from, to) = (path()?, path()?);
            Change {
                kind: ChangeKind::Renamed,
                old: Some(side(from, old_mode, old_id)),
                new: Some(side(to, new_mode, new_id)),
            }
        }
        // Modified, or its type changed (a file became a link, say).
        Some('M' | 'T') => {
            let path = path()?;
            Change {
                kind: ChangeKind::Modified,
                old: Some(side(path.clone(), old_mode, old_id)),
                new: Some(side(path, new_mode, new_id)),
            }
        }
        _ => return Err(malformed("an entry of a status it doesn't expect")),
    })
}

fn malformed(what: &str) -> GitError {
    GitError::Command {
        args: "diff-tree".into(),
        stderr: format!("unexpected output: {what}"),
    }
}

/// `path` as a string a client can show and send back: as it is when it's UTF-8 without
/// control characters, `"`, or `\\`; otherwise quoted as git prints it with `core.quotePath`
/// (`"a\\376.sh"`). A plain path never starts with `"`, so [`unquote_path`] undoes it, and no
/// two paths give the same string.
pub fn quote_path(path: &[u8]) -> String {
    let plain = |b: &u8| *b >= 0x20 && *b != 0x7f && *b != b'"' && *b != b'\\';
    if let Ok(text) = std::str::from_utf8(path)
        && path.iter().all(plain)
    {
        return text.to_owned();
    }
    let mut out = String::from("\"");
    for &b in path {
        match b {
            b'\x07' => out.push_str("\\a"),
            b'\x08' => out.push_str("\\b"),
            b'\t' => out.push_str("\\t"),
            b'\n' => out.push_str("\\n"),
            b'\x0b' => out.push_str("\\v"),
            b'\x0c' => out.push_str("\\f"),
            b'\r' => out.push_str("\\r"),
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b if !(0x20..0x7f).contains(&b) => out.push_str(&format!("\\{b:03o}")),
            b => out.push(char::from(b)),
        }
    }
    out.push('"');
    out
}

/// The path [`quote_path`] made `quoted` from; `None` when `quoted` isn't exactly what it
/// makes, so each path has one spelling.
pub fn unquote_path(quoted: &str) -> Option<Vec<u8>> {
    let path = unescape(quoted)?;
    (quote_path(&path) == quoted).then_some(path)
}

fn unescape(quoted: &str) -> Option<Vec<u8>> {
    let Some(inner) = quoted.strip_prefix('"') else {
        return Some(quoted.as_bytes().to_vec());
    };
    let inner = inner.strip_suffix('"')?.as_bytes();
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while i < inner.len() {
        let b = inner[i];
        i += 1;
        if b != b'\\' {
            out.push(b);
            continue;
        }
        let escaped = *inner.get(i)?;
        i += 1;
        out.push(match escaped {
            b'a' => 0x07,
            b'b' => 0x08,
            b't' => b'\t',
            b'n' => b'\n',
            b'v' => 0x0b,
            b'f' => 0x0c,
            b'r' => b'\r',
            b'"' => b'"',
            b'\\' => b'\\',
            b'0'..=b'3' => {
                let digits = inner.get(i - 1..i + 2)?;
                i += 2;
                u8::from_str_radix(std::str::from_utf8(digits).ok()?, 8).ok()?
            }
            _ => return None,
        });
    }
    Some(out)
}

/// The size of each blob `ids` names, by ID, without reading them.
pub fn sizes(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    ids: &[&str],
) -> Result<HashMap<String, u64>, GitError> {
    let mut sizes = HashMap::new();
    if ids.is_empty() {
        return Ok(sizes);
    }
    let input: String = ids.iter().map(|id| format!("{id}\n")).collect();
    let check = commit::run(
        dirs,
        worktree,
        index,
        &["cat-file", "--batch-check"],
        Some(input.as_bytes()),
    )?;
    for line in check.lines() {
        let fields: Vec<&str> = line.split(' ').collect();
        let [id, "blob", size] = fields[..] else {
            return Err(GitError::Command {
                args: "cat-file --batch-check".into(),
                stderr: format!("not a blob: {line}"),
            });
        };
        let size: u64 = size.parse().map_err(|_| GitError::Command {
            args: "cat-file --batch-check".into(),
            stderr: format!("unexpected output: {line}"),
        })?;
        sizes.insert(id.to_owned(), size);
    }
    Ok(sizes)
}

/// The blobs `ids` names, by ID.
pub fn blobs(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    ids: &[&str],
) -> Result<HashMap<String, Blob>, GitError> {
    let mut blobs = HashMap::new();
    if ids.is_empty() {
        return Ok(blobs);
    }
    let sizes = sizes(dirs, worktree, index, ids)?;
    let mut wanted: Vec<(String, u64)> = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            continue;
        }
        let size = *sizes.get(*id).ok_or_else(|| GitError::Command {
            args: "cat-file --batch-check".into(),
            stderr: format!("no size for {id}"),
        })?;
        if size > MAX_BLOB {
            blobs.insert((*id).to_owned(), Blob::TooLarge(size));
        } else {
            wanted.push(((*id).to_owned(), size));
        }
    }
    let mut batch: Vec<&(String, u64)> = Vec::new();
    let mut bytes = 0;
    for item in &wanted {
        if !batch.is_empty() && bytes + item.1 > BATCH_BYTES {
            read_batch(dirs, worktree, index, &batch, &mut blobs)?;
            (batch, bytes) = (Vec::new(), 0);
        }
        batch.push(item);
        bytes += item.1;
    }
    if !batch.is_empty() {
        read_batch(dirs, worktree, index, &batch, &mut blobs)?;
    }
    Ok(blobs)
}

/// Read `batch`'s blobs, whose sizes are known, in one run of git.
fn read_batch(
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
    batch: &[&(String, u64)],
    blobs: &mut HashMap<String, Blob>,
) -> Result<(), GitError> {
    let input: String = batch.iter().map(|(id, _)| format!("{id}\n")).collect();
    let out = commit::run_bytes(
        dirs,
        worktree,
        index,
        &["cat-file", "--batch"],
        Some(input.as_bytes()),
    )?;
    let bad = || GitError::Command {
        args: "cat-file --batch".into(),
        stderr: "unexpected output".into(),
    };
    let mut rest = &out[..];
    for (id, size) in batch {
        // `<id> blob <size>\n<contents>\n`
        let header = rest.iter().position(|b| *b == b'\n').ok_or_else(bad)?;
        let expected = format!("{id} blob {size}");
        if &rest[..header] != expected.as_bytes() {
            return Err(bad());
        }
        let start = header + 1;
        let end = start + *size as usize;
        let contents = rest.get(start..end).ok_or_else(bad)?;
        blobs.insert(id.clone(), Blob::Bytes(contents.to_vec()));
        rest = rest.get(end + 1..).ok_or_else(bad)?;
    }
    Ok(())
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
            .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
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
        index: PathBuf,
    }

    impl Setup {
        /// A repo on `main` holding `a.txt`, `b.txt`, and `c.txt`, with a worktree on
        /// `feat/x`.
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let repo = root.join("repo");
            std::fs::create_dir(&repo).unwrap();
            git(&repo, &["init", "--quiet", "--initial-branch=main"]);
            std::fs::write(repo.join("a.txt"), "a\n").unwrap();
            let long: String = (0..50).map(|n| format!("line {n}\n")).collect();
            std::fs::write(repo.join("b.txt"), long).unwrap();
            std::fs::write(repo.join("c.txt"), "c\n").unwrap();
            git(&repo, &["add", "."]);
            git(&repo, &["commit", "--quiet", "-m", "initial"]);
            let wt = GitCli
                .create(&repo, "feat/x", "main", CreateMode::New)
                .unwrap();
            let mut dirs = GitCli.git_dirs(&repo, &wt).unwrap();
            dirs.launch = crate::Launch::Unconfined;
            let index = root.join("private/git/review.index");
            Setup {
                _dir: dir,
                repo,
                wt,
                dirs,
                index,
            }
        }

        /// Commit everything in the worktree on `feat/x`.
        fn commit(&self, message: &str) -> String {
            git(&self.wt, &["add", "--all"]);
            git(
                &self.wt,
                &["commit", "--quiet", "--allow-empty", "-m", message],
            );
            git(&self.wt, &["rev-parse", "HEAD"]).trim().to_owned()
        }

        fn rev(&self, rev: &str) -> String {
            git(&self.repo, &["rev-parse", rev]).trim().to_owned()
        }
    }

    fn summary(changes: &[Change]) -> Vec<(ChangeKind, Option<String>, String)> {
        changes
            .iter()
            .map(|c| {
                (
                    c.kind,
                    c.old.as_ref().map(|s| quote_path(&s.path)),
                    quote_path(c.path()),
                )
            })
            .collect()
    }

    #[test]
    fn paths_are_quoted_only_when_they_must_be_and_unquote_back() {
        for (path, quoted) in [
            (&b"src/main.rs"[..], "src/main.rs"),
            (b"caf\xc3\xa9.txt", "caf\u{e9}.txt"),
            (b"a\xfe.sh", "\"a\\376.sh\""),
            (b"a\xff.sh", "\"a\\377.sh\""),
            (b"tab\there", "\"tab\\there\""),
            (b"new\nline", "\"new\\nline\""),
            (b"\"x", "\"\\\"x\""),
            (b"back\\slash", "\"back\\\\slash\""),
            (b"del\x7f", "\"del\\177\""),
        ] {
            assert_eq!(quote_path(path), quoted, "{path:?}");
            assert_eq!(unquote_path(quoted).as_deref(), Some(path), "{quoted}");
        }
        // Only the spelling quote_path gives: not a needless quote, a needless escape, or a
        // control character left plain.
        for bad in [
            "\"unterminated",
            "\"\\q\"",
            "\"\\4\"",
            "\"\\9zz\"",
            "\"plain.rs\"",
            "\"\\141\\376\"",
            "tab\there",
        ] {
            assert_eq!(unquote_path(bad), None, "{bad}");
        }
    }

    #[test]
    fn names_that_differ_only_in_bytes_that_arent_utf8_stay_apart() {
        use std::os::unix::ffi::OsStrExt;
        let s = Setup::new();
        let base = s.rev("main");
        // Staged straight into the index, so no filesystem has to accept the names.
        for (name, text) in [(&b"a\xfe.sh"[..], "harmless\n"), (b"a\xff.sh", "evil\n")] {
            std::fs::write(s.wt.join("blob"), text).unwrap();
            let id = git(&s.wt, &["hash-object", "-w", "blob"]);
            std::fs::remove_file(s.wt.join("blob")).unwrap();
            let mut info = format!("100644,{},", id.trim()).into_bytes();
            info.extend_from_slice(name);
            let status = Command::new("git")
                .arg("-C")
                .arg(&s.wt)
                .args(["update-index", "--add", "--cacheinfo"])
                .arg(std::ffi::OsStr::from_bytes(&info))
                .status()
                .unwrap();
            assert!(status.success());
        }
        git(&s.wt, &["commit", "--quiet", "-m", "names"]);
        let head = s.rev("feat/x");
        let changes = changes(&s.dirs, &s.wt, &s.index, &base, &head).unwrap();
        let paths: Vec<&[u8]> = changes.iter().map(|c| c.path()).collect();
        assert_eq!(paths, [&b"a\xfe.sh"[..], b"a\xff.sh"]);
        assert_ne!(changes[0].new, changes[1].new);
    }

    #[test]
    fn changes_name_added_modified_deleted_and_renamed_files() {
        let s = Setup::new();
        let base = s.rev("main");
        std::fs::write(s.wt.join("a.txt"), "changed\n").unwrap();
        std::fs::rename(s.wt.join("b.txt"), s.wt.join("d.txt")).unwrap();
        std::fs::remove_file(s.wt.join("c.txt")).unwrap();
        std::fs::write(s.wt.join("e.sh"), "#!/bin/sh\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(s.wt.join("e.sh"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        let head = s.commit("one");
        let changes = changes(&s.dirs, &s.wt, &s.index, &base, &head).unwrap();
        assert_eq!(
            summary(&changes),
            [
                (ChangeKind::Modified, Some("a.txt".into()), "a.txt".into()),
                (ChangeKind::Deleted, Some("c.txt".into()), "c.txt".into()),
                (ChangeKind::Renamed, Some("b.txt".into()), "d.txt".into()),
                (ChangeKind::Added, None, "e.sh".into()),
            ]
        );
        let added = changes[3].new.as_ref().unwrap();
        assert_eq!(added.mode, 0o100755);
        assert_eq!(added.id, s.rev(&format!("{head}:e.sh")));
        assert_eq!(
            changes[0].old.as_ref().unwrap().id,
            s.rev(&format!("{base}:a.txt"))
        );
        assert!(changes[1].new.is_none());
    }

    #[test]
    fn steps_list_each_pairs_changes_and_nothing_for_an_unchanged_pair() {
        let s = Setup::new();
        let c0 = s.rev("main");
        std::fs::write(s.wt.join("a.txt"), "one\n").unwrap();
        let c1 = s.commit("one");
        std::fs::rename(s.wt.join("a.txt"), s.wt.join("z.txt")).unwrap();
        let c2 = s.commit("two");
        // Another commit with the same files, then one that changes them again.
        let c3 = s.commit("three");
        std::fs::write(s.wt.join("z.txt"), "four\n").unwrap();
        let c4 = s.commit("four");
        let steps = steps(&s.dirs, &s.wt, &s.index, &[&c0, &c1, &c1, &c2, &c3, &c4]).unwrap();
        assert_eq!(steps.len(), 5);
        assert_eq!(
            summary(&steps[0]),
            [(ChangeKind::Modified, Some("a.txt".into()), "a.txt".into())]
        );
        assert!(steps[1].is_empty());
        assert_eq!(
            summary(&steps[2]),
            [(ChangeKind::Renamed, Some("a.txt".into()), "z.txt".into())]
        );
        assert!(steps[3].is_empty());
        assert_eq!(
            summary(&steps[4]),
            [(ChangeKind::Modified, Some("z.txt".into()), "z.txt".into())]
        );
        assert!(
            super::steps(&s.dirs, &s.wt, &s.index, &[&c0])
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn the_merge_base_is_where_the_branch_left_its_base() {
        let s = Setup::new();
        let start = s.rev("main");
        std::fs::write(s.wt.join("a.txt"), "branch\n").unwrap();
        let head = s.commit("branch");
        std::fs::write(s.repo.join("c.txt"), "main moved on\n").unwrap();
        git(&s.repo, &["commit", "--quiet", "-am", "main"]);
        assert_ne!(s.rev("main"), start);
        let base = merge_base(&s.dirs, &s.wt, &s.index, "refs/heads/main", &head).unwrap();
        assert_eq!(base, start);
    }

    #[test]
    fn blobs_come_back_byte_for_byte_and_large_ones_by_size() {
        let s = Setup::new();
        let binary = b"\x89PNG\r\n\x1a\n\0\xff\xfe".to_vec();
        std::fs::write(s.wt.join("img.png"), &binary).unwrap();
        // Together over git's output cap, so they take more than one read.
        let big = |byte: u8| vec![byte; (MAX_BLOB - 1024) as usize];
        std::fs::write(s.wt.join("big1"), big(b'1')).unwrap();
        std::fs::write(s.wt.join("big2"), big(b'2')).unwrap();
        std::fs::write(s.wt.join("big3"), big(b'3')).unwrap();
        let huge = std::fs::File::create(s.wt.join("huge")).unwrap();
        huge.set_len(MAX_BLOB + 1).unwrap();
        let head = s.commit("files");
        let id = |path: &str| s.rev(&format!("{head}:{path}"));
        let ids = ["img.png", "big1", "big2", "big3", "huge"].map(id);
        let refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let blobs = blobs(&s.dirs, &s.wt, &s.index, &refs).unwrap();
        assert_eq!(blobs[&ids[0]], Blob::Bytes(binary));
        assert_eq!(blobs[&ids[1]], Blob::Bytes(big(b'1')));
        assert_eq!(blobs[&ids[3]], Blob::Bytes(big(b'3')));
        assert_eq!(blobs[&ids[4]], Blob::TooLarge(MAX_BLOB + 1));
        assert!(
            super::blobs(&s.dirs, &s.wt, &s.index, &[])
                .unwrap()
                .is_empty()
        );
    }
}
