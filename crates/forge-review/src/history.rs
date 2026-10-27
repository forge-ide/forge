//! Which step of a file's history changed each line: given the file's versions from a start
//! point (the merge-base, or a checkpoint) through each later checkpoint, every line the
//! first-to-last diff adds is traced back to the step that wrote it, and every line it removes
//! forward to the step that removed it. A hunk belongs to the latest step among its lines.

use std::time::Instant;

use similar::DiffTag;

use crate::diff::{DiffHunk, lines, split};

/// The steps that added and removed lines, for a diff between a file's first and last
/// versions. Steps count from 1: step `i` turned version `i - 1` into version `i`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct History {
    /// For each line of the last version, the step that wrote it; `None` when it was there
    /// from the start.
    added_by: Vec<Option<usize>>,
    /// For each line of the first version, the step that removed it; `None` while it's there.
    removed_by: Vec<Option<usize>>,
}

/// The history of a file through `versions`, first to last. Past `deadline`, each step's diff
/// settles for a larger set of changes than the smallest, which can credit a line to a later
/// step than the one that wrote it, never to one that didn't change it. Versions are split
/// into lines as each step reaches them, so only two are split at a time.
pub fn history(versions: &[&[u8]], deadline: Instant) -> History {
    let Some(first) = versions.first() else {
        return History::default();
    };
    let mut before = split(first);
    let count = before.len();
    // Each line of the current version: the step that wrote it. Each line of the first: where
    // it is in the current version, while it's there.
    let mut added_by: Vec<Option<usize>> = vec![None; count];
    let mut at: Vec<Option<usize>> = (0..count).map(Some).collect();
    let mut removed_by: Vec<Option<usize>> = vec![None; count];
    for (step, version) in versions.iter().enumerate().skip(1) {
        let after = split(version);
        let d = lines(&before, &after, deadline);
        // Where each line of the version before lands in this one, if it's kept.
        let mut kept: Vec<Option<usize>> = vec![None; d.old_len()];
        let mut next = vec![Some(step); d.new_len()];
        for op in d.ops() {
            let (tag, old, new) = op.as_tag_tuple();
            if tag == DiffTag::Equal {
                for (o, n) in old.zip(new) {
                    kept[o] = Some(n);
                    next[n] = added_by[o];
                }
            }
        }
        added_by = next;
        drop(d);
        before = after;
        for (line, pos) in at.iter_mut().enumerate() {
            if let Some(p) = *pos {
                *pos = kept[p];
                if pos.is_none() {
                    removed_by[line] = Some(step);
                }
            }
        }
    }
    History {
        added_by,
        removed_by,
    }
}

impl History {
    /// The step that wrote line `line` (0-based) of the last version.
    pub fn added_by(&self, line: usize) -> Option<usize> {
        self.added_by.get(line).copied().flatten()
    }

    /// The step that removed line `line` (0-based) of the first version.
    pub fn removed_by(&self, line: usize) -> Option<usize> {
        self.removed_by.get(line).copied().flatten()
    }

    /// The latest step that changed any of `hunk`'s lines, for a hunk of the diff between the
    /// first and last versions.
    pub fn last_step(&self, hunk: &DiffHunk) -> Option<usize> {
        let added = hunk.added.iter().filter_map(|&l| self.added_by(l));
        let removed = hunk.removed.iter().filter_map(|&l| self.removed_by(l));
        added.chain(removed).max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff;
    use std::time::Duration;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    /// [`super::history`] of strings.
    fn history(versions: &[&str], deadline: Instant) -> History {
        let bytes: Vec<&[u8]> = versions.iter().map(|v| v.as_bytes()).collect();
        super::history(&bytes, deadline)
    }

    /// The last step of each hunk between the first and last of `versions`.
    fn steps(versions: &[&str]) -> Vec<Option<usize>> {
        let h = history(versions, soon());
        let d = diff(
            versions[0].as_bytes(),
            versions[versions.len() - 1].as_bytes(),
            soon(),
        );
        d.hunks.iter().map(|hunk| h.last_step(hunk)).collect()
    }

    fn numbered(lines: std::ops::RangeInclusive<u32>) -> String {
        lines.map(|n| format!("{n}\n")).collect()
    }

    /// `text` with its whole line `from` replaced by `to`.
    fn set(text: &str, from: &str, to: &str) -> String {
        text.lines()
            .map(|l| format!("{}\n", if l == from { to } else { l }))
            .collect()
    }

    #[test]
    fn lines_inserted_above_keep_the_step_that_wrote_them() {
        let v0 = "a\nb\n";
        let v1 = "x\na\nb\n";
        let v2 = "x\na\nb\ny\n";
        let h = history(&[v0, v1, v2], soon());
        assert_eq!(h.added_by(0), Some(1));
        assert_eq!((h.added_by(1), h.added_by(2)), (None, None));
        assert_eq!(h.added_by(3), Some(2));
    }

    #[test]
    fn a_line_rewritten_later_belongs_to_the_later_step() {
        let v0 = "a\n";
        let v1 = "a\nfirst draft\n";
        let v2 = "a\nsecond draft\n";
        let h = history(&[v0, v1, v2], soon());
        assert_eq!(h.added_by(1), Some(2));
        // Written in step 1 and gone by step 2: not in the diff at all.
        assert_eq!(steps(&[v0, v1, v2]), [Some(2)]);
    }

    #[test]
    fn a_removed_line_belongs_to_the_step_that_removed_it() {
        let v0 = "a\nb\nc\n";
        let v1 = "a\nb\nc\nd\n";
        let v2 = "a\nb\nd\n";
        let h = history(&[v0, v1, v2], soon());
        assert_eq!(h.removed_by(2), Some(2));
        assert_eq!((h.removed_by(0), h.removed_by(1)), (None, None));
        assert_eq!(h.added_by(2), Some(1));
    }

    #[test]
    fn changes_far_apart_each_keep_their_own_step() {
        let v0 = numbered(1..=30);
        let v1 = set(&v0, "2", "two");
        let v2 = set(&v1, "28", "twenty-eight");
        let v3 = v2.clone();
        assert_eq!(steps(&[&v0, &v1, &v2, &v3]), [Some(1), Some(2)]);
    }

    #[test]
    fn a_moved_line_belongs_to_the_step_that_moved_it() {
        let v0 = numbered(1..=10);
        let v1 = set(&v0, "1", "one");
        let mut v2: String = v1
            .lines()
            .filter(|l| *l != "5")
            .map(|l| format!("{l}\n"))
            .collect();
        v2.push_str("5\n");
        let h = history(&[&v0, &v1, &v2], soon());
        assert_eq!(h.removed_by(4), Some(2));
        assert_eq!(h.added_by(9), Some(2));
        assert_eq!(h.added_by(0), Some(1));
    }

    #[test]
    fn a_file_replaced_whole_belongs_to_the_step_that_replaced_it() {
        let v0 = numbered(1..=5);
        let v1 = set(&v0, "3", "three");
        let v2 = numbered(100..=110);
        let v3 = v2.clone();
        let h = history(&[&v0, &v1, &v2, &v3], soon());
        assert!((0..11).all(|l| h.added_by(l) == Some(2)));
        assert!([0, 1, 3, 4].iter().all(|&l| h.removed_by(l) == Some(2)));
        assert_eq!(h.removed_by(2), Some(1));
        assert_eq!(steps(&[&v0, &v1, &v2, &v3]), [Some(2)]);
    }

    #[test]
    fn a_file_that_didnt_exist_at_the_start_is_all_from_the_step_that_created_it() {
        let h = history(&["", "", "a\nb\n", "a\nb\nc\n"], soon());
        assert_eq!(
            (h.added_by(0), h.added_by(1), h.added_by(2)),
            (Some(2), Some(2), Some(3))
        );
        assert_eq!(steps(&["", "", "a\nb\n", "a\nb\nc\n"]), [Some(3)]);
    }

    #[test]
    fn one_version_has_no_steps() {
        let h = history(&["a\n"], soon());
        assert_eq!((h.added_by(0), h.removed_by(0)), (None, None));
    }
}
