#![deny(rustdoc::broken_intra_doc_links, rustdoc::private_intra_doc_links)]

use glob::Pattern;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

mod limits;
mod mutate;
mod tree;
pub use limits::Limits;
pub use mutate::{delete, edit, edit_preview, rename, write, write_bytes, write_preview, FsError};
pub use tree::{
    list_tree, list_tree_gitignored, list_tree_gitignored_with_limit, list_tree_with_limit,
    NodeKind, TreeNode, TreeStats, DEFAULT_MAX_ENTRIES,
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

    // F-356: normalize `.`/`..` in the caller-supplied path before the
    // right-aligned compare. `Path::components()` already drops `CurDir` for
    // non-leading occurrences, but `ParentDir` survives iteration and used to
    // inflate the input length past the canonical count — which skipped the
    // symlink guard entirely for any input containing `..`. Fold `ParentDir`
    // against the prior `Normal` component so the check runs on a semantically
    // equivalent, length-matched view of the input.
    let normalized_input = normalize_components(path);
    let canonical_comps: Vec<_> = canonical.components().collect();
    if canonical_comps.len() >= normalized_input.len() {
        let offset = canonical_comps.len() - normalized_input.len();
        for (i, comp) in normalized_input.iter().enumerate().rev() {
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

/// Returns a component list equivalent to `path.components()` with `CurDir`
/// dropped and `ParentDir` folded against the preceding `Normal` entry. Root
/// and prefix components are preserved so absolute-vs-relative intent is kept
/// intact; `ParentDir` immediately above a `RootDir`/`Prefix` is dropped (you
/// can't go above the root), so the returned vector never inflates past the
/// canonical form and the right-aligned compare in `canonicalize_no_symlink`
/// always runs its symlink guard.
fn normalize_components(path: &Path) -> Vec<std::path::Component<'_>> {
    use std::path::Component;
    let mut out: Vec<Component<'_>> = Vec::new();
    for comp in path.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => match out.last() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => { /* drop — can't go above root */
                }
                _ => out.push(comp),
            },
            _ => out.push(comp),
        }
    }
    out
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

#[cfg(test)]
mod canonicalize_no_symlink_tests {
    //! F-356: `canonicalize_no_symlink` right-aligns `path.components()` against
    //! `canonical.components()` to catch user-space symlink substitution. The
    //! original implementation skipped that compare whenever the component
    //! counts disagreed — which happens for any caller path containing `.` or
    //! `..` (those inflate `path.components().len()` without appearing in the
    //! canonical form). That silently bypassed the symlink guard for
    //! symlink-through-dot and symlink-through-parent-dir inputs, leaving the
    //! defense-in-depth narrative in the module docstring technically inert.
    //!
    //! These tests nail down the matrix called out in the Definition of Done:
    //! `.`, `..`, symlink-in-the-middle, symlink-at-the-leaf — each with and
    //! without the sandbox boundary being crossed. The `enforce_allowed` layer
    //! that runs one frame up in the write/edit/rename/delete wrappers is
    //! orthogonal; these tests focus on the symlink-component decision this
    //! helper owns.
    use super::*;

    // ------------------------------------------------------------------
    // Dot/parent-dir components without any symlink — must be accepted.
    // ------------------------------------------------------------------

    #[test]
    fn accepts_path_with_cur_dir_component() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(real_dir.join("a").join("b")).unwrap();
        std::fs::File::create(real_dir.join("a").join("b").join("c.txt")).unwrap();

        let ambiguous = real_dir.join("a").join(".").join("b").join("c.txt");
        let got = canonicalize_no_symlink(&ambiguous).expect("dot component must be accepted");

        assert_eq!(got, real_dir.join("a").join("b").join("c.txt"));
    }

    #[test]
    fn accepts_path_with_parent_dir_component() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(real_dir.join("a").join("x")).unwrap();
        std::fs::create_dir_all(real_dir.join("a").join("b")).unwrap();
        std::fs::File::create(real_dir.join("a").join("b").join("c.txt")).unwrap();

        let ambiguous = real_dir
            .join("a")
            .join("x")
            .join("..")
            .join("b")
            .join("c.txt");
        let got =
            canonicalize_no_symlink(&ambiguous).expect("parent-dir component must be accepted");

        assert_eq!(got, real_dir.join("a").join("b").join("c.txt"));
    }

    #[test]
    fn accepts_path_with_cur_dir_on_nonexistent_leaf() {
        // Lenient-fallback path: `a/./b/new.txt` where `new.txt` does not yet
        // exist. `fs.write` must still be allowed to create it.
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(real_dir.join("a").join("b")).unwrap();

        let ambiguous = real_dir.join("a").join(".").join("b").join("new.txt");
        let got = canonicalize_no_symlink(&ambiguous)
            .expect("dot component on non-existent leaf must be accepted");

        assert_eq!(got, real_dir.join("a").join("b").join("new.txt"));
    }

    #[test]
    fn accepts_path_with_parent_dir_on_nonexistent_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(real_dir.join("a").join("x")).unwrap();
        std::fs::create_dir_all(real_dir.join("a").join("b")).unwrap();

        let ambiguous = real_dir
            .join("a")
            .join("x")
            .join("..")
            .join("b")
            .join("new.txt");
        let got = canonicalize_no_symlink(&ambiguous)
            .expect("parent-dir component on non-existent leaf must be accepted");

        assert_eq!(got, real_dir.join("a").join("b").join("new.txt"));
    }

    // ------------------------------------------------------------------
    // Real symlinks without any `.`/`..` — must be rejected (these already
    // worked before F-356, pinning them here catches regressions in the
    // normalized compare).
    // ------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_in_the_middle() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        let real_sub = real_dir.join("real_mid");
        std::fs::create_dir_all(real_sub.join("leaf")).unwrap();
        std::fs::File::create(real_sub.join("leaf").join("file.txt")).unwrap();

        let link = real_dir.join("link_mid");
        std::os::unix::fs::symlink(&real_sub, &link).unwrap();

        let via_link = link.join("leaf").join("file.txt");
        let err =
            canonicalize_no_symlink(&via_link).expect_err("symlink in the middle must be rejected");
        assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_at_the_leaf() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        let real_target = real_dir.join("real_leaf.txt");
        std::fs::File::create(&real_target).unwrap();

        let link = real_dir.join("link_leaf.txt");
        std::os::unix::fs::symlink(&real_target, &link).unwrap();

        let err = canonicalize_no_symlink(&link).expect_err("symlink at the leaf must be rejected");
        assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
    }

    // ------------------------------------------------------------------
    // F-356 bug: symlink + `.`/`..` in the same input. The length mismatch
    // caused by the `.`/`..` used to short-circuit the guard, so these fed
    // a resolved symlink through silently.
    // ------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_in_the_middle_when_combined_with_cur_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        let real_sub = real_dir.join("real_mid");
        std::fs::create_dir_all(real_sub.join("leaf")).unwrap();
        std::fs::File::create(real_sub.join("leaf").join("file.txt")).unwrap();

        let link = real_dir.join("link_mid");
        std::os::unix::fs::symlink(&real_sub, &link).unwrap();

        let via_link_and_dot = link.join(".").join("leaf").join("file.txt");
        let err = canonicalize_no_symlink(&via_link_and_dot)
            .expect_err("symlink-in-middle combined with `.` must be rejected");
        assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_in_the_middle_when_combined_with_parent_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        let real_sub = real_dir.join("real_mid");
        std::fs::create_dir_all(real_sub.join("leaf")).unwrap();
        std::fs::File::create(real_sub.join("leaf").join("file.txt")).unwrap();

        let link = real_dir.join("link_mid");
        std::os::unix::fs::symlink(&real_sub, &link).unwrap();

        // `link/sibling/../leaf/file.txt` — the `..` normalizes to `link/leaf`
        // but the `link` symlink is still in the middle.
        std::fs::create_dir_all(real_sub.join("sibling")).unwrap();
        let via_link_and_dotdot = link
            .join("sibling")
            .join("..")
            .join("leaf")
            .join("file.txt");
        let err = canonicalize_no_symlink(&via_link_and_dotdot)
            .expect_err("symlink-in-middle combined with `..` must be rejected");
        assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
    }

    // ------------------------------------------------------------------
    // Boundary-crossing variants. The helper catches user-space symlinks
    // whose target rides the *same* ancestor tree (component-rename at
    // fixed depth). Symlinks whose target sits in a different ancestor
    // tree alter the canonical form's component count — those are rejected
    // one frame up by `enforce_allowed`'s allowlist match, which is tested
    // through the public `write()` API in `tests/fs_write.rs`. The helper
    // tests below pin what this function owns directly.
    // ------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_at_the_leaf_pointing_outside_via_sibling() {
        // Leaf symlink points to a sibling file. The helper catches the
        // link name vs resolved file name at the same depth.
        let tmp = tempfile::tempdir().unwrap();
        let real_dir = tmp.path().canonicalize().unwrap();
        let target = real_dir.join("secret.txt");
        std::fs::File::create(&target).unwrap();

        let link = real_dir.join("leak.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let err = canonicalize_no_symlink(&link).expect_err("leaf-rename symlink must be rejected");
        assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
    }
}
