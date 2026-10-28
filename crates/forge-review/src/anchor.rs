//! Where a comment thread's lines are now. A thread is written on a range of a file at one
//! checkpoint; at a later one the range follows the lines if every one of them is still there,
//! unchanged and still together. Otherwise, if one changed, went away, or had a line put
//! between it and the next, the thread is outdated.

use std::time::Instant;

use similar::DiffTag;

use crate::diff::{display, lines, split};

/// The most lines a quote holds; past them it ends with a note of how many more there are.
pub const MAX_QUOTE: usize = 100;

/// Where a thread's lines went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Followed {
    /// Still there, together and unchanged, at these 1-based, inclusive lines.
    At(u32, u32),
    /// Changed or gone, or the range wasn't in the file.
    Gone,
    /// The diff ran out of time before it could tell; try again later.
    Unsure,
}

/// The 1-based, inclusive range `start..=end` of `old`, where it is in `new`.
pub fn follow(start: u32, end: u32, old: &[u8], new: &[u8], deadline: Instant) -> Followed {
    line_map(old, new, deadline).follow(start, end)
}

/// Where each line of one version landed in another, from one diff, to follow any number of
/// ranges with.
#[derive(Clone, Debug)]
pub struct LineMap {
    /// For each old line (0-based), the new line it's kept as.
    at: Vec<Option<usize>>,
    /// The diff ran past its deadline.
    late: bool,
}

/// The [`LineMap`] from `old` to `new`.
pub fn line_map(old: &[u8], new: &[u8], deadline: Instant) -> LineMap {
    let (old, new) = (split(old), split(new));
    let mut at = vec![None; old.len()];
    for op in lines(&old, &new, deadline).ops() {
        let (tag, o, n) = op.as_tag_tuple();
        if tag == DiffTag::Equal {
            for (o, n) in o.zip(n) {
                at[o] = Some(n);
            }
        }
    }
    LineMap {
        at,
        late: Instant::now() >= deadline,
    }
}

impl LineMap {
    /// Where the 1-based, inclusive range `start..=end` of the old version is in the new one.
    pub fn follow(&self, start: u32, end: u32) -> Followed {
        match self.place(start, end) {
            Some((start, end)) => Followed::At(start, end),
            // Past its deadline a diff settles for more changes than there are, so lines it
            // calls changed may not be. Lines it calls kept always are.
            None if self.late => Followed::Unsure,
            None => Followed::Gone,
        }
    }

    fn place(&self, start: u32, end: u32) -> Option<(u32, u32)> {
        let (first, last) = (start.checked_sub(1)? as usize, end.checked_sub(1)? as usize);
        if first > last || last >= self.at.len() {
            return None;
        }
        let begin = self.at[first]?;
        let together = (first..=last)
            .enumerate()
            .all(|(i, o)| self.at[o] == Some(begin + i));
        together.then(|| (begin as u32 + 1, (begin + last - first + 1) as u32))
    }
}

/// The lines `start..=end` (1-based, inclusive) of `text`, as the reviewer sees them; fewer
/// when the range runs past its end. Past [`MAX_QUOTE`] lines, the rest is a note of how many.
pub fn quote(text: &[u8], start: u32, end: u32) -> Vec<String> {
    let (Some(first), Some(last)) = (start.checked_sub(1), end.checked_sub(1)) else {
        return Vec::new();
    };
    let lines = split(text);
    let range: Vec<&[u8]> = lines
        .iter()
        .skip(first as usize)
        .take((last + 1).saturating_sub(first) as usize)
        .copied()
        .collect();
    let mut quoted: Vec<String> = range
        .iter()
        .take(MAX_QUOTE)
        .map(|line| display(line.strip_suffix(b"\n").unwrap_or(line)))
        .collect();
    if range.len() > MAX_QUOTE {
        quoted.push(format!("\u{2026} ({} more lines)", range.len() - MAX_QUOTE));
    }
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn soon() -> Instant {
        Instant::now() + Duration::from_secs(5)
    }

    fn numbered(n: u32) -> String {
        (1..=n).map(|i| format!("{i}\n")).collect()
    }

    fn follow(start: u32, end: u32, old: &str, new: &str) -> Option<(u32, u32)> {
        match super::follow(start, end, old.as_bytes(), new.as_bytes(), soon()) {
            Followed::At(start, end) => Some((start, end)),
            Followed::Gone => None,
            Followed::Unsure => panic!("unsure with time to spare"),
        }
    }

    #[test]
    fn a_range_follows_lines_added_or_removed_elsewhere() {
        let old = numbered(10);
        assert_eq!(follow(4, 6, &old, &old), Some((4, 6)));
        let above = format!("new\nnew\n{old}");
        assert_eq!(follow(4, 6, &old, &above), Some((6, 8)));
        let below = format!("{old}more\n");
        assert_eq!(follow(4, 6, &old, &below), Some((4, 6)));
        let fewer: String = old.lines().skip(2).map(|l| format!("{l}\n")).collect();
        assert_eq!(follow(4, 6, &old, &fewer), Some((2, 4)));
    }

    #[test]
    fn a_change_inside_the_range_or_a_line_put_into_it_makes_it_outdated() {
        let old = numbered(10);
        let edited = old.replace("5\n", "five\n");
        assert_eq!(follow(4, 6, &old, &edited), None);
        let inserted = old.replace("5\n", "5\ninserted\n");
        assert_eq!(follow(4, 6, &old, &inserted), None);
        let removed = old.replace("6\n", "");
        assert_eq!(follow(4, 6, &old, &removed), None);
        assert_eq!(follow(4, 6, &old, ""), None);
        // Just beside the range is fine.
        let beside = old.replace("7\n", "seven\n");
        assert_eq!(follow(4, 6, &old, &beside), Some((4, 6)));
    }

    #[test]
    fn a_range_that_isnt_in_the_file_has_nowhere_to_go() {
        let old = numbered(3);
        assert_eq!(follow(3, 4, &old, &old), None);
        assert_eq!(follow(0, 1, &old, &old), None);
        assert_eq!(follow(2, 1, &old, &old), None);
    }

    #[test]
    fn one_line_map_follows_many_ranges() {
        let old = numbered(10);
        let new = format!("new\n{}", old.replace("5\n", "five\n"));
        let map = line_map(old.as_bytes(), new.as_bytes(), soon());
        assert_eq!(map.follow(1, 2), Followed::At(2, 3));
        assert_eq!(map.follow(4, 6), Followed::Gone);
        assert_eq!(map.follow(9, 10), Followed::At(10, 11));
        assert_eq!(map.follow(10, 11), Followed::Gone);
    }

    #[test]
    fn a_diff_out_of_time_is_unsure_rather_than_gone() {
        // Every line differs, in an order no small edit script explains, but line 1 is kept.
        let old: String = (0..20_000)
            .map(|n| format!("{}\n", n * 7 % 20_000))
            .collect();
        let mut new: String = (0..20_000)
            .map(|n| format!("{}\n", n * 13 % 20_000))
            .collect();
        new.insert_str(0, "0\n");
        let late = Instant::now() - Duration::from_secs(1);
        assert_ne!(
            super::follow(5, 5, old.as_bytes(), new.as_bytes(), late),
            Followed::Gone
        );
    }

    #[test]
    fn a_long_quote_ends_with_how_much_more_there_is() {
        let text: String = (1..=250).map(|n| format!("{n}\n")).collect();
        let quoted = quote(text.as_bytes(), 1, 250);
        assert_eq!(quoted.len(), MAX_QUOTE + 1);
        assert_eq!(quoted[99], "100");
        assert_eq!(quoted[100], "\u{2026} (150 more lines)");
        assert_eq!(quote(text.as_bytes(), 1, 100).len(), 100);
    }

    #[test]
    fn a_quote_is_the_lines_as_shown() {
        let text = b"a\nb\xff\nc\nd";
        assert_eq!(quote(text, 2, 4), ["b\u{FFFD}", "c", "d"]);
        assert_eq!(quote(text, 4, 9), ["d"]);
        assert!(quote(text, 0, 1).is_empty());
    }
}
