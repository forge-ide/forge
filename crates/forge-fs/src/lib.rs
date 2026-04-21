#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]

use glob::Pattern;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

mod limits;
mod mutate;
mod tree;
pub use limits::Limits;
pub use mutate::{
    delete, edit, edit_preview, rename, write, write_bytes, write_preview, ApprovalPreview, FsError,
};
pub use tree::{
    list_tree, list_tree_gitignored, list_tree_gitignored_with_limit, list_tree_with_limit,
    NodeKind, TreeNode, DEFAULT_MAX_ENTRIES,
};

/// Internal entry points exposed solely for the `benches/mutate.rs` allocation
/// guard. Not part of the public API — do not depend on anything in here.
#[doc(hidden)]
pub mod __bench_internals {
    pub use crate::mutate::apply_unified_diff;
}

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
/// `canonicalize_no_symlink` — a lenient helper that accepts not-yet-existing
/// targets so a caller can `fs.write` a brand-new file.
///
/// Threat-model rationale: a read against a non-existent path can only ever
/// fail (there is nothing to read), and surfacing the failure as an explicit
/// `Io` error keeps the trust boundary loud. By contrast, `write` and `edit`
/// must support file creation — the path-traversal protection there comes from
/// the explicit symlink-component check in `canonicalize_no_symlink` plus
/// the `enforce_allowed` glob match against the resolved parent. Both entry
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

/// Shared helper: return the canonical form of `path`, rejecting any
/// user-visible symlink traversed *within* the path hierarchy. Used by
/// write, edit, rename, and delete to keep symlink rejection honest without
/// tripping over system-level redirects (e.g. macOS `/var` → `/private/var`)
/// that live above the workspace root.
///
/// **Lenient by design.** When `std::fs::canonicalize` fails (typically because
/// the leaf does not yet exist), this helper falls back to canonicalizing the
/// parent and joining the leaf name. This is what permits `fs.write` to create
/// new files — see the canonicalization-policy doc on [`mutate::write`] for
/// the full rationale and how this differs from [`read_file`]'s strict
/// canonicalization.
///
/// # Symlink policy
///
/// System-level symlinks that sit above the workspace (like macOS `/var` →
/// `/private/var`, which every temp-path resolves through) are transparent:
/// they add components at the front of the canonical form but don't rename
/// components at the same depth the user referenced. User-created symlinks
/// inside a workspace rename components at the same depth — we detect this
/// by aligning `path` and `canonical` component-by-component **from the
/// right** and flagging any mismatch before we hit `path`'s `RootDir`.
///
/// The real escape-prevention is `enforce_allowed` comparing the canonical
/// path to the canonical allowlist; this extra check is defense-in-depth
/// against benign-looking intra-workspace symlinks.
pub(crate) fn canonicalize_no_symlink(path: &Path) -> std::result::Result<PathBuf, FsError> {
    let canonical = match std::fs::canonicalize(path) {
        Ok(p) => p,
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
            canonical_parent.join(name)
        }
    };

    let input: Vec<_> = path.components().collect();
    let canonical_comps: Vec<_> = canonical.components().collect();
    if canonical_comps.len() >= input.len() {
        let offset = canonical_comps.len() - input.len();
        for (i, comp) in input.iter().enumerate().rev() {
            if matches!(
                comp,
                std::path::Component::RootDir | std::path::Component::Prefix(_)
            ) {
                break;
            }
            if canonical_comps[i + offset] != *comp {
                return Err(FsError::SymlinkDenied {
                    path: path.to_path_buf(),
                });
            }
        }
    }

    Ok(canonical)
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
