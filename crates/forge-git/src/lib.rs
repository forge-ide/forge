//! Worktrees, branches, and shadow refs.

pub mod checkpoint;
pub mod commit;
pub mod confine;
pub mod naming;
pub mod review;
pub mod worktree;

pub use confine::{Confine, Launch};
pub use naming::{NoSlug, derive_branch, slugify};
pub use worktree::{
    CreateMode, GitCli, GitDirs, GitError, NameUse, WORKTREES_DIR, Worktree, Worktrees,
    worktree_path,
};
