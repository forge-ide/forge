//! Worktrees, branches, and shadow refs.

pub mod commit;
pub mod naming;
pub mod worktree;

pub use naming::{NoSlug, derive_branch, slugify};
pub use worktree::{
    CreateMode, GitCli, GitDirs, GitError, NameUse, WORKTREES_DIR, Worktree, Worktrees,
    worktree_path,
};
