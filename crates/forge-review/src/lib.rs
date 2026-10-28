//! Changesets and comment threads: diffs of a session's files between checkpoints, which turn
//! each hunk came from, and stable hunk IDs. Everything here works on text already read out
//! of git; nothing in it reads the worktree.

pub mod anchor;
pub mod diff;
pub mod history;
pub mod id;
pub mod request;

pub use anchor::{Followed, LineMap, follow, line_map, quote};
pub use diff::{
    CONTEXT, DiffHunk, FileDiff, counts, diff, diff_lines, display, is_binary, line_count, split,
};
pub use history::{History, history};
pub use id::hunk_id;
pub use request::request_changes;

/// This crate's package name.
pub const NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    #[test]
    fn name_matches_package() {
        assert_eq!(super::NAME, "forge-review");
    }
}
