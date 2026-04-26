//! F-571: tree-walk perf + allocation-budget guard.
//!
//! This bench mirrors the structure of `benches/mutate.rs` (F-111): the
//! pre-fix walker is inlined as `walk_dir_baseline` / `gitignored_baseline`
//! so we can prove the speedup against an in-binary baseline rather than
//! relying on git-history numbers that drift. Two scenarios:
//!
//! 1. `walk_dir_50k` — a 50_000-entry tempdir spread across 1_000 subdirs
//!    (the wide-fanout shape that exercises `sort_by_key`'s repeated
//!    `to_lowercase` calls). Compares `sort_by_key` (pre-fix) against
//!    `sort_by_cached_key` (post-fix). The DoD calls for ≥50% latency
//!    reduction; the cached-key win is largest on directories with many
//!    siblings — which is exactly this fixture.
//!
//! 2. `node_modules_shape` — ~80_000 entries deep-nested at budget 10_000,
//!    matching the real `node_modules` shape called out in the issue. The
//!    public `list_tree_with_limit` is the post-fix path; the inlined
//!    `walk_dir_baseline_drain` simulates the pre-fix gitignored-walker
//!    behavior (read every entry past the budget just to count them).
//!
//! The allocation-counting global allocator is the same pattern as
//! `benches/mutate.rs`. Because both `tree_walk` and `mutate` link a custom
//! `#[global_allocator]`, they must run as separate bench binaries — the
//! `Cargo.toml` `[[bench]]` entries enforce that.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use forge_fs::{
    list_tree_gitignored_with_limit, list_tree_with_limit, NodeKind, TreeNode, TreeStats,
};
use std::collections::HashMap;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Allocation-counting global allocator.
// ---------------------------------------------------------------------------

struct CountingAllocator;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
        if new_size > layout.size() {
            ALLOC_BYTES.fetch_add(new_size - layout.size(), Ordering::Relaxed);
        }
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn snapshot() -> (usize, usize) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

fn delta(before: (usize, usize)) -> (usize, usize) {
    let (c, b) = snapshot();
    (c - before.0, b - before.1)
}

// ---------------------------------------------------------------------------
// Baseline walker (pre-F-571). Verbatim copy of the previous `walk_dir`
// body, but with `sort_by_key` retained where the optimized path now uses
// `sort_by_cached_key`. Lives only in the bench binary.
// ---------------------------------------------------------------------------

fn classify(path: &Path) -> NodeKind {
    match std::fs::symlink_metadata(path) {
        Ok(md) if md.file_type().is_symlink() => NodeKind::Symlink,
        Ok(md) if md.is_dir() => NodeKind::Dir,
        Ok(md) if md.is_file() => NodeKind::File,
        _ => NodeKind::Other,
    }
}

/// Pre-F-571 `walk_dir`: sorts via `sort_by_key`, recomputing
/// `to_lowercase()` once per comparator call. Same I/O shape as the
/// optimized path; only the sort comparator differs, so a delta in
/// wall-clock here is attributable to the cached-key change.
fn walk_dir_baseline(
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
    // PRE-F-571: `sort_by_key` recomputes the lowercase string per comparator
    // call → ~O(n log n) `String` allocations per directory.
    raw.sort_by_key(|e| e.file_name().to_string_lossy().to_lowercase());

    let mut iter = raw.into_iter();
    while let Some(entry) = iter.next() {
        if *budget == 0 {
            stats.truncated = true;
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
            Some(walk_dir_baseline(&path, remaining_depth - 1, budget, stats))
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

/// Pre-F-571 `list_tree_gitignored_with_limit`: drains the rest of the
/// `ignore::Walk` stream after the budget trips just to inflate
/// `omitted_count`. The optimized path short-circuits the moment the budget
/// is hit and reports `omitted_count = u64::MAX`. On a `node_modules`-shaped
/// tree the drain dominates wall-time — that's the bench this captures.
fn list_tree_gitignored_baseline(root: &Path, max_depth: u32, max_entries: usize) -> TreeNode {
    let canonical = std::fs::canonicalize(root).expect("canonicalize");
    let name = canonical
        .file_name()
        .map(|o| o.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());

    let mut builder = ignore::WalkBuilder::new(&canonical);
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
        // PRE-F-571: drain the whole iterator to count omissions.
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
            buckets.entry(entry_path.clone()).or_default();
        }
        let parent = entry_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| canonical.clone());
        buckets.entry(parent).or_default().push(node);
    }

    fn assemble(node: TreeNode, buckets: &mut HashMap<PathBuf, Vec<TreeNode>>) -> TreeNode {
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
    assemble(root_node, &mut buckets)
}

fn list_tree_baseline(root: &Path, max_depth: u32, max_entries: usize) -> TreeNode {
    let canonical = std::fs::canonicalize(root).expect("canonicalize root");
    let mut budget = max_entries;
    let mut stats = TreeStats::default();
    let name = canonical
        .file_name()
        .map(|o| o.to_string_lossy().into_owned())
        .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
    let children = walk_dir_baseline(&canonical, max_depth, &mut budget, &mut stats);
    TreeNode {
        name,
        kind: classify(&canonical),
        path: canonical,
        children: Some(children),
        stats,
    }
}

// ---------------------------------------------------------------------------
// Fixture builders.
// ---------------------------------------------------------------------------

fn allow(root: &Path) -> Vec<String> {
    let base = root.canonicalize().unwrap();
    vec![format!("{}/**", base.display()), base.display().to_string()]
}

/// Wide tempdir: 1 000 subdirs × 50 files each = 50 000 entries. The point of
/// this shape is the per-directory sort cost — every dir has 50 siblings and
/// every iteration of the sort comparator paid `to_lowercase` in the pre-fix
/// path. The wins are linear in directory width, not depth.
fn build_50k_tempdir() -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    for d in 0..1_000 {
        // Mix-cased subdir names so the lowercase key is meaningful, not a
        // no-op the optimizer can fold to identity.
        let sub = tmp.path().join(format!("Sub_{d:04}_X"));
        fs::create_dir(&sub).unwrap();
        for f in 0..50 {
            fs::write(sub.join(format!("File_{f:03}.TXT")), b"").unwrap();
        }
    }
    tmp
}

/// Deep `node_modules` shape: ~80 000 entries spread across nested dirs.
/// The intent is to surface the post-budget drain cost — at budget 10_000
/// the optimized path stops; the baseline keeps walking the rest of the
/// tree just to count omissions. `walk_dir`'s drain is bounded to the
/// current directory (it doesn't open new ones), so the wide+shallow shape
/// here is what makes the drain visible: every subdir has hundreds of
/// siblings, and the sort fires for each.
fn build_node_modules_shape() -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    // 200 packages × 400 files each = 80_000 entries. Each "package" is a
    // single subdir with sibling files, mirroring an unzipped `npm install`
    // tree closely enough that the sort + drain costs reflect production.
    for p in 0..200 {
        let pkg = tmp.path().join(format!("pkg_{p:04}"));
        fs::create_dir(&pkg).unwrap();
        for f in 0..400 {
            fs::write(pkg.join(format!("Mod_{f:04}.JS")), b"").unwrap();
        }
    }
    tmp
}

// ---------------------------------------------------------------------------
// Allocation-budget guard. Runs once at startup.
// ---------------------------------------------------------------------------

/// Sanity guard for the cached-key change. The optimized path does strictly
/// fewer transient allocations because `to_lowercase()` fires once per entry
/// (cached into a side table) instead of `O(n log n)` times in the
/// comparator. The bulk of the walker's allocations come from `read_dir` /
/// `PathBuf` construction — those are unchanged — so a 1.2× total-allocation
/// reduction is the realistic floor at this fixture size; the absolute
/// transient-string drop is what wins us the wall-clock improvement, not the
/// total ratio. Falling below 1.2× signals the cached-key change regressed.
const MIN_ALLOC_RATIO: f64 = 1.2;

fn assert_allocation_budget(tmp: &TempDir) {
    // Warm both impls + cross-check that they produce the same number of
    // immediate children (sort order + budget semantics are identical, only
    // the comparator changes).
    let allowed = allow(tmp.path());
    let opt =
        list_tree_with_limit(tmp.path().to_str().unwrap(), &allowed, 4, 60_000).expect("opt walk");
    let baseline = list_tree_baseline(tmp.path(), 4, 60_000);
    assert_eq!(
        opt.children.as_ref().map(|c| c.len()),
        baseline.children.as_ref().map(|c| c.len()),
        "optimized vs baseline child count diverged"
    );
    drop(opt);
    drop(baseline);

    let before = snapshot();
    let opt =
        list_tree_with_limit(tmp.path().to_str().unwrap(), &allowed, 4, 60_000).expect("opt walk");
    let (opt_allocs, opt_bytes) = delta(before);
    drop(opt);

    let before = snapshot();
    let baseline = list_tree_baseline(tmp.path(), 4, 60_000);
    let (base_allocs, base_bytes) = delta(before);
    drop(baseline);

    let ratio = base_allocs as f64 / opt_allocs.max(1) as f64;
    eprintln!(
        "F-571 allocation budget (50k tempdir, depth=4, budget=60k):\n  \
         optimized: {opt_allocs} allocs, {opt_bytes} bytes\n  \
         baseline:  {base_allocs} allocs, {base_bytes} bytes\n  \
         ratio:     {ratio:.2}x"
    );
    assert!(
        ratio >= MIN_ALLOC_RATIO,
        "F-571 alloc-count regression: optimized {opt_allocs}, baseline {base_allocs}, \
         ratio {ratio:.2}x < required {MIN_ALLOC_RATIO}x"
    );
}

// ---------------------------------------------------------------------------
// Criterion benchmarks.
// ---------------------------------------------------------------------------

fn bench_walk_dir_50k(c: &mut Criterion) {
    let tmp = build_50k_tempdir();
    let path = tmp.path().to_str().unwrap().to_string();
    let allowed = allow(tmp.path());

    assert_allocation_budget(&tmp);

    let mut group = c.benchmark_group("walk_dir_50k");
    // Filesystem benches are noisy and ~slow; a small sample size keeps the
    // wall-clock per `cargo bench` tractable while still surfacing the gap.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8));

    group.bench_function("optimized_cached_key", |b| {
        b.iter(|| {
            let tree = list_tree_with_limit(black_box(&path), black_box(&allowed), 4, 60_000)
                .expect("list_tree ok");
            black_box(tree);
        })
    });
    group.bench_function("baseline_sort_by_key", |b| {
        b.iter(|| {
            let tree = list_tree_baseline(black_box(tmp.path()), 4, 60_000);
            black_box(tree);
        })
    });
    group.finish();
}

fn bench_node_modules_shape(c: &mut Criterion) {
    let tmp = build_node_modules_shape();
    let path_buf: PathBuf = tmp.path().to_path_buf();
    let path = path_buf.to_str().unwrap().to_string();
    let allowed = allow(tmp.path());

    let mut group = c.benchmark_group("node_modules_shape");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8));

    // Budget = 10 000 mirrors `DEFAULT_MAX_ENTRIES`. The optimized path
    // benefits from the cached lowercase key on every directory's sort.
    group.bench_function("optimized_budget_10k", |b| {
        b.iter(|| {
            let tree = list_tree_with_limit(black_box(&path), black_box(&allowed), 6, 10_000)
                .expect("list_tree ok");
            black_box(tree);
        })
    });
    group.bench_function("baseline_budget_10k", |b| {
        b.iter(|| {
            let tree = list_tree_baseline(black_box(tmp.path()), 6, 10_000);
            black_box(tree);
        })
    });
    group.finish();
}

/// `node_modules`-shape gitignored walk at budget 10 000. This is the
/// fixture the F-571 issue calls out by name: ~80 000 entries on disk,
/// budget caps the visible tree at 10 000, the pre-fix path drains the
/// remaining ~70 000 entries from `ignore::Walk` just to count omissions.
/// The optimized path short-circuits at the budget; the wall-clock delta
/// here is the headline DoD number ("≥50% latency reduction at 80k entries
/// budget=10k").
fn bench_gitignored_node_modules(c: &mut Criterion) {
    let tmp = build_node_modules_shape();
    let path = tmp.path().to_str().unwrap().to_string();
    let allowed = allow(tmp.path());

    let mut group = c.benchmark_group("gitignored_node_modules_budget_10k");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(8));

    group.bench_function("optimized_short_circuit", |b| {
        b.iter(|| {
            let tree =
                list_tree_gitignored_with_limit(black_box(&path), black_box(&allowed), 6, 10_000)
                    .expect("gitignored walk ok");
            black_box(tree);
        })
    });
    group.bench_function("baseline_post_budget_drain", |b| {
        b.iter(|| {
            let tree = list_tree_gitignored_baseline(black_box(tmp.path()), 6, 10_000);
            black_box(tree);
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_walk_dir_50k,
    bench_node_modules_shape,
    bench_gitignored_node_modules
);
criterion_main!(benches);
