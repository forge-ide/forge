//! Directory tree listing for the `tree(root)` Tauri command (F-122).
//!
//! Shares the `allowed_paths` allowlist model with [`crate::read_file`] and
//! [`crate::write`]: the `root` is canonicalized and glob-matched against
//! the caller's allowlist before any directory entry is read. Symlinked
//! entries are reported as leaves and never recursed into, so the returned
//! tree cannot escape the allowlist by traversing a link that points outside
//! the sandbox.

use std::path::{Path, PathBuf};

use crate::{enforce_allowed, FsError};

/// Default cap on directory entries walked per `list_tree` call. Every entry
/// allocates a `TreeNode`; a pathological workspace (`node_modules`, deep
/// vendored trees) can push memory pressure and serialization time far past
/// what the webview needs. Callers can override via
/// [`list_tree_with_limit`] if they have a known-bounded use case.
pub const DEFAULT_MAX_ENTRIES: usize = 10_000;

/// A node in the directory tree returned to the webview.
///
/// `path` is the canonicalized absolute path; the frontend prefers it over
/// `name` when building URIs so the round-trip to `read_file` hits the same
/// on-disk object. `children` is `None` for files and symlinks, `Some(..)`
/// for directories (empty vec for empty dirs). Symlinks are included as
/// entries with `kind = Symlink` but never recursed into — see the sandbox
/// rationale at the module head.
///
/// F-357: the root node also carries [`TreeStats`] summarizing budget and
/// per-entry error signals from the whole walk. Nested nodes always carry a
/// default (zeroed) [`TreeStats`] — the summary is a whole-tree concept, not
/// per-directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeNode {
    pub name: String,
    pub path: PathBuf,
    pub kind: NodeKind,
    pub children: Option<Vec<TreeNode>>,
    pub stats: TreeStats,
}

/// F-357: summary of what the walk elided so the caller can render an
/// honest "N files not shown" indicator instead of implying a complete
/// listing. Populated on the root [`TreeNode`] only.
///
/// * `truncated` — the entry budget ([`DEFAULT_MAX_ENTRIES`] or the caller's
///   override) was exhausted mid-walk. More entries exist on disk than the
///   tree describes.
/// * `omitted_count` — best-effort count of entries skipped after the budget
///   tripped. Exact within each directory the walker was traversing when the
///   budget ran out; directories it never opened are not counted (we refuse
///   to spend additional I/O inflating a number we already know is
///   truncated).
/// * `error_count` — per-entry errors the walker swallowed rather than
///   failing the whole request. For `list_tree`, this is entries whose
///   `read_dir` iterator yielded `Err`. For `list_tree_gitignored`, this is
///   entries the `ignore` crate surfaced as errors (permission denied, etc.).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TreeStats {
    pub truncated: bool,
    pub omitted_count: u64,
    pub error_count: u64,
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
/// `root`. Entries beyond the depth cap are omitted (the parent still
/// appears with `children: Some(vec![])`). Entries beyond
/// [`DEFAULT_MAX_ENTRIES`] are dropped, and the root node's
/// [`TreeNode::stats`] flags this with `truncated: true` plus an
/// `omitted_count` so the caller can render an honest "N files not shown"
/// indicator instead of implying a complete listing (F-357).
///
/// `max_depth == 0` returns the root node alone with no children walked.
///
/// # Errors
///
/// - [`FsError::PathDenied`] if the canonical `root` does not glob-match any
///   entry in `allowed_paths`.
/// - [`FsError::Io`] if `std::fs::canonicalize(root)` fails. Per-child I/O
///   errors are swallowed so a transient failure on one entry doesn't hide
///   the rest of the tree, but each one increments [`TreeStats::error_count`]
///   on the root.
pub fn list_tree(
    root: &str,
    allowed_paths: &[String],
    max_depth: u32,
) -> Result<TreeNode, FsError> {
    list_tree_with_limit(root, allowed_paths, max_depth, DEFAULT_MAX_ENTRIES)
}

/// As [`list_tree`] but with a caller-chosen entry cap. Exposed for the unit
/// tests and any future caller whose allowlist bounds the tree tightly.
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
    let mut stats = TreeStats::default();
    let name = canonical
        .file_name()
        .map(|o| o.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());

    let root_node = if canonical.is_dir() {
        let children = if max_depth == 0 {
            Vec::new()
        } else {
            walk_dir(&canonical, max_depth, &mut budget, &mut stats)
        };
        TreeNode {
            name,
            path: canonical,
            kind: NodeKind::Dir,
            children: Some(children),
            stats,
        }
    } else {
        TreeNode {
            name,
            kind: classify(&canonical),
            path: canonical,
            children: None,
            stats,
        }
    };
    Ok(root_node)
}

/// F-126: `list_tree` variant that honors `.gitignore`, `.ignore`, global
/// gitignore and parent `.gitignore` files via the `ignore` crate. Hidden
/// files (`.git/`, dotfiles) are skipped — matching VS Code's files sidebar
/// defaults. The non-ignored [`list_tree`] is retained for callers that want
/// raw `read_dir` semantics (e.g. agent `fs.tree` tool calls that may need
/// to show gitignored files for debugging).
///
/// The allowlist and entry-budget / depth-cap contracts are identical to
/// [`list_tree`]. Symlinks are reported as leaves and never recursed into
/// (the `ignore` crate has `follow_links(false)` by default, which we rely
/// on for sandbox safety).
pub fn list_tree_gitignored(
    root: &str,
    allowed_paths: &[String],
    max_depth: u32,
) -> Result<TreeNode, FsError> {
    list_tree_gitignored_with_limit(root, allowed_paths, max_depth, DEFAULT_MAX_ENTRIES)
}

/// As [`list_tree_gitignored`] but with a caller-chosen entry cap.
pub fn list_tree_gitignored_with_limit(
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

    let name = canonical
        .file_name()
        .map(|o| o.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());

    if !canonical.is_dir() {
        return Ok(TreeNode {
            name,
            kind: classify(&canonical),
            path: canonical,
            children: None,
            stats: TreeStats::default(),
        });
    }

    // Bridge `ignore::Walk`'s flat stream into our recursive `TreeNode` shape.
    // We keep a per-directory bucket keyed by the canonical parent path so
    // each entry slots into its parent's `children` vec as we see it. `Walk`
    // yields parents before children when `max_depth` is set, so the first
    // time a child lands its parent already exists in the map.
    use std::collections::HashMap;

    let mut builder = ignore::WalkBuilder::new(&canonical);
    // max_depth on `ignore::WalkBuilder` counts from the root as depth 0
    // (root itself), which matches `list_tree`'s contract.
    builder
        .max_depth(Some(max_depth as usize))
        .follow_links(false)
        .hidden(true)
        .git_global(true)
        .git_ignore(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false);

    let mut buckets: HashMap<PathBuf, Vec<TreeNode>> = HashMap::new();
    buckets.insert(canonical.clone(), Vec::new());
    let mut seen_entries: usize = 0;
    let mut stats = TreeStats::default();

    // F-357: drain the iterator even after the budget trips so we can count
    // (and report) how many further entries existed rather than silently
    // dropping them. `ignore::Walk` does not cheaply bound the remaining
    // stream, so a one-time post-budget drain is the honest way to surface
    // `omitted_count`. The user already paid for the `read_dir` enumeration
    // — we just skip the `TreeNode` allocation for anything past the cap.
    for entry in builder.build() {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                stats.error_count = stats.error_count.saturating_add(1);
                continue;
            }
        };
        let entry_path = entry.path().to_path_buf();
        if entry_path == canonical {
            continue;
        }
        if seen_entries >= max_entries {
            stats.truncated = true;
            stats.omitted_count = stats.omitted_count.saturating_add(1);
            continue;
        }
        seen_entries += 1;

        let file_type = entry.file_type();
        let kind = match file_type {
            Some(ft) if ft.is_symlink() => NodeKind::Symlink,
            Some(ft) if ft.is_dir() => NodeKind::Dir,
            Some(ft) if ft.is_file() => NodeKind::File,
            _ => NodeKind::Other,
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let node = TreeNode {
            name,
            path: entry_path.clone(),
            kind,
            children: if matches!(kind, NodeKind::Dir) {
                Some(Vec::new())
            } else {
                None
            },
            stats: TreeStats::default(),
        };
        if matches!(kind, NodeKind::Dir) {
            // `entry(_).or_insert_with(Vec::new)` is intentional over `.insert`:
            // if the walker ever yields a child before its parent (parallel
            // mode, future `ignore` changes, or edge cases with parent
            // `.gitignore` traversal), the earlier children already live in
            // `buckets[entry_path]`. A blind `.insert` would overwrite and
            // silently lose them; the entry-API preserves them.
            buckets.entry(entry_path.clone()).or_default();
        }
        let parent = entry_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| canonical.clone());
        buckets.entry(parent).or_default().push(node);
    }

    // Second pass: hoist each directory's bucket into its own `children`
    // vec. Walk depth-first from the root; sort children by lowercase name
    // for stable output matching `walk_dir`.
    fn assemble(
        node: TreeNode,
        buckets: &mut std::collections::HashMap<PathBuf, Vec<TreeNode>>,
    ) -> TreeNode {
        if !matches!(node.kind, NodeKind::Dir) {
            return node;
        }
        let mut children = buckets.remove(&node.path).unwrap_or_default();
        children.sort_by_key(|c| c.name.to_lowercase());
        let assembled = children.into_iter().map(|c| assemble(c, buckets)).collect();
        TreeNode {
            children: Some(assembled),
            ..node
        }
    }

    let root_node = TreeNode {
        name,
        path: canonical.clone(),
        kind: NodeKind::Dir,
        children: Some(Vec::new()),
        stats,
    };
    Ok(assemble(root_node, &mut buckets))
}

/// Walk a single directory, recursing into subdirectories while
/// `remaining_depth > 0`. Budget is decremented per emitted node. Symlinks
/// appear as leaves and are never recursed into.
///
/// F-357: when the budget runs out mid-directory, the remaining entries in
/// the *current* `read_dir` stream are counted into
/// [`TreeStats::omitted_count`] and `truncated` is set. Subdirectories the
/// walker never opened are not counted — inflating the count would require
/// an extra I/O pass we deliberately skip. Per-entry `read_dir` errors
/// (e.g. a file that vanished mid-walk) bump [`TreeStats::error_count`].
fn walk_dir(
    dir: &Path,
    remaining_depth: u32,
    budget: &mut usize,
    stats: &mut TreeStats,
) -> Vec<TreeNode> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => {
            stats.error_count = stats.error_count.saturating_add(1);
            return out;
        }
    };
    let mut raw: Vec<_> = Vec::new();
    for entry in entries {
        match entry {
            Ok(e) => raw.push(e),
            Err(_) => stats.error_count = stats.error_count.saturating_add(1),
        }
    }
    // Stable ordering so snapshot-like diffs don't flake on platform-specific
    // readdir order. Case-insensitive on the name lower-case keeps mixed-case
    // workspace roots deterministic.
    raw.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());

    let mut iter = raw.into_iter();
    while let Some(entry) = iter.next() {
        if *budget == 0 {
            stats.truncated = true;
            // Count this entry plus whatever else sits unread in the iterator.
            // `size_hint().1` is reliable for a `Vec::IntoIter`.
            let remaining = iter.size_hint().1.unwrap_or(0) as u64 + 1;
            stats.omitted_count = stats.omitted_count.saturating_add(remaining);
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
            Some(walk_dir(&path, remaining_depth - 1, budget, stats))
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
            stats: TreeStats::default(),
        });
    }
    out
}

/// Best-effort classifier for a canonicalized leaf. `list_tree` uses strict
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
        // F-357: same budget behavior, but the root now reports truncation.
        assert!(
            tree.stats.truncated,
            "budget exhaustion must flag truncated"
        );
        assert_eq!(
            tree.stats.omitted_count, 15,
            "15 of 20 files must be reported as omitted"
        );
        assert_eq!(tree.stats.error_count, 0);
        let children = tree.children.as_ref().unwrap();
        assert_eq!(children.len(), 5, "budget should cap children to 5");
        for child in children {
            assert_eq!(
                child.stats,
                TreeStats::default(),
                "non-root nodes carry default stats"
            );
        }
    }

    // -----------------------------------------------------------------------
    // F-357: truncation / error signaling
    // -----------------------------------------------------------------------

    #[test]
    fn not_truncated_when_under_cap() {
        let tmp = TempDir::new().unwrap();
        for i in 0..5 {
            fs::write(tmp.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let tree = list_tree_with_limit(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4, 100)
            .expect("list_tree ok");
        assert_eq!(tree.stats, TreeStats::default());
    }

    #[test]
    fn budget_truncation_counts_across_subtree() {
        // Split entries between root and a subdir so the budget is spent mid-
        // walk inside the sub. Confirms the omitted count includes leftover
        // entries in whichever directory was being read when the budget ran
        // out (but not directories we never opened).
        let tmp = TempDir::new().unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        for i in 0..5 {
            fs::write(tmp.path().join(format!("r{i}.txt")), "x").unwrap();
        }
        for i in 0..10 {
            fs::write(tmp.path().join("sub").join(format!("s{i}.txt")), "x").unwrap();
        }
        // Budget = 8: 5 root files + "sub" dir = 6, then 2 of sub's 10 files
        // fit before the budget trips. 8 remaining sub entries must be reported.
        let tree = list_tree_with_limit(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4, 8)
            .expect("list_tree ok");
        assert!(tree.stats.truncated);
        assert_eq!(tree.stats.omitted_count, 8);
    }

    #[test]
    fn regression_20k_entry_dir_reports_truncated() {
        // F-357 DoD: a 20 000-entry directory at default cap must flag
        // `truncated: true`. Uses the real `list_tree` (DEFAULT_MAX_ENTRIES
        // = 10 000) so a future cap change has to update this assertion
        // deliberately.
        let tmp = TempDir::new().unwrap();
        for i in 0..20_000 {
            fs::write(tmp.path().join(format!("f{i:05}.txt")), "").unwrap();
        }
        let tree =
            list_tree(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4).expect("list_tree ok");
        assert!(tree.stats.truncated, "20k-entry dir must report truncated");
        assert_eq!(
            tree.stats.omitted_count,
            20_000 - DEFAULT_MAX_ENTRIES as u64,
            "omitted count must equal total minus cap"
        );
    }

    #[test]
    fn gitignored_walker_reports_truncation() {
        let tmp = TempDir::new().unwrap();
        for i in 0..20 {
            fs::write(tmp.path().join(format!("f{i:02}.txt")), "x").unwrap();
        }
        let tree =
            list_tree_gitignored_with_limit(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4, 5)
                .expect("gitignored walk ok");
        assert!(
            tree.stats.truncated,
            "budget exhaustion must flag truncated"
        );
        assert_eq!(
            tree.stats.omitted_count, 15,
            "15 of 20 files must be reported as omitted"
        );
    }

    #[test]
    fn gitignored_walker_not_truncated_when_under_cap() {
        let tmp = TempDir::new().unwrap();
        for i in 0..3 {
            fs::write(tmp.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let tree = list_tree_gitignored_with_limit(
            tmp.path().to_str().unwrap(),
            &allow(tmp.path()),
            4,
            100,
        )
        .expect("gitignored walk ok");
        assert_eq!(tree.stats, TreeStats::default());
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

    // -----------------------------------------------------------------------
    // F-126: gitignore-aware walker
    // -----------------------------------------------------------------------

    #[test]
    fn gitignored_walker_excludes_ignored_paths() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join(".gitignore"),
            "node_modules/\nbuild/\n*.log\n",
        )
        .unwrap();
        fs::write(tmp.path().join("app.ts"), "").unwrap();
        fs::write(tmp.path().join("keep.md"), "").unwrap();
        fs::write(tmp.path().join("debug.log"), "").unwrap();
        fs::create_dir(tmp.path().join("node_modules")).unwrap();
        fs::write(tmp.path().join("node_modules/junk.js"), "").unwrap();
        fs::create_dir(tmp.path().join("build")).unwrap();
        fs::write(tmp.path().join("build/out.o"), "").unwrap();

        let tree = list_tree_gitignored(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4)
            .expect("gitignored walk ok");
        let children = tree.children.expect("root has children");
        let names: Vec<_> = children.iter().map(|n| n.name.as_str()).collect();
        // .gitignore itself is hidden (starts with dot) so it's excluded by
        // the `hidden(true)` default, matching VS Code's default. `debug.log`
        // and `node_modules` and `build` must all be excluded.
        assert!(
            names.contains(&"app.ts"),
            "app.ts must be present: {names:?}"
        );
        assert!(
            names.contains(&"keep.md"),
            "keep.md must be present: {names:?}"
        );
        assert!(
            !names.contains(&"debug.log"),
            "*.log must be excluded: {names:?}"
        );
        assert!(
            !names.contains(&"node_modules"),
            "node_modules/ must be excluded: {names:?}"
        );
        assert!(
            !names.contains(&"build"),
            "build/ must be excluded: {names:?}"
        );
    }

    #[test]
    fn gitignored_walker_denies_paths_outside_allowlist() {
        let tmp = TempDir::new().unwrap();
        let other = TempDir::new().unwrap();
        let err = list_tree_gitignored(other.path().to_str().unwrap(), &allow(tmp.path()), 4)
            .expect_err("must deny out-of-allowlist root");
        assert!(matches!(err, FsError::PathDenied { .. }));
    }

    #[test]
    fn gitignored_walker_lists_nested_dirs() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join(".gitignore"), "ignored.txt\n").unwrap();
        fs::create_dir(tmp.path().join("src")).unwrap();
        fs::write(tmp.path().join("src/main.rs"), "").unwrap();
        fs::write(tmp.path().join("ignored.txt"), "").unwrap();

        let tree = list_tree_gitignored(tmp.path().to_str().unwrap(), &allow(tmp.path()), 4)
            .expect("walk ok");
        let children = tree.children.expect("has children");
        let src = children
            .iter()
            .find(|n| n.name == "src")
            .expect("src dir present");
        assert_eq!(src.kind, NodeKind::Dir);
        let src_children = src.children.as_ref().unwrap();
        assert_eq!(src_children.len(), 1);
        assert_eq!(src_children[0].name, "main.rs");
        let names: Vec<_> = children.iter().map(|n| n.name.as_str()).collect();
        assert!(
            !names.contains(&"ignored.txt"),
            "ignored.txt must be excluded: {names:?}"
        );
    }
}
