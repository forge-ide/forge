#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]

use glob::Pattern;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

mod limits;
mod mutate;
pub use limits::Limits;
pub use mutate::{edit, edit_preview, write, write_preview, ApprovalPreview, FsError};

/// Result of a successful `fs.read` operation.
#[derive(Debug)]
pub struct ReadResult {
    pub content: String,
    pub bytes: usize,
    pub sha256: String,
}

/// Read a file at `path`, validating it against `allowed_paths` glob patterns
/// and rejecting files larger than `limits.max_read_bytes` *before* allocating
/// the read buffer.
///
/// The path is canonicalized before glob matching to prevent `..` traversal.
///
/// # Canonicalization policy (read vs write/edit asymmetry)
///
/// `read_file` uses **strict** [`std::fs::canonicalize`], which fails when any
/// component of `path` is missing or a broken symlink. This is intentional and
/// asymmetric with [`mutate::write`] / [`mutate::edit`], which use
/// [`canonicalize_no_symlink`] — a lenient helper that accepts not-yet-existing
/// targets so a caller can `fs.write` a brand-new file.
///
/// Threat-model rationale: a read against a non-existent path can only ever
/// fail (there is nothing to read), and surfacing the failure as an explicit
/// `Io` error keeps the trust boundary loud. By contrast, `write` and `edit`
/// must support file creation — the path-traversal protection there comes from
/// the explicit symlink-component check in [`canonicalize_no_symlink`] plus
/// the [`enforce_allowed`] glob match against the resolved parent. Both entry
/// points still reject `..` traversal because the canonical form is what gets
/// glob-matched; the asymmetry is purely about whether the leaf must exist.
pub fn read_file(
    path: &str,
    allowed_paths: &[String],
    limits: &Limits,
) -> Result<ReadResult, FsError> {
    let canonical = std::fs::canonicalize(path).map_err(|e| FsError::Io {
        path: PathBuf::from(path),
        source: e,
    })?;
    enforce_allowed(&canonical, allowed_paths)?;

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

    let raw = std::fs::read(&canonical).map_err(|e| FsError::Io {
        path: canonical.clone(),
        source: e,
    })?;
    let bytes = raw.len();
    let content = String::from_utf8_lossy(&raw).into_owned();
    let sha256 = hex::encode(Sha256::digest(&raw));
    Ok(ReadResult {
        content,
        bytes,
        sha256,
    })
}

/// Shared helper: return the canonical form of `path` only if every component is
/// symlink-free. Used by both write and edit to enforce symlink rejection even
/// when the enclosing directory is allowed.
///
/// **Lenient by design.** When `std::fs::canonicalize` fails (typically because
/// the leaf does not yet exist), this helper falls back to canonicalizing the
/// parent and joining the leaf name. This is what permits `fs.write` to create
/// new files — see the canonicalization-policy doc on [`mutate::write`] for
/// the full rationale and how this differs from [`read_file`]'s strict
/// canonicalization.
pub(crate) fn canonicalize_no_symlink(path: &Path) -> std::result::Result<PathBuf, FsError> {
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
