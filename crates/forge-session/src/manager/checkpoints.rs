//! Checkpoints of a session's worktrees after each turn, and `session.rewind` back to one.
//! Both read and write git through the repo's shared git dir and an index only the daemon
//! writes (see `forge_git::checkpoint`).

use std::path::PathBuf;

use forge_api::events::{CheckpointBody, EventPayload, SessionRewoundBody};
use forge_api::model::Session;
use forge_api::{ApiError, CheckpointId, ErrorData, ErrorKind, SessionId, Timestamp, TurnId};
use forge_git::checkpoint;
use forge_git::commit::CommitError;
use forge_git::{GitDirs, GitError};

use super::{SessionError, SessionManager, invalid_state, lock, repo_of};
use crate::state::can_rewind;
use crate::store::CheckpointRecord;

/// One repo's worktree, as checkpoints see it.
#[derive(Clone, Debug)]
pub(super) struct RepoTarget {
    repo: String,
    worktree: PathBuf,
    dirs: GitDirs,
    index: PathBuf,
}

/// Why a checkpoint or rewind failed, for the human.
struct Failed {
    kind: ErrorKind,
    detail: String,
}

impl From<CommitError> for Failed {
    fn from(e: CommitError) -> Self {
        let kind = match e {
            CommitError::Moved(_)
            | CommitError::WorktreeChanged
            | CommitError::PartlyRewound { .. } => ErrorKind::Conflict,
            CommitError::TooLarge => ErrorKind::ValidationFailed,
            _ => ErrorKind::InvalidState,
        };
        Failed {
            kind,
            detail: e.to_string(),
        }
    }
}

impl From<GitError> for Failed {
    fn from(e: GitError) -> Self {
        CommitError::Git(e).into()
    }
}

impl From<SessionError> for Failed {
    fn from(e: SessionError) -> Self {
        Failed {
            kind: ErrorKind::InvalidState,
            detail: e.to_string(),
        }
    }
}

impl SessionManager {
    /// Snapshot the session's worktrees after `turn`. A failure (changes over the size cap, a
    /// git error) is recorded as an `error` event, and the session carries on without one.
    /// The caller holds the session's lock.
    pub(super) async fn checkpoint_turn(&self, session: &Session, turn: &TurnId) {
        if let Err(failed) = self.take_checkpoint(session, Some(turn.clone())).await {
            tracing::warn!(session_id = %session.id, error = %failed.detail, "checkpoint failed");
            let _ = self
                .emit(
                    &session.id,
                    EventPayload::Error(ErrorData {
                        kind: failed.kind,
                        detail: format!("no checkpoint after this turn: {}", failed.detail),
                        retryable: false,
                    }),
                )
                .await;
        }
    }

    /// Rewind the session's worktrees, and its branch, to `to`. What they held just before is
    /// kept as a checkpoint first; it and every checkpoint after `to` are discarded, and the
    /// agent hears about it with its next prompt.
    pub async fn rewind(&self, id: &SessionId, to: &CheckpointId) -> Result<Session, SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        let key = id.clone();
        let all = self.blocking(move |store| store.checkpoints(&key)).await?;
        let Some(target) = all.iter().find(|c| &c.id == to).cloned() else {
            return Err(ApiError::not_found(format!("checkpoint {to} of session {id}")).into());
        };
        if !can_rewind(session.state) {
            return Err(invalid_state(&session, "rewind"));
        }
        if target.discarded_by.is_some() {
            return Err(ApiError::new(
                ErrorKind::InvalidState,
                format!("checkpoint {to} was discarded by an earlier rewind"),
            )
            .into());
        }
        self.register_log(id).await?;
        let failed = |f: Failed| SessionError::from(ApiError::new(f.kind, f.detail));
        let from = self.take_checkpoint(&session, None).await.map_err(|f| {
            failed(Failed {
                kind: f.kind,
                detail: format!(
                    "couldn't checkpoint the worktree before rewinding: {}",
                    f.detail
                ),
            })
        })?;
        let targets = self.checkpoint_targets(&session).await.map_err(failed)?;
        let (branch, from_commits, to_commits) = (
            session.branch.clone(),
            from.commits.clone(),
            target.commits.clone(),
        );
        tokio::task::spawn_blocking(move || {
            rewind_all(&targets, &branch, &from_commits, &to_commits)
        })
        .await
        .map_err(|e| SessionError::Internal(e.to_string()))?
        .map_err(failed)?;

        let mut discarded_turns: Vec<TurnId> = Vec::new();
        for turn in all
            .iter()
            .filter(|c| c.n > target.n && c.discarded_by.is_none())
            .filter_map(|c| c.turn.clone())
        {
            if !discarded_turns.contains(&turn) {
                discarded_turns.push(turn);
            }
        }
        let (key, n, by) = (id.clone(), target.n, target.id.clone());
        self.blocking(move |store| store.discard_checkpoints(&key, n, &by))
            .await?;
        self.emit(
            id,
            EventPayload::SessionRewound(SessionRewoundBody {
                checkpoint_id: target.id.clone(),
                discarded_turns,
            }),
        )
        .await?;
        *lock(&live.rewind_note) = Some(format!(
            "[forge] The human rewound the worktree to checkpoint {}: files and the branch are as \
             they were then, and changes made after it are gone. Read files again before relying \
             on what you remember of them.",
            target.n
        ));
        tracing::info!(session_id = %id, checkpoint = target.n, "rewound");
        self.get(id).await
    }

    /// Snapshot every worktree of the session as its next checkpoint, record it, and emit its
    /// event. The caller holds the session's lock.
    async fn take_checkpoint(
        &self,
        session: &Session,
        turn: Option<TurnId>,
    ) -> Result<CheckpointRecord, Failed> {
        let key = session.id.clone();
        let all = self.blocking(move |store| store.checkpoints(&key)).await?;
        let n = all.last().map_or(1, |c| c.n + 1);
        let previous = all
            .iter()
            .rev()
            .find(|c| c.discarded_by.is_none())
            .map(|c| c.commits.clone())
            .unwrap_or_default();
        let targets = self.checkpoint_targets(session).await?;
        let reference = checkpoint::checkpoint_ref(session.id.as_str(), n);
        let message = match &turn {
            Some(turn) => format!(
                "Forge checkpoint {n}\n\nSession {}, after turn {turn}.",
                session.id
            ),
            None => format!("Forge checkpoint {n}\n\nSession {}.", session.id),
        };
        let (branch, git_ref) = (session.branch.clone(), reference.clone());
        let commits = tokio::task::spawn_blocking(move || {
            targets
                .iter()
                .map(|t| {
                    let previous = previous
                        .iter()
                        .find(|(repo, _)| repo == &t.repo)
                        .map(|(_, commit)| commit.as_str());
                    checkpoint::take(
                        &t.dirs,
                        &t.worktree,
                        &branch,
                        previous,
                        &t.index,
                        &git_ref,
                        &message,
                    )
                    .map(|commit| (t.repo.clone(), commit))
                })
                .collect::<Result<Vec<_>, CommitError>>()
        })
        .await
        .map_err(|e| Failed {
            kind: ErrorKind::InvalidState,
            detail: e.to_string(),
        })??;
        let record = CheckpointRecord {
            id: CheckpointId::new(),
            session: session.id.clone(),
            n,
            turn: turn.clone(),
            created_at: Timestamp::now(),
            commits,
            discarded_by: None,
        };
        let row = record.clone();
        self.blocking(move |store| store.insert_checkpoint(&row))
            .await?;
        self.emit(
            &session.id,
            EventPayload::Checkpoint(CheckpointBody {
                checkpoint_id: record.id.clone(),
                turn_id: turn,
                git_ref: reference,
            }),
        )
        .await?;
        Ok(record)
    }

    /// The session's worktrees, with the git dirs and index their checkpoints use. Their git
    /// dirs are found once per session (and again after a daemon restart), as `git_commit`'s
    /// are at launch: the worktree's `.git` link is believed only when it points back at the
    /// repo, and rewriting it later doesn't change where the daemon's git goes.
    async fn checkpoint_targets(&self, session: &Session) -> Result<Vec<RepoTarget>, Failed> {
        let live = self.live(&session.id);
        if let Some(targets) = lock(&live.checkpoint_targets).clone() {
            return Ok(targets);
        }
        let mut targets = self.find_checkpoint_targets(session).await?;
        // Their git runs as `git_commit`'s does: confined, unless the session is unsandboxed,
        // reading a snapshot of the user's git config made for that repo.
        let scanner = self.redactor.scanner(&session.id);
        for t in &mut targets {
            let hex: String = t.repo.bytes().map(|b| format!("{b:02x}")).collect();
            let name = format!("git-config-{hex}");
            t.dirs.launch = self.git_launch(session, &t.dirs, &scanner, &name).await;
        }
        // A refusal (the user's git config couldn't be read, say) is tried again next time.
        if !targets
            .iter()
            .any(|t| matches!(t.dirs.launch, forge_git::Launch::Refused(_)))
        {
            *lock(&live.checkpoint_targets) = Some(targets.clone());
        }
        Ok(targets)
    }

    async fn find_checkpoint_targets(&self, session: &Session) -> Result<Vec<RepoTarget>, Failed> {
        let (store, git) = (self.store.clone(), self.git.clone());
        let scratch = self.dirs.sessions.join(session.id.to_string());
        let (id, branch) = (session.id.clone(), session.branch.clone());
        tokio::task::spawn_blocking(move || {
            let records = store.worktrees(&id).map_err(|e| Failed {
                kind: ErrorKind::InvalidState,
                detail: e.to_string(),
            })?;
            records
                .into_iter()
                .map(|w| {
                    let repo = repo_of(&w.path, &branch).ok_or_else(|| Failed {
                        kind: ErrorKind::InvalidState,
                        detail: format!("{} is not a session worktree", w.path.display()),
                    })?;
                    let dirs = git.git_dirs(&repo, &w.path)?;
                    Ok(RepoTarget {
                        index: checkpoint::index_path(&scratch, &w.repo),
                        repo: w.repo,
                        worktree: w.path,
                        dirs,
                    })
                })
                .collect()
        })
        .await
        .map_err(|e| Failed {
            kind: ErrorKind::InvalidState,
            detail: e.to_string(),
        })?
    }
}

/// Rewind each repo from `from` to `to`. If one refuses, the repos already rewound go back to
/// `from`, so the session's repos never disagree; one that can't is named in the error, with
/// the commit that holds what it had.
fn rewind_all(
    targets: &[RepoTarget],
    branch: &str,
    from: &[(String, String)],
    to: &[(String, String)],
) -> Result<(), Failed> {
    let commit = |set: &[(String, String)], repo: &str| {
        set.iter()
            .find(|(r, _)| r == repo)
            .map(|(_, c)| c.clone())
            .ok_or_else(|| Failed {
                kind: ErrorKind::InvalidState,
                detail: format!("the checkpoint has nothing for repo {repo}"),
            })
    };
    for (done, t) in targets.iter().enumerate() {
        let (a, b) = (commit(from, &t.repo)?, commit(to, &t.repo)?);
        if let Err(e) = checkpoint::rewind(&t.dirs, &t.worktree, branch, &a, &b, &t.index) {
            let mut failed = Failed::from(e);
            for t in &targets[..done] {
                let (Ok(a), Ok(b)) = (commit(from, &t.repo), commit(to, &t.repo)) else {
                    continue;
                };
                if let Err(e) = checkpoint::rewind(&t.dirs, &t.worktree, branch, &b, &a, &t.index) {
                    tracing::error!(repo = %t.repo, error = %e, "undoing a partial rewind");
                    failed.detail.push_str(&format!(
                        "; repo {} was rewound and couldn't be put back ({e}); what it held before is commit {a}",
                        t.repo
                    ));
                }
            }
            return Err(failed);
        }
    }
    Ok(())
}
