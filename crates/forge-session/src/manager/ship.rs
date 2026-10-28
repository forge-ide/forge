//! `review.revertHunk` and `review.ship`: what the human does to the session's branch after
//! review. Both start from the checkpoint reviewed and refuse if the worktree or branch moved
//! on since; both run under the session's lock, so no turn starts meanwhile. See
//! `forge_git::ship`.

use forge_api::methods::ShipStrategy;
use forge_api::{ApiError, CheckpointId, ErrorKind, HunkId, SessionId};
use forge_git::review::GITLINK;
use std::path::PathBuf;

use forge_git::ship::{self as git, FileEdit, Merge, MergeInto, ShipError};

use super::checkpoints::{Failed, RepoTarget};
use super::review::{MAX_DIFF_LINES, commit_of, file_parts, pick, start_of};
use super::{SessionError, SessionManager, invalid_state, lock, repo_of};
use crate::state::can_rewind;
use crate::store::CheckpointRecord;
use forge_api::model::SessionState;

/// The most withheld files a refusal names.
const NAMED: usize = 20;

impl From<ShipError> for Failed {
    fn from(e: ShipError) -> Self {
        let kind = match e {
            ShipError::Commit(e) => return e.into(),
            ShipError::NoRemote(_) | ShipError::Reserved(_) | ShipError::Layout(_) => {
                ErrorKind::InvalidState
            }
            _ => ErrorKind::Conflict,
        };
        Failed {
            kind,
            detail: e.to_string(),
        }
    }
}

fn failed(f: Failed) -> SessionError {
    ApiError::new(f.kind, f.detail).into()
}

fn joined<T>(r: Result<T, tokio::task::JoinError>) -> Result<T, SessionError> {
    r.map_err(|e| SessionError::Internal(e.to_string()))
}

impl SessionManager {
    /// Revert hunk `hunk` of `path` in the diff `review.fileDiff` gives for `repo` and `from`:
    /// a commit on the session branch putting its lines back, on top of what the agent left
    /// uncommitted, then a checkpoint of no turn. Only in Ready or Review, and only while the
    /// worktree is as the newest checkpoint has it.
    #[allow(clippy::too_many_arguments)]
    pub async fn revert_hunk(
        &self,
        id: &SessionId,
        repo: Option<&str>,
        path: &str,
        hunk: &HunkId,
        from: Option<&CheckpointId>,
        bases: Vec<(String, String)>,
    ) -> Result<(), SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        if !matches!(session.state, SessionState::Ready | SessionState::Review) {
            return Err(invalid_state(&session, "revert a hunk"));
        }
        let span = self.span(id, from).await?;
        let target = pick(self.review_targets(id).await?, repo, id)?;
        let (wanted, path_owned) = (hunk.clone(), path.to_owned());
        let (t, s) = (target.clone(), span.clone());
        let parts = joined(
            tokio::task::spawn_blocking(move || {
                let start = start_of(&t, &s, &bases)?;
                file_parts(&t, &s, &start, &path_owned)
            })
            .await,
        )??;
        let Some((shown, diff)) = parts.hunks.iter().find(|(h, _)| h.id == wanted) else {
            return Err(ApiError::not_found(format!("hunk {hunk} of {path}")).into());
        };
        let change = &parts.change;
        if [&change.old, &change.new]
            .into_iter()
            .flatten()
            .any(|side| side.mode == GITLINK)
        {
            return Err(ApiError::validation(format!(
                "{path} is a submodule; its pinned commit can't be reverted by hunk"
            ))
            .into());
        }
        let content = forge_review::revert(&parts.old, &parts.new, diff);
        // The file's mode at the head, or before it was deleted; a file the changeset added
        // and the revert empties goes away.
        let mode = change
            .new
            .as_ref()
            .or(change.old.as_ref())
            .map(|s| format!("{:06o}", s.mode))
            .unwrap_or_else(|| "100644".to_owned());
        let deleted = change.old.is_none() && content.is_empty();
        let head = commit_of(&span.head, &target.repo)?;
        let settle = self.settle_message(&span.head, id);
        let message = format!(
            "Revert a hunk of {path}\n\nThe human reverted lines {}-{} of {path} in Forge's \
             review of session {id}.",
            shown.new_start,
            shown.new_start + shown.new_lines.saturating_sub(1),
        );
        let (branch, file) = (session.branch.clone(), change.path().to_vec());
        let commit = joined(
            tokio::task::spawn_blocking(move || {
                let edit = FileEdit {
                    path: &file,
                    content: (!deleted).then_some((mode.as_str(), content.as_slice())),
                    message: &message,
                };
                git::edit_file(
                    &target.dirs,
                    &target.worktree,
                    &branch,
                    &head,
                    &target.index,
                    &edit,
                    &settle,
                )
            })
            .await,
        )?
        .map_err(|e| failed(e.into()))?;
        // The revert is reviewed like any change: a checkpoint of no turn holds it, and threads
        // on the lines it put back go outdated.
        if let Err(f) = self.take_checkpoint(&session, None).await {
            tracing::warn!(session_id = %id, error = %f.detail, "no checkpoint after a revert");
        }
        self.sweep_threads(id).await;
        let note = format!(
            "[forge] The human reverted part of your changes in commit {commit} on your branch. \
             Read the files again before relying on what you remember of them."
        );
        let mut slot = lock(&live.rewind_note);
        *slot = Some(match slot.take() {
            Some(earlier) => format!("{earlier}\n\n{note}"),
            None => note,
        });
        tracing::info!(session_id = %id, "reverted a hunk");
        Ok(())
    }

    /// Ship checkpoint `checkpoint`, which must be the session's newest and still match the
    /// worktree and branch: merged into each repo's base branch (`bases`), which closes the
    /// session as shipped, or pushed, which leaves it in review. Returns the ref it went to
    /// (the first repo's).
    #[allow(clippy::too_many_arguments)]
    pub async fn ship(
        &self,
        id: &SessionId,
        checkpoint: &CheckpointId,
        strategy: ShipStrategy,
        branch: Option<String>,
        include_withheld: bool,
        bases: Vec<(String, String)>,
    ) -> Result<String, SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        if !can_rewind(session.state) {
            return Err(invalid_state(&session, "ship"));
        }
        let key = id.clone();
        let all = self.blocking(move |store| store.checkpoints(&key)).await?;
        let Some(reviewed) = all.iter().find(|c| &c.id == checkpoint).cloned() else {
            return Err(
                ApiError::not_found(format!("checkpoint {checkpoint} of session {id}")).into(),
            );
        };
        if reviewed.discarded_by.is_some() {
            return Err(ApiError::new(
                ErrorKind::InvalidState,
                format!("checkpoint {checkpoint} was discarded by an earlier rewind"),
            )
            .into());
        }
        if let Some(newest) = all.iter().rev().find(|c| c.discarded_by.is_none())
            && newest.id != reviewed.id
        {
            return Err(ApiError::new(
                ErrorKind::Conflict,
                format!(
                    "checkpoint {} isn't the session's newest ({}); review the changes since \
                     and ship that",
                    reviewed.n, newest.n
                ),
            )
            .into());
        }
        let targets = self.checkpoint_targets(&session).await.map_err(failed)?;
        let push_to = match (strategy, branch) {
            (ShipStrategy::MergeLocal, Some(_)) => {
                return Err(ApiError::validation(
                    "a branch name is for push-branch only; merge-local merges into each repo's \
                     base branch"
                        .to_owned(),
                )
                .into());
            }
            (ShipStrategy::MergeLocal, None) => None,
            (ShipStrategy::PushBranch, name) => {
                let name = name.unwrap_or_else(|| session.branch.clone());
                for t in &targets {
                    let repo = repo_of(&t.worktree, &session.branch).unwrap_or_default();
                    self.git
                        .validate_branch(&repo, &name)
                        .map_err(|e| ApiError::validation(e.to_string()))?;
                }
                Some(name)
            }
        };
        if !include_withheld {
            let changeset = self.changeset(id, None, bases.clone()).await?;
            let withheld: Vec<String> = changeset
                .files
                .iter()
                .filter(|f| f.too_large || f.additions + f.deletions > MAX_DIFF_LINES)
                .map(|f| match &f.repo {
                    Some(repo) if targets.len() > 1 => format!("{repo}: {}", f.path),
                    _ => f.path.clone(),
                })
                .collect();
            if !withheld.is_empty() {
                let more = withheld.len().saturating_sub(NAMED);
                let mut named = withheld[..withheld.len().min(NAMED)].join(", ");
                if more > 0 {
                    named.push_str(&format!(" and {more} more"));
                }
                return Err(ApiError::validation(format!(
                    "some files were too large to review: {named}; ship with includeWithheld to \
                     ship them unreviewed"
                ))
                .into());
            }
        }
        let settle = self.settle_message(&reviewed, id);
        // Every worktree Forge made in the project's repos, whatever its session's state: git's
        // records of them come from dirs their agents can write.
        let project = session.project_id.clone();
        let sessions = self
            .blocking(move |store| {
                let mut paths = Vec::new();
                for s in store.list(&project, None)? {
                    paths.extend(store.worktrees(&s.id)?.into_iter().map(|w| w.path));
                }
                Ok(paths)
            })
            .await?;
        let (session_branch, sid) = (session.branch.clone(), id.clone());
        let shipped = joined(
            tokio::task::spawn_blocking(move || {
                ship_all(
                    &targets,
                    &reviewed,
                    &session_branch,
                    &bases,
                    &settle,
                    push_to.as_deref(),
                    &sid,
                    &sessions,
                )
            })
            .await,
        )?
        .map_err(failed)?;
        tracing::info!(session_id = %id, ?strategy, "shipped");
        if strategy == ShipStrategy::MergeLocal {
            self.close_locked(&live, &session, "shipped").await?;
        }
        Ok(shipped)
    }

    fn settle_message(&self, checkpoint: &CheckpointRecord, id: &SessionId) -> String {
        format!(
            "Changes from Forge checkpoint {}\n\nWhat the agent had left uncommitted when the \
             human reviewed session {id}.",
            checkpoint.n
        )
    }
}

/// Ship every repo: check them all first, then put each branch at the reviewed checkpoint,
/// then merge or push. Merges are worked out for every repo before any base branch moves,
/// and undone in the repos already merged if one fails.
#[allow(clippy::too_many_arguments)]
fn ship_all(
    targets: &[RepoTarget],
    reviewed: &CheckpointRecord,
    branch: &str,
    bases: &[(String, String)],
    settle: &str,
    push_to: Option<&str>,
    id: &SessionId,
    sessions: &[PathBuf],
) -> Result<String, Failed> {
    let base_of = |repo: &str| {
        bases
            .iter()
            .find(|(r, _)| r == repo)
            .map(|(_, b)| b.clone())
            .ok_or_else(|| Failed {
                kind: ErrorKind::InvalidState,
                detail: format!("repo {repo} is no longer in the project"),
            })
    };
    let mut commits = Vec::with_capacity(targets.len());
    for t in targets {
        let commit = commit_of(reviewed, &t.repo).map_err(|e| Failed {
            kind: e.kind(),
            detail: e.detail().to_owned(),
        })?;
        forge_git::checkpoint::unchanged(&t.dirs, &t.worktree, branch, &commit, &t.index)?;
        commits.push(commit);
    }
    let mut tips = Vec::with_capacity(targets.len());
    for (t, commit) in targets.iter().zip(&commits) {
        tips.push(git::commit_reviewed(
            &t.dirs,
            &t.worktree,
            branch,
            commit,
            &t.index,
            settle,
        )?);
    }
    if let Some(to) = push_to {
        let mut first = None;
        for (t, tip) in targets.iter().zip(&tips) {
            let base = base_of(&t.repo)?;
            let remote = git::push(&t.dirs, &t.worktree, &t.index, &base, tip, to)?;
            first.get_or_insert(format!("{remote}/{to}"));
        }
        return Ok(first.unwrap_or_default());
    }
    let message = format!("Merge branch '{branch}'\n\nShipped from Forge session {id}.");
    let mut merges: Vec<(&RepoTarget, Merge)> = Vec::with_capacity(targets.len());
    for (t, tip) in targets.iter().zip(&tips) {
        let base = base_of(&t.repo)?;
        let repo = repo_of(&t.worktree, branch).ok_or_else(|| Failed {
            kind: ErrorKind::InvalidState,
            detail: format!("{} is not a session worktree", t.worktree.display()),
        })?;
        let into = MergeInto {
            base: &base,
            repo: &repo,
            sessions,
        };
        merges.push((
            t,
            git::plan_merge(&t.dirs, &t.worktree, &t.index, &into, tip, &message)?,
        ));
    }
    for (done, (t, m)) in merges.iter().enumerate() {
        if let Err(e) = git::apply_merge(&t.dirs, &t.worktree, &t.index, m) {
            let mut f = Failed::from(e);
            for (t, m) in &merges[..done] {
                if let Err(e) = git::undo_merge(&t.dirs, &t.worktree, &t.index, m) {
                    tracing::error!(repo = %t.repo, error = %e, "undoing a partial ship");
                    f.detail.push_str(&format!(
                        "; repo {} was merged and couldn't be put back ({e}); its base branch \
                         {} was at {}",
                        t.repo, m.base, m.old
                    ));
                }
            }
            return Err(f);
        }
    }
    Ok(merges
        .first()
        .map(|(_, m)| format!("refs/heads/{}", m.base))
        .unwrap_or_default())
}
