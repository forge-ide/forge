//! `review.changeset` and `review.fileDiff`: what a session changed, from its base branch's
//! merge-base (or a checkpoint) to its latest checkpoint, and which turn each hunk came from.
//! Only checkpoint commits are read, never the worktree, so what's reviewed is exactly what
//! the checkpoints hold. Their contents are the agent's, so none of it is logged.

use std::borrow::Cow;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use forge_api::model::{Changeset, FileChange, FileStatus, Hunk};
use forge_api::{ApiError, CheckpointId, ErrorKind, SessionId};
use forge_git::GitError;
use forge_git::review::{self as git, Blob, Change, ChangeKind, GITLINK, Side};
use forge_review::{
    DiffHunk, counts, diff_lines, display, history, hunk_id, is_binary, line_count, split,
};

use super::checkpoints::RepoTarget;
use super::{SessionError, SessionManager};
use crate::store::CheckpointRecord;

/// How long one call's diffs look for the smallest set of changes before settling for a
/// larger one (still exact), so one pathological file can't hold the call up.
const DIFF_TIME: Duration = Duration::from_secs(5);

/// The checkpoints a review spans: from the merge-base, or checkpoint `from`, to `head`.
#[derive(Clone)]
pub(super) struct Span {
    pub(super) head: CheckpointRecord,
    from: Option<CheckpointRecord>,
    /// The live checkpoints after the start, in order, ending with `head`.
    steps: Vec<CheckpointRecord>,
}

impl SessionManager {
    /// What the session changed, from each repo's merge-base with its base branch (`bases`:
    /// repo name, base branch), or from checkpoint `from`, to its latest checkpoint.
    pub async fn changeset(
        &self,
        id: &SessionId,
        from: Option<&CheckpointId>,
        bases: Vec<(String, String)>,
    ) -> Result<Changeset, SessionError> {
        let span = self.span(id, from).await?;
        let targets = self.review_targets(id).await?;
        let head = span.head.id.clone();
        let (base, files) = tokio::task::spawn_blocking(move || {
            let deadline = Instant::now() + DIFF_TIME;
            let mut base = String::new();
            let mut files = Vec::new();
            for t in &targets {
                let start = start_of(t, &span, &bases)?;
                if base.is_empty() {
                    base = start.clone();
                }
                files.extend(changed_files(
                    t,
                    &start,
                    &commit_of(&span.head, &t.repo)?,
                    deadline,
                )?);
            }
            Ok::<_, ApiError>((base, files))
        })
        .await
        .map_err(|e| SessionError::Internal(e.to_string()))??;
        let key = id.clone();
        let rounds = self.blocking(move |store| store.rounds(&key)).await?;
        let round = rounds.len() as u32 + 1;
        let key = id.clone();
        let viewed = self
            .blocking(move |store| store.viewed(&key, round))
            .await?;
        let mut files = files;
        for f in &mut files {
            f.viewed = viewed
                .iter()
                .any(|(repo, path)| Some(repo) == f.repo.as_ref() && path == &f.path);
        }
        Ok(Changeset {
            session_id: id.clone(),
            base,
            head: head.to_string(),
            files,
            round,
            last_reviewed: rounds.last().map(|r| r.checkpoint.clone()),
        })
    }

    /// The hunks of `path` in [`Self::changeset`]'s diff, each with the turn that last changed
    /// one of its lines.
    pub async fn file_diff(
        &self,
        id: &SessionId,
        repo: Option<&str>,
        path: &str,
        from: Option<&CheckpointId>,
        bases: Vec<(String, String)>,
    ) -> Result<Vec<Hunk>, SessionError> {
        let span = self.span(id, from).await?;
        let target = pick(self.review_targets(id).await?, repo, id)?;
        let path = path.to_owned();
        tokio::task::spawn_blocking(move || {
            let start = start_of(&target, &span, &bases)?;
            file_hunks(&target, &span, &start, &path)
        })
        .await
        .map_err(|e| SessionError::Internal(e.to_string()))?
        .map_err(SessionError::from)
    }

    pub(super) async fn span(
        &self,
        id: &SessionId,
        from: Option<&CheckpointId>,
    ) -> Result<Span, SessionError> {
        let key = id.clone();
        let all = self.blocking(move |store| store.checkpoints(&key)).await?;
        let live: Vec<CheckpointRecord> = all
            .iter()
            .filter(|c| c.discarded_by.is_none())
            .cloned()
            .collect();
        let Some(head) = live.last().cloned() else {
            return Err(ApiError::new(
                ErrorKind::InvalidState,
                format!("session {id} has no checkpoint yet; one is taken when a turn ends"),
            )
            .into());
        };
        let Some(from) = from else {
            return Ok(Span {
                head,
                from: None,
                steps: live,
            });
        };
        let Some(start) = all.iter().find(|c| &c.id == from).cloned() else {
            return Err(ApiError::not_found(format!("checkpoint {from} of session {id}")).into());
        };
        if start.discarded_by.is_some() {
            return Err(ApiError::new(
                ErrorKind::InvalidState,
                format!("checkpoint {from} was discarded by an earlier rewind"),
            )
            .into());
        }
        let steps = live.into_iter().filter(|c| c.n > start.n).collect();
        Ok(Span {
            head,
            from: Some(start),
            steps,
        })
    }

    pub(super) async fn review_targets(
        &self,
        id: &SessionId,
    ) -> Result<Vec<RepoTarget>, SessionError> {
        let session = self.get(id).await?;
        self.checkpoint_targets(&session)
            .await
            .map_err(|f| ApiError::new(f.kind, f.detail).into())
    }
}

/// The target for `repo`, which may be left out when the session has one.
pub(super) fn pick(
    targets: Vec<RepoTarget>,
    repo: Option<&str>,
    id: &SessionId,
) -> Result<RepoTarget, ApiError> {
    match repo {
        Some(name) => targets.into_iter().find(|t| t.repo == name),
        None if targets.len() == 1 => targets.into_iter().next(),
        None => {
            return Err(ApiError::validation(
                "the session spans more than one repo; say which one".to_owned(),
            ));
        }
    }
    .ok_or_else(|| ApiError::not_found(format!("repo {repo:?} of session {id}")))
}

pub(super) fn git_failed(e: GitError) -> ApiError {
    ApiError::new(ErrorKind::InvalidState, e.to_string())
}

/// `checkpoint`'s commit in `repo`.
pub(super) fn commit_of(checkpoint: &CheckpointRecord, repo: &str) -> Result<String, ApiError> {
    checkpoint
        .commits
        .iter()
        .find(|(r, _)| r == repo)
        .map(|(_, c)| c.clone())
        .ok_or_else(|| {
            ApiError::new(
                ErrorKind::InvalidState,
                format!("checkpoint {} has nothing for repo {repo}", checkpoint.id),
            )
        })
}

/// The commit `t`'s diff starts from: the `from` checkpoint's, or the merge-base of the base
/// branch and the head checkpoint.
pub(super) fn start_of(
    t: &RepoTarget,
    span: &Span,
    bases: &[(String, String)],
) -> Result<String, ApiError> {
    if let Some(from) = &span.from {
        return commit_of(from, &t.repo);
    }
    let base = bases
        .iter()
        .find(|(repo, _)| repo == &t.repo)
        .map(|(_, base)| base)
        .ok_or_else(|| {
            ApiError::new(
                ErrorKind::InvalidState,
                format!("repo {} is no longer in the project", t.repo),
            )
        })?;
    let head = commit_of(&span.head, &t.repo)?;
    git::merge_base(
        &t.dirs,
        &t.worktree,
        &t.index,
        &format!("refs/heads/{base}"),
        &head,
    )
    .map_err(git_failed)
}

/// The most blob bytes one call holds at once: a changeset reads its files in groups up to
/// this size, and a file's diff reads its versions at checkpoints in between only if they
/// fit.
const MAX_HELD: u64 = 256 * 1024 * 1024;

/// The most lines a side may have to be diffed; past it, as past 16 MiB, a file is too
/// large to show. Only generated or padded files get there, and a diff of millions of lines
/// would hold far more memory than its bytes.
pub(super) const MAX_LINES: usize = 1_000_000;

/// The most changed lines `review.fileDiff` returns for a file; the changeset still counts
/// them all.
pub(super) const MAX_DIFF_LINES: u32 = 200_000;

/// A file's contents as review reads them.
enum Body<'a> {
    Bytes(Cow<'a, [u8]>),
    /// Why it isn't read: its size, or its line count.
    TooLarge(String),
}

/// The contents of `side` (empty when the file isn't there). A gitlink reads as git shows
/// one: the commit it pins.
fn body<'a>(side: Option<&Side>, blobs: &'a HashMap<String, Blob>) -> Result<Body<'a>, ApiError> {
    let Some(side) = side else {
        return Ok(Body::Bytes(Cow::Borrowed(b"")));
    };
    if side.mode == GITLINK {
        return Ok(Body::Bytes(Cow::Owned(
            format!("Subproject commit {}\n", side.id).into_bytes(),
        )));
    }
    match blobs.get(&side.id) {
        Some(Blob::Bytes(bytes)) => Ok(match line_count(bytes) {
            lines if lines > MAX_LINES => Body::TooLarge(format!("{lines} lines")),
            _ => Body::Bytes(Cow::Borrowed(bytes)),
        }),
        Some(Blob::TooLarge(size)) => Ok(Body::TooLarge(format!("{size} bytes"))),
        None => Err(ApiError::new(
            ErrorKind::InvalidState,
            format!("git returned no blob for {}", git::quote_path(&side.path)),
        )),
    }
}

/// The blobs `sides` hold, each once; gitlinks have none here.
fn blob_ids<'a>(sides: impl Iterator<Item = &'a Side>) -> Vec<&'a str> {
    let mut ids: Vec<&str> = sides
        .filter(|s| s.mode != GITLINK)
        .map(|s| s.id.as_str())
        .collect();
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn read_blobs(t: &RepoTarget, ids: &[&str]) -> Result<HashMap<String, Blob>, ApiError> {
    git::blobs(&t.dirs, &t.worktree, &t.index, ids).map_err(git_failed)
}

/// What reading `id` would hold: nothing for a blob too large to be read.
fn held(sizes: &HashMap<String, u64>, id: &str) -> u64 {
    sizes
        .get(id)
        .copied()
        .filter(|size| *size <= git::MAX_BLOB)
        .unwrap_or(0)
}

fn mode(side: Option<&Side>) -> Option<String> {
    side.map(|s| format!("{:06o}", s.mode))
}

fn sides(c: &Change) -> impl Iterator<Item = &Side> {
    c.old.iter().chain(c.new.iter())
}

fn changed_files(
    t: &RepoTarget,
    start: &str,
    head: &str,
    deadline: Instant,
) -> Result<Vec<FileChange>, ApiError> {
    let changes = git::changes(&t.dirs, &t.worktree, &t.index, start, head).map_err(git_failed)?;
    let ids = blob_ids(changes.iter().flat_map(sides));
    let sizes = git::sizes(&t.dirs, &t.worktree, &t.index, &ids).map_err(git_failed)?;
    let mut files = Vec::with_capacity(changes.len());
    let mut group: Vec<&Change> = Vec::new();
    let mut bytes = 0;
    for c in &changes {
        let size: u64 = blob_ids(sides(c)).iter().map(|id| held(&sizes, id)).sum();
        if !group.is_empty() && bytes + size > MAX_HELD {
            files.extend(describe(t, &group, deadline)?);
            (group, bytes) = (Vec::new(), 0);
        }
        group.push(c);
        bytes += size;
    }
    if !group.is_empty() {
        files.extend(describe(t, &group, deadline)?);
    }
    Ok(files)
}

/// `group`'s files as the changeset lists them, from one read of their blobs.
fn describe(
    t: &RepoTarget,
    group: &[&Change],
    deadline: Instant,
) -> Result<Vec<FileChange>, ApiError> {
    let blobs = read_blobs(t, &blob_ids(group.iter().flat_map(|c| sides(c))))?;
    group
        .iter()
        .map(|c| {
            let (old, new) = (body(c.old.as_ref(), &blobs)?, body(c.new.as_ref(), &blobs)?);
            let (mut additions, mut deletions, mut binary) = (0, 0, false);
            // Too large to diff, there are no counts; `review.fileDiff` says why.
            let too_large = !matches!((&old, &new), (Body::Bytes(_), Body::Bytes(_)));
            if let (Body::Bytes(old), Body::Bytes(new)) = (&old, &new) {
                (additions, deletions) = counts(old, new, deadline);
                binary = is_binary(old) || is_binary(new);
            }
            Ok(FileChange {
                repo: Some(t.repo.clone()),
                path: git::quote_path(c.path()),
                status: status(c),
                old_path: match c.kind {
                    ChangeKind::Renamed => c.old.as_ref().map(|s| git::quote_path(&s.path)),
                    _ => None,
                },
                old_mode: mode(c.old.as_ref()),
                new_mode: mode(c.new.as_ref()),
                additions,
                deletions,
                binary,
                too_large,
                viewed: false,
            })
        })
        .collect()
}

fn status(c: &Change) -> FileStatus {
    match c.kind {
        ChangeKind::Added => FileStatus::Added,
        ChangeKind::Modified => FileStatus::Modified,
        ChangeKind::Deleted => FileStatus::Deleted,
        ChangeKind::Renamed => FileStatus::Renamed,
    }
}

fn file_hunks(t: &RepoTarget, span: &Span, start: &str, path: &str) -> Result<Vec<Hunk>, ApiError> {
    Ok(file_parts(t, span, start, path)?
        .hunks
        .into_iter()
        .map(|(hunk, _)| hunk)
        .collect())
}

/// A file of a changeset, with what its hunks are made of.
pub(super) struct FileParts {
    pub(super) change: Change,
    /// The file at the diff's start and at its head (empty where it isn't there).
    pub(super) old: Vec<u8>,
    pub(super) new: Vec<u8>,
    pub(super) hunks: Vec<(Hunk, DiffHunk)>,
}

pub(super) fn file_parts(
    t: &RepoTarget,
    span: &Span,
    start: &str,
    path: &str,
) -> Result<FileParts, ApiError> {
    let wanted = git::unquote_path(path).ok_or_else(|| {
        ApiError::validation(format!(
            "{path:?} isn't a path as review.changeset gives one"
        ))
    })?;
    let head = commit_of(&span.head, &t.repo)?;
    let changes = git::changes(&t.dirs, &t.worktree, &t.index, start, &head).map_err(git_failed)?;
    let Some(change) = changes.into_iter().find(|c| c.path() == wanted) else {
        return Err(ApiError::not_found(format!(
            "{path} in the changeset of repo {}",
            t.repo
        )));
    };
    // The file at each checkpoint along the way, following renames back from the head.
    let mut commits = vec![start.to_owned()];
    for cp in &span.steps {
        commits.push(commit_of(cp, &t.repo)?);
    }
    let refs: Vec<&str> = commits.iter().map(String::as_str).collect();
    let steps = git::steps(&t.dirs, &t.worktree, &t.index, &refs).map_err(git_failed)?;
    let versions = trace(&change, &steps);
    let last = versions.len() - 1;
    // The versions in between are read only if they all fit; without them, which turn wrote
    // what isn't known, and hunks say none rather than a wrong one.
    let all = blob_ids(versions.iter().flatten());
    let sizes = git::sizes(&t.dirs, &t.worktree, &t.index, &all).map_err(git_failed)?;
    let attribute = all.iter().map(|id| held(&sizes, id)).sum::<u64>() <= MAX_HELD;
    let ends = [&versions[0], &versions[last]];
    let ids = if attribute {
        all
    } else {
        blob_ids(ends.into_iter().flatten())
    };
    let blobs = read_blobs(t, &ids)?;
    // Each text read, and the version it's of: a version in between that's too large to read
    // is left out, so its changes count as the next one's.
    let mut texts: Vec<Cow<[u8]>> = Vec::with_capacity(versions.len());
    let mut owners: Vec<usize> = Vec::with_capacity(versions.len());
    for (k, side) in versions.iter().enumerate() {
        let end = k == 0 || k == last;
        if !end && !attribute {
            continue;
        }
        match body(side.as_ref(), &blobs)? {
            Body::Bytes(bytes) => {
                texts.push(bytes);
                owners.push(k);
            }
            Body::TooLarge(why) if end => {
                return Err(ApiError::validation(format!(
                    "{path} is too large to show ({why}; the limits are {} MiB and {MAX_LINES} \
                     lines)",
                    git::MAX_BLOB / 1024 / 1024
                )));
            }
            Body::TooLarge(_) => {}
        }
    }
    let lines: Vec<&[u8]> = texts.iter().map(|t| t.as_ref()).collect();
    let deadline = Instant::now() + DIFF_TIME;
    let (first, last) = (split(lines[0]), split(lines[lines.len() - 1]));
    let d = diff_lines(&first, &last, deadline, MAX_DIFF_LINES).map_err(|changed| {
        ApiError::validation(format!(
            "{path} changes {changed} lines; review.fileDiff shows up to {MAX_DIFF_LINES}"
        ))
    })?;
    drop((first, last));
    let history = history(&lines, deadline);
    let (head_id, from_id) = (
        span.head.id.to_string(),
        span.from
            .as_ref()
            .map_or_else(|| "base".to_owned(), |c| c.id.to_string()),
    );
    let scope = [head_id.as_str(), from_id.as_str(), t.repo.as_str(), path];
    let hunks = d
        .hunks
        .into_iter()
        .map(|h| {
            let hunk = Hunk {
                id: hunk_id(&scope, &h),
                old_start: h.old_start,
                old_lines: h.old_lines,
                new_start: h.new_start,
                new_lines: h.new_lines,
                lines: h.lines.iter().map(|l| display(l)).collect(),
                turn_id: history
                    .last_step(&h)
                    .filter(|_| attribute)
                    .and_then(|step| span.steps.get(owners[step] - 1))
                    .and_then(|cp| cp.turn.clone()),
            };
            (hunk, h)
        })
        .collect();
    Ok(FileParts {
        change,
        old: lines[0].to_vec(),
        new: lines[lines.len() - 1].to_vec(),
        hunks,
    })
}

/// The file `change` names at each commit along `steps` (one more commit than steps), traced
/// back from the head through renames: `None` where it isn't there. The ends are `change`'s
/// own sides, so the diff between them is the changeset's.
fn trace(change: &Change, steps: &[Vec<Change>]) -> Vec<Option<Side>> {
    let mut sides = vec![None; steps.len() + 1];
    sides[steps.len()] = change.new.clone();
    let mut name = change.path().to_owned();
    for k in (1..=steps.len()).rev() {
        let present = sides[k].is_some();
        let entry = steps[k - 1].iter().find(|e| {
            if present {
                e.new.as_ref().is_some_and(|n| n.path == name)
            } else {
                // Not there after this step: deleted in it, or renamed away.
                e.old.as_ref().is_some_and(|o| o.path == name)
                    && e.new.as_ref().is_none_or(|n| n.path != name)
            }
        });
        sides[k - 1] = match entry {
            Some(e) => {
                if let Some(old) = &e.old {
                    name = old.path.clone();
                }
                e.old.clone()
            }
            None => sides[k].clone(),
        };
    }
    sides[0] = change.old.clone();
    sides
}

#[cfg(test)]
mod tests {
    use super::*;

    fn side(path: &str, id: &str) -> Side {
        Side {
            path: path.as_bytes().to_vec(),
            mode: 0o100644,
            id: id.into(),
        }
    }

    fn change(kind: ChangeKind, old: Option<Side>, new: Option<Side>) -> Change {
        Change { kind, old, new }
    }

    fn ids(sides: &[Option<Side>]) -> Vec<Option<(&str, &str)>> {
        sides
            .iter()
            .map(|s| {
                s.as_ref()
                    .map(|s| (std::str::from_utf8(&s.path).unwrap(), s.id.as_str()))
            })
            .collect()
    }

    #[test]
    fn a_file_is_traced_back_through_its_edits_and_renames() {
        let modified = |path: &str, from: &str, to: &str| {
            change(
                ChangeKind::Modified,
                Some(side(path, from)),
                Some(side(path, to)),
            )
        };
        // Edited twice as mv.txt, then renamed and edited, with another file changing too.
        let steps = vec![
            vec![
                modified("mv.txt", "a0", "a1"),
                modified("other", "o0", "o1"),
            ],
            vec![modified("mv.txt", "a1", "a2")],
            vec![],
            vec![change(
                ChangeKind::Renamed,
                Some(side("mv.txt", "a2")),
                Some(side("moved.txt", "a3")),
            )],
        ];
        let whole = change(
            ChangeKind::Renamed,
            Some(side("mv.txt", "a0")),
            Some(side("moved.txt", "a3")),
        );
        assert_eq!(
            ids(&trace(&whole, &steps)),
            [
                Some(("mv.txt", "a0")),
                Some(("mv.txt", "a1")),
                Some(("mv.txt", "a2")),
                Some(("mv.txt", "a2")),
                Some(("moved.txt", "a3")),
            ]
        );
    }

    #[test]
    fn a_deleted_or_added_file_is_absent_where_it_isnt_there() {
        let steps = vec![
            vec![change(
                ChangeKind::Modified,
                Some(side("x", "x0")),
                Some(side("x", "x1")),
            )],
            vec![
                change(ChangeKind::Deleted, Some(side("x", "x1")), None),
                change(ChangeKind::Added, None, Some(side("f", "f2"))),
            ],
        ];
        let deleted = change(ChangeKind::Deleted, Some(side("x", "x0")), None);
        assert_eq!(
            ids(&trace(&deleted, &steps)),
            [Some(("x", "x0")), Some(("x", "x1")), None]
        );
        let added = change(ChangeKind::Added, None, Some(side("f", "f2")));
        assert_eq!(ids(&trace(&added, &steps)), [None, None, Some(("f", "f2"))]);
    }
}
