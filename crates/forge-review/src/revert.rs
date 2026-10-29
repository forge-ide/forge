//! Reverting one hunk of a diff: the new version with that hunk's lines put back as the old
//! version had them, every other change kept.

use crate::diff::{DiffHunk, split};

/// `new` with `hunk`, a hunk of the diff from `old` to `new`, undone. Within a hunk the
/// context lines are the same bytes on both sides, so the hunk's span of `new` is replaced by
/// its span of `old`, line breaks (and a last line without one) included.
pub fn revert(old: &[u8], new: &[u8], hunk: &DiffHunk) -> Vec<u8> {
    let (old, new) = (split(old), split(new));
    let (old_at, new_at) = (
        begin(hunk.old_start, hunk.old_lines),
        begin(hunk.new_start, hunk.new_lines),
    );
    let old_end = (old_at + hunk.old_lines as usize).min(old.len());
    let new_end = (new_at + hunk.new_lines as usize).min(new.len());
    let mut out = Vec::new();
    for line in new[..new_at.min(new.len())]
        .iter()
        .chain(&old[old_at.min(old_end)..old_end])
        .chain(&new[new_end..])
    {
        out.extend_from_slice(line);
    }
    out
}

/// The 0-based index a side's span starts at, from its unified start: 1-based, or the line
/// before it when the span is empty.
fn begin(start: u32, lines: u32) -> usize {
    if lines == 0 {
        start as usize
    } else {
        start.saturating_sub(1) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff;
    use std::time::{Duration, Instant};

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    fn numbered(n: u32) -> String {
        (1..=n).map(|i| format!("{i}\n")).collect()
    }

    /// Each hunk of the diff from `old` to `new`, reverted on its own.
    fn reverted(old: &str, new: &str) -> Vec<String> {
        let d = diff(old.as_bytes(), new.as_bytes(), soon());
        d.hunks
            .iter()
            .map(|h| String::from_utf8(revert(old.as_bytes(), new.as_bytes(), h)).unwrap())
            .collect()
    }

    #[test]
    fn reverting_a_hunk_undoes_only_that_hunk() {
        // Line `n` replaced by `edit(n)`.
        let edited = |edit: &dyn Fn(u32) -> String| (1..=30).map(edit).collect::<String>();
        let old = numbered(30);
        let first = |n| {
            if n == 3 {
                "three\n".into()
            } else {
                format!("{n}\n")
            }
        };
        let second = |n| {
            if n == 25 {
                "25\nextra\n".into()
            } else {
                format!("{n}\n")
            }
        };
        let new = edited(&|n| if n == 3 { first(n) } else { second(n) });
        let each = reverted(&old, &new);
        assert_eq!(each.len(), 2);
        assert_eq!(each[0], edited(&second));
        assert_eq!(each[1], edited(&first));
        // What's left after one revert is exactly the other hunk.
        let rest = diff(old.as_bytes(), each[0].as_bytes(), soon());
        assert_eq!(rest.hunks.len(), 1);
        assert_eq!(rest.additions, 1);
        assert_eq!(rest.deletions, 0);
    }

    #[test]
    fn additions_and_deletions_alone_revert() {
        let old = numbered(10);
        let added = format!("top\n{old}");
        assert_eq!(reverted(&old, &added), [old.as_str()]);
        let removed = old.replace("\n5\n", "\n");
        assert_eq!(reverted(&old, &removed), [old.as_str()]);
        let appended = format!("{old}end\n");
        assert_eq!(reverted(&old, &appended), [old.as_str()]);
    }

    #[test]
    fn a_whole_new_or_deleted_file_reverts_to_the_other_side() {
        assert_eq!(reverted("", "a\nb\n"), [""]);
        assert_eq!(reverted("a\nb\n", ""), ["a\nb\n"]);
    }

    #[test]
    fn a_missing_line_break_at_the_end_comes_back_as_it_was() {
        assert_eq!(reverted("a\nb", "a\nb\nc\n"), ["a\nb"]);
        assert_eq!(reverted("a\nb\n", "a\nb"), ["a\nb\n"]);
        assert_eq!(reverted("x\ny", "x\nz"), ["x\ny"]);
    }

    #[test]
    fn bytes_that_arent_text_come_back_exactly() {
        let old = b"a\n\xff\xfe\0\nc\r\n".to_vec();
        let new = b"a\nb\nc\r\n".to_vec();
        let d = diff(&old, &new, soon());
        assert_eq!(revert(&old, &new, &d.hunks[0]), old);
    }
}
