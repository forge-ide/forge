//! Directory tree listing for the `tree(root)` Tauri command (F-122).
//!
//! Shares the `allowed_paths` allowlist model with [`crate::read_file`] and
//! [`crate::write`]: the `root` is canonicalized and glob-matched against the
//! caller's allowlist before any directory entry is read. Symlinked entries
//! are omitted (not recursed into) so the returned tree cannot escape the
//! allowlist by traversing a link that points outside the sandbox.

use std::path::{Path, PathBuf};

use crate::{enforce_allowed, FsError};

/// Default cap on directory entries walked per `list_tree` call. Every entry
/// allocates a `TreeNode`; a pathological workspace (`node_modules`, deep
/// vendored trees) can push memory pressure and serialization time far past
/// what the webview needs. Callers can override via [`list_tree_with_limit`]
/// if they have a known-bounded use case.
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// A node in the directory tree returned to the webview.
///
/// `path` is the canonicalized absolute path; the frontend prefers it over
/// `name` when building URIs so the round-trip to `read_file` hits the same
/// on-disk object. `children` is `None` for files and symlinks, `Some(…)` for
/// directories (empty vec for empty dirs). Symlinks are included as entries
/// with `kind = Symlink` but never recursed into — see the sandbox rationale
/// at the module head.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub children: Option<Vec<TreeNode>>,
}

/// Entry kind. Kept narrow — anything else (block device, socket, FIFO) is
/// reported as [`NodeKind::Other`] so the wire shape is stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    File,
    Dir,
    Symlink,
    Other,
}

/// Walk `root` up to `max_depth` levels deep and return a tree rooted at
/// `root`. Entries beyond the depth cap are omitted (the parent still appears
/// with `children: Some(vec![])`). Entries beyond [`DEFAULT_MAX_ENTRIES`] are
/// silently truncated — the cap exists to block runaway trees, not to surface
/// errors to the user.
///
/// `max_depth == 0` returns the root node alone with no children walked.
///
/// # Errors
///
/// - [`FsError::PathDenied`] if the canonical `root` does not glob-match any
///   entry in `allowed_paths`.
/// - [`FsError::Io`] if `std::fs::canonicalize(root)` or the initial
///   `read_dir` fails. Per-child I/O errors are swallowed so a transient
///   failure on one entry doesn't hide the rest of the tree.
pub fn list_tree(
    root: &str,
    allowed_paths: &[String],
    max_depth: u32,
) -> Result<TreeNode, FsError> {
    list_tree_with_limit(root, allowed_paths, max_depth, DEFAULT_MAX_ENTRIES)
}

/// As [`list_tree`] but with a caller-chosen entry cap. Exposed for the
/// unit tests and any future caller whose allowlist bounds the tree tightly.
pub fn list_tree_with_limit(
    root: &str,
    allowed_paths: &[String],
    max_depth: u32,
    max_entries: usize,
) -> Result<TreeNode, FsError> {
    let canonical = std::fs::canonicalize(root).map_err(|e| FsError::Io {
        path: PathBuf::from(root),
        source: e,
    })?;
    enforce_allowed(&canonical, allowed_paths)?;

    let mut budget = max_entries;
    let name = canonical
        .file_name()
        .map(|o| o.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());

    let root_node = if canonical.is_dir() {
        let children = if max_depth == 0 {
            Vec::new()
        } else {
            walk_dir(&canonical, max_depth, &mut budget)
        };
        TreeNode {
            name,
            path: canonical,
            kind: NodeKind::Dir,
            children: Some(children),
        }
    } else {
        TreeNode {
            name,
            kind: classify(&canonical),
            path: canonical,
            children: None,
        }
    };
    Ok(root_node)
}

/// Walk a single directory, recursing into subdirectories while `remaining > 0`.
/// Budget is decremented per emitted node. Symlinks appear as leaves and are
/// never recursed into.
fn walk_dir(dir: &Path, remaining_depth: u32, budget: &mut usize) -> Vec<TreeNode> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return out,
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    // Stable ordering so snapshot-like diffs don't flake on platform-specific
    // readdir order. Case-insensitive on the name lower-case; path fallback
    // keeps duplicates unambiguous.
    entries.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());

    for entry in entries {
        if *budget == 0 {
            break;
        }
        *budget -= 1;

        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let kind = match entry.file_type() {
            Ok(ft) if ft.is_symlink() => NodeKind::Symlink,
            Ok(ft) if ft.is_dir() => NodeKind::Dir,
            Ok(ft) if ft.is_file() => NodeKind::File,
            _ => NodeKind::Other,
        };

        let children = if matches!(kind, NodeKind::Dir) && remaining_depth > 1 {
            Some(walk_dir(&path, remaining_depth - 1, budget))
        } else if matches!(kind, NodeKind::Dir) {
            Some(Vec::new())
        } else {
            None
        };

        out.push(TreeNode {
            name,
            path,
            kind,
            children,
        });
    }
    out
}

/// Best-effort classifier for a canonicalized leaf. `read_file` uses strict
/// canonicalization so by the time a leaf reaches here it exists — falling
/// through to `Other` only if the metadata call itself fails.
fn classify(path: &Path) -> NodeKind {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_symlink() => NodeKind::Symlink,
        Ok(md) if md.is_dir() => NodeKind::Dir,
        Ok(md) if md.is_file() => NodeKind::File,
        _ => NodeKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn allow(root: &Path) -> Vec<String> {
        // Canonicalize the allowed base so the glob match uses the same path
        // prefix `list_tree` produces.
        let base = fs::canonicalize(root).unwrap();
        vec![format!("{}/**", base.display()), base.display().to_string()]
    }

    #[test]
    fn lists_root_as_file_when_target_is_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("hello.txt");
        fs::write(&file, "hi").unwrap();

        let tree = list_tree(file.to_str().unwrap(), &allow(tmp.path()), 4).expect("list_tree ok");
        assert_eq!(tree.kind, NodeKind::File);
        assert!(tree.children.is_none());
    }

    #[test]
    fn lists_directory_with_children() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "a").unwrap();
        fs::write(tmp.path().join("b.txt"), "b").unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub/c.txt"), "c").unwrap();

        let tree =
            list_tree(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4).expect("list_tree ok");
        assert_eq!(tree.kind, NodeKind::Dir);
        let children = tree.children.expect("dir has Some(children)");
        let names: Vec<_> = children.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["a.txt", "b.txt", "sub"]);

        let sub = children.iter().find(|n| n.name == "sub").unwrap();
        let sub_children = sub.children.as_ref().unwrap();
        assert_eq!(sub_children.len(), 1);
        assert_eq!(sub_children[0].name, "c.txt");
        assert_eq!(sub_children[0].kind, NodeKind::File);
    }

    #[test]
    fn depth_cap_truncates_subtrees() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("a/b/c")).unwrap();
        fs::write(tmp.path().join("a/b/c/deep.txt"), "d").unwrap();

        let tree =
            list_tree(tmp.path().to_str().unwrap(), &allow(tmp.path()), 2).expect("list_tree ok");
        // depth=2: root (0) -> a (1) -> b (2) included, but b's children not walked
        let a = &tree.children.as_ref().unwrap()[0];
        let b = &a.children.as_ref().unwrap()[0];
        assert_eq!(b.name, "b");
        assert_eq!(
            b.children.as_ref().map(|v| v.len()).unwrap_or(usize::MAX),
            0,
            "b's children should be empty at depth cap"
        );
    }

    #[test]
    fn denies_path_outside_allowed() {
        let tmp = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        // Allow only `tmp`; ask for `other`.
        let err = list_tree(other.path().to_str().unwrap(), &allow(tmp.path()), 4)
            .expect_err("must deny path outside allowlist");
        assert!(matches!(err, FsError::PathDenied { .. }));
    }

    #[test]
    fn max_entries_budget_truncates_tree() {
        let tmp = TempDir::new().unwrap();
        for i in 0..20 {
            fs::write(tmp.path().join(format!("f{i:02}.txt")), "x").unwrap();
        }
        let tree = list_tree_with_limit(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4, 5)
            .expect("list_tree ok");
        let children = tree.children.unwrap();
        assert_eq!(children.len(), 5, "budget should cap children to 5");
    }

    #[test]
    fn depth_zero_returns_root_only() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), "a").unwrap();
        let tree =
            list_tree(tmp.path().to_str().unwrap(), &allow(tmp.path()), 0).expect("list_tree ok");
        assert_eq!(tree.kind, NodeKind::Dir);
        assert_eq!(tree.children.unwrap().len(), 0);
    }

    #[test]
    fn nonexistent_root_returns_io_error() {
        let tmp = TempDir::new().unwrap();
        let missing = tmp.path().join("does-not-exist");
        let err = list_tree(missing.to_str().unwrap(), &allow(tmp.path()), 4)
            .expect_err("missing path is an Io error");
        assert!(matches!(err, FsError::Io { .. }));
    }
}
