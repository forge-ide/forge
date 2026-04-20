//! `fs.write` and `fs.edit` — atomic path-validated file mutation with unified
//! diff previews. Shared conventions:
//!
//! - Paths are canonicalized; symlinks anywhere in the path chain are rejected.
//! - Writes refuse to create parent directories implicitly.
//! - Edits require the target file to exist and apply a unified-diff patch.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::{canonicalize_no_symlink, enforce_allowed, Limits};

/// Error variants for mutation operations. Matches the DoD vocabulary.
#[derive(Debug, thiserror::Error)]
pub enum FsError {
    #[error("path '{}' is not allowed by allowed_paths", path.display())]
    PathDenied { path: PathBuf },
    #[error("path '{}' traverses a symlink", path.display())]
    SymlinkDenied { path: PathBuf },
    #[error("parent directory of '{}' does not exist", path.display())]
    ParentMissing { path: PathBuf },
    #[error("target file '{}' does not exist", path.display())]
    TargetMissing { path: PathBuf },
    #[error("malformed unified-diff patch: {reason}")]
    MalformedPatch { reason: String },
    #[error("'{}' is {actual} bytes, exceeds limit of {limit}", path.display())]
    TooLarge {
        path: PathBuf,
        actual: u64,
        limit: u64,
    },
    #[error("io error on '{}': {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Minimal approval preview payload for mutation ops. Renderable by the UI
/// layer (F-027) without depending on this crate.
#[derive(Debug, Clone, PartialEq)]
pub struct ApprovalPreview {
    pub description: String,
}

/// Atomically write `content` to `path` after validating the path and
/// rejecting payloads larger than `limits.max_write_bytes`.
///
/// Steps: size cap → reject symlinks → canonicalize → enforce `allowed_paths`
/// glob → refuse if the parent dir is missing → write `content` to a sibling
/// `NamedTempFile` → `persist` (atomic rename on the same filesystem).
///
/// # Canonicalization policy (write vs read asymmetry)
///
/// Unlike [`crate::read_file`], which uses strict [`std::fs::canonicalize`],
/// `write` uses `canonicalize_no_symlink` — a lenient helper that
/// canonicalizes the parent directory and joins the file name, so a
/// not-yet-existing leaf is permitted. This is required for the
/// file-creation case; without it `fs.write` could only ever overwrite
/// existing files. Symlink rejection is preserved by walking every path
/// component and rejecting any that is itself a symlink, before joining the
/// new leaf onto the canonical parent. The trust boundary remains loud:
/// `enforce_allowed` glob-matches the resolved path, so `..` traversal is
/// still rejected — only the "leaf must exist" requirement is relaxed.
pub fn write(
    path: &str,
    content: &str,
    allowed_paths: &[String],
    limits: &Limits,
) -> Result<(), FsError> {
    let input = Path::new(path);
    let actual = content.len() as u64;
    if actual > limits.max_write_bytes {
        return Err(FsError::TooLarge {
            path: input.to_path_buf(),
            actual,
            limit: limits.max_write_bytes,
        });
    }
    let canonical = canonicalize_no_symlink(input)?;
    enforce_allowed(&canonical, allowed_paths)?;

    let parent = canonical.parent().ok_or_else(|| FsError::ParentMissing {
        path: canonical.clone(),
    })?;
    if !parent.is_dir() {
        return Err(FsError::ParentMissing {
            path: canonical.clone(),
        });
    }

    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| FsError::Io {
        path: canonical.clone(),
        source: e,
    })?;
    tmp.write_all(content.as_bytes()).map_err(|e| FsError::Io {
        path: canonical.clone(),
        source: e,
    })?;
    tmp.as_file_mut().sync_all().map_err(|e| FsError::Io {
        path: canonical.clone(),
        source: e,
    })?;
    tmp.persist(&canonical).map_err(|e| FsError::Io {
        path: canonical.clone(),
        source: e.error,
    })?;
    Ok(())
}

/// Apply a unified-diff `patch` to the file at `path` after validating the
/// path. Rejects source files larger than `limits.max_read_bytes` before
/// reading them into RAM, and delegates post-patch size enforcement to
/// [`write()`]. Writes atomically. Requires the target to exist.
///
/// # Canonicalization policy (edit vs read asymmetry)
///
/// `edit` uses the same lenient `canonicalize_no_symlink` as
/// [`write()`] (not the strict [`std::fs::canonicalize`] used by
/// [`crate::read_file`]) so the helper is shared across both mutation paths.
/// `edit` then explicitly re-asserts that the canonical target *is* a file
/// via the `canonical.is_file()` check below — distinct from the read path,
/// which fails earlier in canonicalization itself. The threat model is the
/// same as `write`: symlink components are rejected pre-canonicalization,
/// and `enforce_allowed` glob-matches the resolved path.
pub fn edit(
    path: &str,
    patch: &str,
    allowed_paths: &[String],
    limits: &Limits,
) -> Result<(), FsError> {
    let input = Path::new(path);
    let canonical = canonicalize_no_symlink(input)?;
    enforce_allowed(&canonical, allowed_paths)?;

    if !canonical.is_file() {
        return Err(FsError::TargetMissing {
            path: canonical.clone(),
        });
    }

    let metadata = std::fs::metadata(&canonical).map_err(|e| FsError::Io {
        path: canonical.clone(),
        source: e,
    })?;
    let actual = metadata.len();
    if actual > limits.max_read_bytes {
        return Err(FsError::TooLarge {
            path: canonical,
            actual,
            limit: limits.max_read_bytes,
        });
    }

    let original = std::fs::read_to_string(&canonical).map_err(|e| FsError::Io {
        path: canonical.clone(),
        source: e,
    })?;
    let updated = apply_unified_diff(&original, patch)?;

    // Reuse atomic write (skipping re-canonicalization by round-tripping the string).
    let canonical_str = canonical.to_str().ok_or_else(|| FsError::Io {
        path: canonical.clone(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, "non-utf8 path"),
    })?;
    write(canonical_str, &updated, allowed_paths, limits)
}

/// `ApprovalPreview` for a prospective write. Returns a content-only preview
/// of the proposed new bytes — does **not** read the existing file.
///
/// Reading the target file here would run before `allowed_paths` enforcement
/// (which happens in [`write()`]) and leak attacker-chosen file contents into
/// the approval event (see F-042 / audit finding H3). The proposed `content`
/// is already supplied by the caller, so echoing it back does not expand the
/// trust surface. If a before/after diff is needed for UX, compute it via an
/// explicit `fs.read` call that goes through the same approval gate.
pub fn write_preview(path: &str, content: &str) -> ApprovalPreview {
    ApprovalPreview {
        description: format!("Write file {path} ({} bytes)\n{content}", content.len()),
    }
}

/// `ApprovalPreview` for a prospective edit. The patch itself is the unified
/// diff; we just echo it back with a header.
pub fn edit_preview(path: &str, patch: &str) -> ApprovalPreview {
    ApprovalPreview {
        description: format!("Edit file {path}\n{patch}"),
    }
}

/// Minimal unified-diff applier. Supports standard `@@ -a,b +c,d @@` hunks
/// with `' '`, `'-'`, `'+'` line prefixes. Rejects anything else as
/// [`FsError::MalformedPatch`].
///
/// Perf: writes every emitted line directly into a single `String` buffer
/// pre-sized to the original file length. Context/removed-verified lines are
/// appended as borrowed slices (no per-line `String` allocation), and `+`
/// additions only pay for the one terminating `'\n'` byte on top of the slice
/// copy. Net effect: allocation count is O(1) in the number of lines, dominated
/// by the single `with_capacity` up-front plus any overflow growth when
/// additions exceed the original size. See `benches/mutate.rs` for the
/// ≥5× allocation-reduction guard.
#[doc(hidden)]
pub fn apply_unified_diff(original: &str, patch: &str) -> Result<String, FsError> {
    let src_lines: Vec<&str> = split_preserving_newline(original);
    // Pre-size to the original file: most edits are localized, so this is a
    // tight upper bound for context + removed lines. Additions grow the buffer
    // via normal `String` doubling; still amortized O(1) per push.
    let mut out = String::with_capacity(original.len());
    let mut cursor = 0usize; // index into src_lines
    let mut saw_hunk = false;
    let mut lines = patch.lines().peekable();

    while let Some(raw) = lines.next() {
        if raw.starts_with("--- ") || raw.starts_with("+++ ") {
            continue; // file headers are optional / ignored
        }
        if let Some(rest) = raw.strip_prefix("@@ ") {
            saw_hunk = true;
            let old_start = parse_hunk_old_start(rest)?;
            // Copy lines from cursor up to hunk start (1-indexed → 0-indexed).
            let target = old_start.saturating_sub(1);
            if target < cursor || target > src_lines.len() {
                return Err(FsError::MalformedPatch {
                    reason: format!("hunk range out of bounds at line {old_start}"),
                });
            }
            for l in &src_lines[cursor..target] {
                out.push_str(l);
            }
            cursor = target;

            // Process hunk body.
            while let Some(peek) = lines.peek() {
                if peek.starts_with("@@ ") {
                    break;
                }
                let body = lines.next().unwrap();
                if let Some(ctx) = body.strip_prefix(' ') {
                    let src = src_lines
                        .get(cursor)
                        .ok_or_else(|| FsError::MalformedPatch {
                            reason: "context line past end of source".into(),
                        })?;
                    if src.trim_end_matches('\n') != ctx.trim_end_matches('\n') {
                        return Err(FsError::MalformedPatch {
                            reason: format!("context mismatch at line {}", cursor + 1),
                        });
                    }
                    out.push_str(src);
                    cursor += 1;
                } else if let Some(removed) = body.strip_prefix('-') {
                    let src = src_lines
                        .get(cursor)
                        .ok_or_else(|| FsError::MalformedPatch {
                            reason: "delete past end of source".into(),
                        })?;
                    if src.trim_end_matches('\n') != removed.trim_end_matches('\n') {
                        return Err(FsError::MalformedPatch {
                            reason: format!("delete mismatch at line {}", cursor + 1),
                        });
                    }
                    cursor += 1;
                } else if let Some(added) = body.strip_prefix('+') {
                    // Preserve newline behavior: input lines are stored without
                    // their trailing '\n' by split_preserving_newline; re-add
                    // unless this was a "\ No newline at end of file" marker.
                    out.push_str(added);
                    out.push('\n');
                } else if body.starts_with("\\ ") {
                    // "\ No newline at end of file" — strip trailing newline
                    // from the buffer if present.
                    if out.ends_with('\n') {
                        out.pop();
                    }
                } else if body.is_empty() {
                    // Some diff tools emit a bare empty line as a zero-width context line.
                    let src = src_lines
                        .get(cursor)
                        .ok_or_else(|| FsError::MalformedPatch {
                            reason: "empty context past end of source".into(),
                        })?;
                    if !src.trim_end_matches('\n').is_empty() {
                        return Err(FsError::MalformedPatch {
                            reason: format!("empty context mismatch at line {}", cursor + 1),
                        });
                    }
                    out.push_str(src);
                    cursor += 1;
                } else {
                    return Err(FsError::MalformedPatch {
                        reason: format!("unrecognized line prefix: {body:?}"),
                    });
                }
            }
        } else {
            // Anything outside a recognized header before the first hunk is noise;
            // anything after the first hunk without being captured is a format error.
            if saw_hunk {
                return Err(FsError::MalformedPatch {
                    reason: format!("unexpected line outside hunk: {raw:?}"),
                });
            }
        }
    }

    if !saw_hunk {
        return Err(FsError::MalformedPatch {
            reason: "no hunks found".into(),
        });
    }

    // Append any source lines remaining after the last hunk.
    for l in &src_lines[cursor..] {
        out.push_str(l);
    }

    Ok(out)
}

/// Split `s` into lines while preserving the trailing newline on each line so
/// concatenation round-trips exactly. A final line without a terminator yields
/// a line without `'\n'`.
fn split_preserving_newline(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if *b == b'\n' {
            out.push(&s[start..=i]);
            start = i + 1;
        }
    }
    if start < bytes.len() {
        out.push(&s[start..]);
    }
    out
}

fn parse_hunk_old_start(rest: &str) -> Result<usize, FsError> {
    // rest begins like "-a,b +c,d @@ optional_trailing"
    let minus = rest
        .split_whitespace()
        .find(|t| t.starts_with('-'))
        .ok_or_else(|| FsError::MalformedPatch {
            reason: format!("hunk header missing '-' range: {rest:?}"),
        })?;
    let spec = minus.trim_start_matches('-');
    let (start_str, _count_str) = spec.split_once(',').unwrap_or((spec, "1"));
    start_str
        .parse::<usize>()
        .map_err(|_| FsError::MalformedPatch {
            reason: format!("hunk start not a number: {start_str:?}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar::TextDiff;

    #[test]
    fn split_preserves_trailing_newlines() {
        let lines = split_preserving_newline("a\nb\nc");
        assert_eq!(lines, vec!["a\n", "b\n", "c"]);
    }

    #[test]
    fn apply_unified_diff_edits_middle_line() {
        let original = "alpha\nbeta\ngamma\n";
        let updated = "alpha\nBETA\ngamma\n";
        let patch = TextDiff::from_lines(original, updated)
            .unified_diff()
            .to_string();
        let result = apply_unified_diff(original, &patch).unwrap();
        assert_eq!(result, updated);
    }

    #[test]
    fn apply_unified_diff_rejects_non_patch_input() {
        let err = apply_unified_diff("a\n", "garbage\n").unwrap_err();
        assert!(matches!(err, FsError::MalformedPatch { .. }));
    }
}
