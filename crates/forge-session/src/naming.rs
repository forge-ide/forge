//! What already holds a session's branch name in each repo, and what the user's answer allows.

use std::path::{Path, PathBuf};

use forge_api::methods::{BranchCollision, CollisionKind, CollisionResolution};
use forge_api::{ApiError, ErrorKind, SessionId};
use forge_git::{CreateMode, NameUse};

/// How a branch name is used in one repo.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoName {
    pub repo: String,
    /// The repository root.
    pub path: PathBuf,
    pub worktree: PathBuf,
    pub used: NameUse,
    /// The live session that owns the worktree path, if any.
    pub owner: Option<SessionId>,
}

impl RepoName {
    /// Collisions to show the user, most serious first.
    pub fn collisions(&self) -> Vec<BranchCollision> {
        let mut out = Vec::new();
        let mut add = |kind, session_id| {
            out.push(BranchCollision {
                repo: self.repo.clone(),
                kind,
                session_id,
            })
        };
        if let Some(owner) = &self.owner {
            add(CollisionKind::OwnedByLiveSession, Some(owner.clone()));
        }
        if self.used.branch_exists {
            add(CollisionKind::BranchExists, None);
        }
        if self.used.worktree_exists {
            add(CollisionKind::WorktreeExists, None);
        }
        out
    }

    fn is_free(&self) -> bool {
        self.owner.is_none() && self.used.is_free()
    }

    fn checked_out_here(&self) -> bool {
        let here = std::fs::canonicalize(&self.worktree).ok();
        self.used
            .checked_out_at
            .as_deref()
            .is_some_and(|at| Some(at) == here.as_deref() || at == self.worktree)
    }
}

/// How to get the worktree in this repo, given the user's answer to any collision.
pub fn decide(
    name: &RepoName,
    branch: &str,
    resolution: Option<CollisionResolution>,
) -> Result<CreateMode, ApiError> {
    if name.is_free() {
        return Ok(CreateMode::New);
    }
    let conflict = |detail: String| Err(ApiError::new(ErrorKind::Conflict, detail));
    let repo = &name.repo;
    let at = display(&name.worktree);
    if let Some(owner) = &name.owner {
        return conflict(format!(
            "{repo}: the worktree {at} belongs to live session {owner}; choose another branch name"
        ));
    }
    match resolution {
        None => {
            let kinds: Vec<&str> = name
                .collisions()
                .iter()
                .map(|c| match c.kind {
                    CollisionKind::BranchExists => "the branch exists",
                    CollisionKind::WorktreeExists => "its worktree exists",
                    CollisionKind::OwnedByLiveSession => "a live session owns it",
                })
                .collect();
            conflict(format!(
                "{repo}: {branch:?} is taken ({}); choose another name, reuse the branch, or adopt the worktree",
                kinds.join(", ")
            ))
        }
        Some(CollisionResolution::ReuseBranch) => {
            if name.used.worktree_exists {
                return conflict(format!(
                    "{repo}: a worktree already exists at {at}; adopt it or choose another name"
                ));
            }
            if let Some(elsewhere) = &name.used.checked_out_at {
                return conflict(format!(
                    "{repo}: {branch:?} is checked out at {}; choose another name",
                    display(elsewhere)
                ));
            }
            Ok(CreateMode::ReuseBranch)
        }
        Some(CollisionResolution::AdoptWorktree) => {
            if !name.used.worktree_exists {
                return conflict(format!("{repo}: there is no worktree at {at} to adopt"));
            }
            if !name.checked_out_here() {
                return conflict(format!(
                    "{repo}: {at} is not a worktree of {branch:?}; move it away or choose another name"
                ));
            }
            Ok(CreateMode::Adopt)
        }
    }
}

fn display(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(used: NameUse, owner: Option<SessionId>) -> RepoName {
        RepoName {
            repo: "api".into(),
            path: PathBuf::from("/r"),
            worktree: PathBuf::from("/r/.worktrees/feat/x"),
            used,
            owner,
        }
    }

    fn mode(n: &RepoName, r: Option<CollisionResolution>) -> Result<CreateMode, ErrorKind> {
        decide(n, "feat/x", r).map_err(|e| e.kind())
    }

    use CollisionResolution::{AdoptWorktree, ReuseBranch};

    #[test]
    fn free_names_are_created_whatever_the_answer() {
        let free = name(NameUse::default(), None);
        assert!(free.collisions().is_empty());
        for r in [None, Some(ReuseBranch), Some(AdoptWorktree)] {
            assert_eq!(mode(&free, r), Ok(CreateMode::New));
        }
    }

    #[test]
    fn live_owners_always_conflict() {
        let owner = SessionId::new();
        let n = name(
            NameUse {
                branch_exists: true,
                worktree_exists: true,
                checked_out_at: Some("/r/.worktrees/feat/x".into()),
            },
            Some(owner.clone()),
        );
        let kinds: Vec<_> = n.collisions().iter().map(|c| c.kind).collect();
        assert_eq!(
            kinds,
            [
                CollisionKind::OwnedByLiveSession,
                CollisionKind::BranchExists,
                CollisionKind::WorktreeExists
            ]
        );
        assert_eq!(n.collisions()[0].session_id, Some(owner));
        for r in [None, Some(ReuseBranch), Some(AdoptWorktree)] {
            assert_eq!(mode(&n, r), Err(ErrorKind::Conflict));
        }
    }

    #[test]
    fn an_existing_branch_can_be_reused_unless_checked_out() {
        let n = name(
            NameUse {
                branch_exists: true,
                ..NameUse::default()
            },
            None,
        );
        assert_eq!(mode(&n, None), Err(ErrorKind::Conflict));
        assert_eq!(mode(&n, Some(ReuseBranch)), Ok(CreateMode::ReuseBranch));
        assert_eq!(mode(&n, Some(AdoptWorktree)), Err(ErrorKind::Conflict));

        let main = name(
            NameUse {
                branch_exists: true,
                checked_out_at: Some("/r".into()),
                ..NameUse::default()
            },
            None,
        );
        assert_eq!(mode(&main, Some(ReuseBranch)), Err(ErrorKind::Conflict));
    }

    #[test]
    fn an_existing_worktree_can_be_adopted_only_if_it_is_on_the_branch() {
        let n = name(
            NameUse {
                branch_exists: true,
                worktree_exists: true,
                checked_out_at: Some("/r/.worktrees/feat/x".into()),
            },
            None,
        );
        assert_eq!(mode(&n, Some(AdoptWorktree)), Ok(CreateMode::Adopt));
        assert_eq!(mode(&n, Some(ReuseBranch)), Err(ErrorKind::Conflict));

        let stray_dir = name(
            NameUse {
                worktree_exists: true,
                ..NameUse::default()
            },
            None,
        );
        assert_eq!(
            mode(&stray_dir, Some(AdoptWorktree)),
            Err(ErrorKind::Conflict)
        );
        let detail = decide(&stray_dir, "feat/x", None).unwrap_err();
        assert!(
            detail.detail().contains("worktree exists"),
            "{}",
            detail.detail()
        );
    }
}
