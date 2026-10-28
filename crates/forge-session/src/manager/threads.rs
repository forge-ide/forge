//! Review threads: comments on lines of the changeset, replies from the human and (through
//! Forge's MCP endpoint) the agent, resolving, files marked viewed per round, and "Request
//! changes", which sends the agent every thread it hasn't answered as one message.
//!
//! A thread is written on a range of a file at the newest checkpoint. Where those lines are at
//! a later checkpoint is worked out from git each time: they follow the code while every line
//! is still there, unchanged and together, and the thread is outdated, for good, the first
//! checkpoint they aren't. Bodies are untrusted text, never logged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_api::events::{EventPayload, ReviewThreadBody, ThreadChange, TurnSource};
use forge_api::model::{Comment, CommentAuthor, LineRange, Thread, ThreadStatus};
use forge_api::{ApiError, CommentId, ErrorKind, SessionId, ThreadId, Timestamp, TurnId};
use forge_git::review::{self as git, Blob, GITLINK};
use forge_review::{Followed, LineMap, line_count, line_map, quote, request_changes};

use super::checkpoints::RepoTarget;
use super::review::{MAX_LINES, commit_of, git_failed, pick, start_of};
use super::{SessionError, SessionManager};
use crate::store::{CheckpointRecord, ThreadRecord};

/// The longest comment or reply.
pub const MAX_BODY: usize = 16 * 1024;

/// How long following one thread's lines may look for the smallest diff.
const FOLLOW_TIME: Duration = Duration::from_secs(5);

impl SessionManager {
    /// Start a thread on lines `range` of `path`, a file in the changeset (from each repo's
    /// merge-base with its base branch, `bases`), as it is at the newest checkpoint.
    pub async fn comment(
        &self,
        id: &SessionId,
        repo: Option<&str>,
        path: &str,
        range: LineRange,
        body: String,
        bases: Vec<(String, String)>,
    ) -> Result<Thread, SessionError> {
        check_body(&body)?;
        let span = self.span(id, None).await?;
        let target = pick(self.review_targets(id).await?, repo, id)?;
        let (wanted, head) = (path.to_owned(), span.head.clone());
        let (t, s) = (target.clone(), span);
        let (path, blob, quoted) =
            tokio::task::spawn_blocking(move || anchor_new(&t, &s, &bases, &wanted, range))
                .await
                .map_err(|e| SessionError::Internal(e.to_string()))??;
        let record = ThreadRecord {
            id: ThreadId::new(),
            session: id.clone(),
            repo: target.repo.clone(),
            path,
            start_line: range.start_line,
            end_line: range.end_line,
            checkpoint: head.id.clone(),
            blob,
            status: ThreadStatus::Open,
            created_at: Timestamp::now(),
            comments: vec![new_comment(CommentAuthor::Human, body)],
        };
        let row = record.clone();
        self.blocking(move |store| store.insert_thread(&row))
            .await?;
        self.thread_event(id, &record.id, ThreadChange::Created)
            .await?;
        Ok(Thread {
            id: record.id,
            session_id: id.clone(),
            repo: Some(record.repo),
            path: record.path,
            range,
            checkpoint_id: record.checkpoint,
            quote: quoted,
            current: Some(range),
            status: ThreadStatus::Open,
            comments: record.comments,
        })
    }

    /// The human's reply to a thread. A reply never changes the thread's status.
    pub async fn reply(&self, thread: &ThreadId, body: String) -> Result<Thread, SessionError> {
        check_body(&body)?;
        let record = self.thread_record(thread).await?;
        self.add_comment(&record, new_comment(CommentAuthor::Human, body))
            .await?;
        self.thread_view(thread).await
    }

    /// The agent's reply, through `review_reply`: only to a thread of its own session, and with
    /// secrets redacted as in the event log. The error is for the agent.
    pub(super) async fn agent_reply(
        &self,
        session: &SessionId,
        thread: &ThreadId,
        body: &str,
    ) -> Result<(), String> {
        let theirs = || format!("no review thread of this session has the ID {thread}");
        let key = thread.clone();
        let record = self
            .blocking(move |store| store.thread(&key))
            .await
            .map_err(|e| e.to_string())?
            .filter(|t| &t.session == session)
            .ok_or_else(theirs)?;
        check_body(body).map_err(|e| e.detail().to_owned())?;
        let body = self.redactor.scanner(session).redact(body).into_owned();
        // Redacting can lengthen it; what's stored keeps to the limit too.
        check_body(&body).map_err(|e| e.detail().to_owned())?;
        self.add_comment(&record, new_comment(CommentAuthor::Agent, body))
            .await
            .map_err(|e| e.to_string())
    }

    /// Resolve an open or outdated thread; it stays resolved.
    pub async fn resolve(&self, thread: &ThreadId) -> Result<Thread, SessionError> {
        let record = self.thread_record(thread).await?;
        let key = thread.clone();
        // One step, so a sweep outdating it meanwhile can't undo the resolve, nor two resolve it.
        let resolved = self
            .blocking(move |store| {
                store.transition_thread(
                    &key,
                    &[ThreadStatus::Open, ThreadStatus::Outdated],
                    ThreadStatus::Resolved,
                )
            })
            .await?;
        if !resolved {
            return Err(ApiError::new(
                ErrorKind::InvalidState,
                format!("thread {thread} is already resolved"),
            )
            .into());
        }
        self.thread_event(&record.session, thread, ThreadChange::Resolved)
            .await?;
        self.thread_view(thread).await
    }

    /// The session's threads, oldest first, optionally only those with `status`.
    pub async fn threads(
        &self,
        id: &SessionId,
        status: Option<ThreadStatus>,
    ) -> Result<Vec<Thread>, SessionError> {
        self.get(id).await?;
        let key = id.clone();
        let records = self.blocking(move |store| store.threads(&key)).await?;
        let records = records
            .into_iter()
            .filter(|t| status.is_none_or(|s| t.status == s))
            .collect();
        self.views(id, records).await
    }

    /// Mark a file viewed, or not, in the session's current review round.
    pub async fn set_viewed(
        &self,
        id: &SessionId,
        repo: Option<&str>,
        path: &str,
        viewed: bool,
    ) -> Result<(), SessionError> {
        self.get(id).await?;
        if git::unquote_path(path).is_none() {
            return Err(ApiError::validation(format!(
                "{path:?} isn't a path as review.changeset gives one"
            ))
            .into());
        }
        let repo = pick(self.review_targets(id).await?, repo, id)?.repo;
        let (key, path) = (id.clone(), path.to_owned());
        self.blocking(move |store| {
            let round = store.rounds(&key)?.len() as u32 + 1;
            store.set_viewed(&key, round, &repo, &path, viewed)
        })
        .await
    }

    /// Send the agent every thread that isn't resolved and whose last comment is the human's,
    /// as one message, in a turn of its own, and start the next review round.
    pub async fn request_changes(self: &Arc<Self>, id: &SessionId) -> Result<TurnId, SessionError> {
        self.get(id).await?;
        let key = id.clone();
        let records = self.blocking(move |store| store.threads(&key)).await?;
        let waiting: Vec<ThreadRecord> = records
            .into_iter()
            .filter(|t| t.status != ThreadStatus::Resolved)
            .filter(|t| {
                t.comments
                    .last()
                    .is_some_and(|c| c.author == CommentAuthor::Human)
            })
            .collect();
        if waiting.is_empty() {
            return Err(ApiError::validation(
                "there are no threads to send: every unresolved one has the agent's answer"
                    .to_owned(),
            )
            .into());
        }
        let head = self.span(id, None).await?.head;
        let threads = self.views(id, waiting).await?;
        let turn = self
            .prompt(id, request_changes(&threads), TurnSource::Review)
            .await?;
        let key = id.clone();
        self.blocking(move |store| store.add_round(&key, &head.id, Timestamp::now()))
            .await?;
        Ok(turn)
    }

    /// Mark the open threads whose lines changed at the newest checkpoint outdated. A failure
    /// leaves them as they are.
    pub(super) async fn sweep_threads(&self, id: &SessionId) {
        let key = id.clone();
        let open = match self.blocking(move |store| store.threads(&key)).await {
            Ok(all) => all
                .into_iter()
                .filter(|t| t.status == ThreadStatus::Open)
                .collect::<Vec<_>>(),
            Err(e) => {
                tracing::warn!(session_id = %id, error = %e, "listing threads to update");
                return;
            }
        };
        if open.is_empty() {
            return;
        }
        let followed = match self.follow_threads(id, open).await {
            Ok(followed) => followed,
            Err(e) => {
                tracing::warn!(session_id = %id, error = %e, "following threads");
                return;
            }
        };
        // Only lines known to be gone outdate a thread; one whose diff ran out of time is
        // tried again at the next checkpoint. Only a thread still open is outdated, so one the
        // human resolved meanwhile stays resolved.
        for (view, fate) in followed {
            if fate != Followed::Gone {
                continue;
            }
            let key = view.id.clone();
            let outdated = self
                .blocking(move |store| {
                    store.transition_thread(&key, &[ThreadStatus::Open], ThreadStatus::Outdated)
                })
                .await;
            if let Ok(true) = outdated {
                let _ = self
                    .thread_event(id, &view.id, ThreadChange::Outdated)
                    .await;
            }
        }
    }

    async fn thread_record(&self, thread: &ThreadId) -> Result<ThreadRecord, SessionError> {
        let key = thread.clone();
        self.blocking(move |store| store.thread(&key))
            .await?
            .ok_or_else(|| ApiError::not_found(format!("thread {thread}")).into())
    }

    async fn thread_view(&self, thread: &ThreadId) -> Result<Thread, SessionError> {
        let record = self.thread_record(thread).await?;
        let session = record.session.clone();
        let mut views = self.views(&session, vec![record]).await?;
        Ok(views.remove(0))
    }

    async fn add_comment(
        &self,
        record: &ThreadRecord,
        comment: Comment,
    ) -> Result<(), SessionError> {
        let key = record.id.clone();
        self.blocking(move |store| store.add_comment(&key, &comment))
            .await?;
        self.thread_event(&record.session, &record.id, ThreadChange::Reply)
            .await
    }

    async fn thread_event(
        &self,
        session: &SessionId,
        thread: &ThreadId,
        change: ThreadChange,
    ) -> Result<(), SessionError> {
        self.register_log(session).await?;
        self.emit(
            session,
            EventPayload::ReviewThread(ReviewThreadBody {
                thread_id: thread.clone(),
                change,
            }),
        )
        .await
    }

    /// `records` as the API shows them: each with the lines it quotes, and where they are at
    /// the newest checkpoint unless it's outdated.
    async fn views(
        &self,
        id: &SessionId,
        records: Vec<ThreadRecord>,
    ) -> Result<Vec<Thread>, SessionError> {
        let followed = self.follow_threads(id, records).await?;
        Ok(followed.into_iter().map(|(view, _)| view).collect())
    }

    /// [`Self::views`], each with where its lines went.
    async fn follow_threads(
        &self,
        id: &SessionId,
        records: Vec<ThreadRecord>,
    ) -> Result<Vec<(Thread, Followed)>, SessionError> {
        if records.is_empty() {
            return Ok(Vec::new());
        }
        let head = self.span(id, None).await?.head;
        let targets = self.review_targets(id).await?;
        let key = id.clone();
        let checkpoints = self.blocking(move |store| store.checkpoints(&key)).await?;
        tokio::task::spawn_blocking(move || follow_all(&targets, &checkpoints, &head, records))
            .await
            .map_err(|e| SessionError::Internal(e.to_string()))?
            .map_err(SessionError::from)
    }
}

fn check_body(body: &str) -> Result<(), ApiError> {
    if body.trim().is_empty() {
        return Err(ApiError::validation("a comment needs some text".to_owned()));
    }
    if body.len() > MAX_BODY {
        return Err(ApiError::validation(format!(
            "a comment is up to {} KiB",
            MAX_BODY / 1024
        )));
    }
    Ok(())
}

fn new_comment(author: CommentAuthor, body: String) -> Comment {
    Comment {
        id: CommentId::new(),
        author,
        body,
        created_at: Timestamp::now(),
    }
}

/// For a new thread on `path` at the newest checkpoint: the path as the changeset gives it,
/// the file's blob there, and the quoted lines. Refuses a file that isn't in the changeset,
/// isn't there at the newest checkpoint, or is too large, and a range that isn't in it.
fn anchor_new(
    t: &RepoTarget,
    span: &super::review::Span,
    bases: &[(String, String)],
    path: &str,
    range: LineRange,
) -> Result<(String, String, Vec<String>), ApiError> {
    let wanted = git::unquote_path(path).ok_or_else(|| {
        ApiError::validation(format!(
            "{path:?} isn't a path as review.changeset gives one"
        ))
    })?;
    let start = start_of(t, span, bases)?;
    let head = commit_of(&span.head, &t.repo)?;
    let changes =
        git::changes(&t.dirs, &t.worktree, &t.index, &start, &head).map_err(git_failed)?;
    let refused = |why: &str| ApiError::validation(format!("can't comment on {path}: {why}"));
    let change = changes
        .into_iter()
        .find(|c| c.path() == wanted)
        .ok_or_else(|| refused("it isn't in the changeset"))?;
    let side = change.new.ok_or_else(|| refused("it was deleted"))?;
    if side.mode == GITLINK {
        return Err(refused("it's a submodule"));
    }
    let blobs = git::blobs(&t.dirs, &t.worktree, &t.index, &[&side.id]).map_err(git_failed)?;
    let Some(Blob::Bytes(bytes)) = blobs.get(&side.id) else {
        return Err(refused("it's too large to show"));
    };
    let lines = line_count(bytes);
    if lines > MAX_LINES {
        return Err(refused("it's too large to show"));
    }
    if range.start_line == 0 || range.start_line > range.end_line || range.end_line as usize > lines
    {
        return Err(refused(&format!(
            "lines {}-{} aren't in it (it has {lines})",
            range.start_line, range.end_line
        )));
    }
    let quoted = quote(bytes, range.start_line, range.end_line);
    Ok((git::quote_path(&side.path), side.id, quoted))
}

/// Where a thread's lines are at the head, as far as git can tell.
enum Where {
    /// In this blob.
    Blob(String),
    /// Not anywhere: the file is gone, or never could be followed.
    Gone,
    /// Git couldn't say (a commit or blob missing); left as it is.
    Unsure,
}

/// Each thread with its quote, and where its lines are at `head` unless it's outdated. Git
/// is asked once per repo and checkpoint the threads were written on, blobs are read once,
/// each pair of versions is diffed once, and all of it shares one time budget: a thread its
/// diff ran out of time on is unsure, never outdated, so a slow file costs a call no more than
/// [`FOLLOW_TIME`]. A thread git can't follow (its checkpoint pruned, say) is unsure too,
/// without failing the rest.
fn follow_all(
    targets: &[RepoTarget],
    checkpoints: &[CheckpointRecord],
    head: &CheckpointRecord,
    records: Vec<ThreadRecord>,
) -> Result<Vec<(Thread, Followed)>, ApiError> {
    let deadline = Instant::now() + FOLLOW_TIME;
    let mut now: Vec<Where> = Vec::with_capacity(records.len());
    let mut changes: HashMap<(String, String), Option<Vec<git::Change>>> = HashMap::new();
    for r in &records {
        let target = targets.iter().find(|t| t.repo == r.repo);
        let written = checkpoints.iter().find(|c| c.id == r.checkpoint);
        let (Some(t), Some(written)) = (target, written) else {
            now.push(Where::Gone);
            continue;
        };
        if r.status == ThreadStatus::Outdated {
            now.push(Where::Gone);
            continue;
        }
        let (from, to) = (commit_of(written, &r.repo)?, commit_of(head, &r.repo)?);
        let key = (r.repo.clone(), from.clone());
        let found = changes.entry(key).or_insert_with(|| {
            match git::changes(&t.dirs, &t.worktree, &t.index, &from, &to) {
                Ok(found) => Some(found),
                Err(e) => {
                    tracing::warn!(thread_id = %r.id, error = %e, "following a thread");
                    None
                }
            }
        });
        let Some(found) = found else {
            now.push(Where::Unsure);
            continue;
        };
        let moved = found.iter().find(|c| {
            c.old
                .as_ref()
                .is_some_and(|o| git::quote_path(&o.path) == r.path)
        });
        now.push(match moved {
            None => Where::Blob(r.blob.clone()),
            Some(c) => match c.new.as_ref().filter(|n| n.mode != GITLINK) {
                Some(n) => Where::Blob(n.id.clone()),
                None => Where::Gone,
            },
        });
    }
    // Every blob, per repo; one that can't be read just isn't there.
    let mut blobs: HashMap<String, HashMap<String, Blob>> = HashMap::new();
    for t in targets {
        let mut ids: Vec<&str> = records
            .iter()
            .zip(&now)
            .filter(|(r, _)| r.repo == t.repo)
            .flat_map(|(r, n)| {
                let at = match n {
                    Where::Blob(id) => Some(id.as_str()),
                    Where::Gone | Where::Unsure => None,
                };
                std::iter::once(r.blob.as_str()).chain(at)
            })
            .collect();
        ids.sort_unstable();
        ids.dedup();
        let read = |ids: &[&str]| git::blobs(&t.dirs, &t.worktree, &t.index, ids);
        let found = match read(&ids) {
            Ok(found) => found,
            Err(_) => ids
                .iter()
                .filter_map(|id| read(&[id]).ok())
                .flatten()
                .collect(),
        };
        blobs.insert(t.repo.clone(), found);
    }
    let bytes = |repo: &str, id: &str| match blobs.get(repo).and_then(|b| b.get(id)) {
        Some(Blob::Bytes(b)) if line_count(b) <= MAX_LINES => Some(b.as_slice()),
        _ => None,
    };
    let readable = |repo: &str, id: &str| blobs.get(repo).is_some_and(|b| b.contains_key(id));
    // One diff per pair of versions, however many threads are on it.
    let mut maps: HashMap<(String, String), LineMap> = HashMap::new();
    Ok(records
        .into_iter()
        .zip(now)
        .map(|(r, now)| {
            let written = bytes(&r.repo, &r.blob);
            let quoted = written
                .map(|b| quote(b, r.start_line, r.end_line))
                .unwrap_or_default();
            let fate = match now {
                Where::Unsure => Followed::Unsure,
                Where::Gone => Followed::Gone,
                // Missing from the object store: git can't say, which isn't the lines going.
                Where::Blob(id) if !readable(&r.repo, &r.blob) || !readable(&r.repo, &id) => {
                    Followed::Unsure
                }
                Where::Blob(id) => match (written, bytes(&r.repo, &id)) {
                    (Some(old), Some(new)) => maps
                        .entry((r.blob.clone(), id))
                        .or_insert_with(|| line_map(old, new, deadline))
                        .follow(r.start_line, r.end_line),
                    // Too large to follow, now or then.
                    _ => Followed::Gone,
                },
            };
            let current = match fate {
                Followed::At(start_line, end_line) => Some(LineRange {
                    start_line,
                    end_line,
                }),
                Followed::Gone | Followed::Unsure => None,
            };
            let view = Thread {
                id: r.id,
                session_id: r.session,
                repo: Some(r.repo),
                path: r.path,
                range: LineRange {
                    start_line: r.start_line,
                    end_line: r.end_line,
                },
                checkpoint_id: r.checkpoint,
                quote: quoted,
                current,
                status: r.status,
                comments: r.comments,
            };
            (view, fate)
        })
        .collect())
}
