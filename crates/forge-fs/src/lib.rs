use anyhow::{bail, Context, Result};
use glob::Pattern;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

mod mutate;
pub use mutate::{edit, edit_preview, write, write_preview, ApprovalPreview, FsError};

/// Result of a successful `fs.read` operation.
#[derive(Debug)]
pub struct ReadResult {
    pub content: String,
    pub bytes: usize,
    pub sha256: String,
}

/// Read a file at `path`, validating it against the `allowed_paths` glob patterns.
///
/// The path is canonicalized before glob matching to prevent `..` traversal attacks.
/// Returns `Err` if no pattern matches or the file cannot be read.
pub fn read_file(path: &str, allowed_paths: &[String]) -> Result<ReadResult> {
    // Canonicalize first so `..` components can't bypass glob patterns.
    let canonical =
        std::fs::canonicalize(path).with_context(|| format!("cannot resolve path '{path}'"))?;
    validate_against_globs(&canonical, allowed_paths)?;
    let raw = std::fs::read(&canonical)?;
    let bytes = raw.len();
    let content = String::from_utf8_lossy(&raw).into_owned();
    let sha256 = hex::encode(Sha256::digest(&raw));
    Ok(ReadResult {
        content,
        bytes,
        sha256,
    })
}

fn validate_against_globs(path: &Path, allowed_paths: &[String]) -> Result<()> {
    for pattern in allowed_paths {
        if let Ok(pat) = Pattern::new(pattern) {
            if pat.matches_path(path) {
                return Ok(());
            }
        }
    }
    bail!("path '{}' is not allowed by allowed_paths", path.display());
}

/// Shared helper: return the canonical form of `path` only if every component is
/// symlink-free. Used by both write and edit to enforce symlink rejection even
/// when the enclosing directory is allowed.
pub(crate) fn canonicalize_no_symlink(path: &Path) -> std::result::Result<PathBuf, FsError> {
    // Walk each ancestor; if any ancestor is a symlink, reject.
    // The target itself may or may not exist yet (write creates it).
    let mut cursor = PathBuf::new();
    for comp in path.components() {
        cursor.push(comp);
        if let Ok(md) = std::fs::symlink_metadata(&cursor) {
            if md.file_type().is_symlink() {
                return Err(FsError::SymlinkDenied {
                    path: path.to_path_buf(),
                });
            }
        }
    }
    // Prefer canonicalizing the parent (always exists for write; target itself for edit).
    // If canonicalize succeeds on the full path, use it; otherwise canonicalize parent and
    // append the file name.
    match std::fs::canonicalize(path) {
        Ok(p) => Ok(p),
        Err(_) => {
            let parent = path.parent().ok_or_else(|| FsError::ParentMissing {
                path: path.to_path_buf(),
            })?;
            let canonical_parent =
                std::fs::canonicalize(parent).map_err(|_| FsError::ParentMissing {
                    path: path.to_path_buf(),
                })?;
            let name = path.file_name().ok_or_else(|| FsError::ParentMissing {
                path: path.to_path_buf(),
            })?;
            Ok(canonical_parent.join(name))
        }
    }
}

pub(crate) fn enforce_allowed(
    path: &Path,
    allowed_paths: &[String],
) -> std::result::Result<(), FsError> {
    for pattern in allowed_paths {
        if let Ok(pat) = Pattern::new(pattern) {
            if pat.matches_path(path) {
                return Ok(());
            }
        }
    }
    Err(FsError::PathDenied {
        path: path.to_path_buf(),
    })
}
