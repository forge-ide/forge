//! Hunk IDs that stay the same each time the same diff is computed, across calls and daemon
//! restarts, so a client can name a hunk it was shown. They hash what the hunk is: the
//! checkpoints it's between, its file, its ranges, and its lines. A hunk that changed, or one
//! from a diff against another checkpoint, gets another ID. The hash is SHA-256, cut to the
//! 128 bits an ID holds: the agent writes the lines, so it mustn't be able to make two hunks
//! share one.

use forge_api::HunkId;
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::diff::DiffHunk;

/// The ID of `hunk`, in the diff that `scope` names (its checkpoints, repo, and path).
pub fn hunk_id(scope: &[&str], hunk: &DiffHunk) -> HunkId {
    // Each part is length-prefixed, so no two different hunks write the same bytes.
    let mut bytes = Vec::new();
    let mut part = |p: &[u8]| {
        bytes.extend_from_slice(&(p.len() as u64).to_le_bytes());
        bytes.extend_from_slice(p);
    };
    for s in scope {
        part(s.as_bytes());
    }
    for n in [
        hunk.old_start,
        hunk.old_lines,
        hunk.new_start,
        hunk.new_lines,
    ] {
        part(&n.to_le_bytes());
    }
    for line in &hunk.lines {
        part(line);
    }
    let digest = Sha256::digest(&bytes);
    let mut first = [0u8; 16];
    first.copy_from_slice(&digest[..16]);
    format!(
        "{}{}",
        HunkId::PREFIX,
        Ulid::from(u128::from_be_bytes(first))
    )
    .parse()
    .expect("any 128 bits are a ULID")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hunk(lines: &[&str]) -> DiffHunk {
        DiffHunk {
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            lines: lines.iter().map(|l| l.as_bytes().to_vec()).collect(),
            removed: vec![0],
            added: vec![0],
        }
    }

    #[test]
    fn the_same_hunk_gets_the_same_id_and_any_difference_another() {
        let scope = ["chk_1", "base", "repo", "src/a.rs"];
        let id = hunk_id(&scope, &hunk(&["-a", "+b"]));
        assert!(id.as_str().starts_with("hnk_"));
        // Fixed, so a client's hunk IDs survive a daemon upgrade unless the format changes.
        // Fixed, so a client's hunk IDs survive a daemon upgrade unless the format changes.
        // Checked against an independent SHA-256 of the same bytes.
        assert_eq!(id.as_str(), "hnk_61WMNVRGSAPQ3NQ8EKS9EV41QK");
        assert_eq!(id.as_str().parse::<HunkId>().unwrap(), id);
        assert_eq!(hunk_id(&scope, &hunk(&["-a", "+b"])), id);
        assert_ne!(hunk_id(&scope, &hunk(&["-a", "+c"])), id);
        assert_ne!(
            hunk_id(&["chk_2", "base", "repo", "src/a.rs"], &hunk(&["-a", "+b"])),
            id
        );
        let mut moved = hunk(&["-a", "+b"]);
        moved.new_start = 2;
        assert_ne!(hunk_id(&scope, &moved), id);
        // Parts can't run into each other.
        assert_ne!(
            hunk_id(&["ab", "c"], &hunk(&["x"])),
            hunk_id(&["a", "bc"], &hunk(&["x"]))
        );
        assert_ne!(
            hunk_id(&scope, &hunk(&["-a+b"])),
            hunk_id(&scope, &hunk(&["-a", "+b"]))
        );
    }
}
