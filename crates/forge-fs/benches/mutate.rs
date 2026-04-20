//! Perf + allocation-budget guard for `apply_unified_diff`.
//!
//! This bench does two things:
//!
//! 1. Asserts that the optimized impl performs a bounded number of heap
//!    allocations while applying a 1000-line unified-diff patch to a ~10 MB
//!    synthetic file — specifically, at least 5× fewer allocations than a
//!    copy of the pre-F-111 naive implementation included below. The
//!    assertion runs during the bench binary's startup; the bench therefore
//!    **fails loudly on baseline** (if someone reverts the optimization) and
//!    passes on the current impl.
//!
//! 2. Reports wall-clock throughput via criterion for both implementations
//!    so regressions are visible as numbers, not just pass/fail.
//!
//! The naive implementation below is a verbatim copy of the pre-F-111
//! `apply_unified_diff` in `crates/forge-fs/src/mutate.rs` (per-line
//! `String` allocations + final `Vec<String>::concat()`). It exists only in
//! the bench binary — production code has a single optimized path.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use forge_fs::__bench_internals::apply_unified_diff;

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
        // realloc can either resize in place or allocate fresh; count it once.
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
// Naive implementation (pre-F-111). Kept here to prove the allocation delta.
// ---------------------------------------------------------------------------

#[derive(Debug)]
#[allow(dead_code)] // String is read via #[derive(Debug)] when unwrap panics.
struct NaiveErr(String);

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

fn parse_hunk_old_start(rest: &str) -> Result<usize, NaiveErr> {
    let minus = rest
        .split_whitespace()
        .find(|t| t.starts_with('-'))
        .ok_or_else(|| NaiveErr(format!("hunk header missing '-' range: {rest:?}")))?;
    let spec = minus.trim_start_matches('-');
    let (start_str, _count_str) = spec.split_once(',').unwrap_or((spec, "1"));
    start_str
        .parse::<usize>()
        .map_err(|_| NaiveErr(format!("hunk start not a number: {start_str:?}")))
}

fn apply_unified_diff_naive(original: &str, patch: &str) -> Result<String, NaiveErr> {
    let src_lines: Vec<&str> = split_preserving_newline(original);
    let mut out: Vec<String> = Vec::with_capacity(src_lines.len());
    let mut cursor = 0usize;
    let mut saw_hunk = false;
    let mut lines = patch.lines().peekable();

    while let Some(raw) = lines.next() {
        if raw.starts_with("--- ") || raw.starts_with("+++ ") {
            continue;
        }
        if let Some(rest) = raw.strip_prefix("@@ ") {
            saw_hunk = true;
            let old_start = parse_hunk_old_start(rest)?;
            let target = old_start.saturating_sub(1);
            if target < cursor || target > src_lines.len() {
                return Err(NaiveErr(format!(
                    "hunk range out of bounds at line {old_start}"
                )));
            }
            for l in &src_lines[cursor..target] {
                out.push((*l).to_string());
            }
            cursor = target;

            while let Some(peek) = lines.peek() {
                if peek.starts_with("@@ ") {
                    break;
                }
                let body = lines.next().unwrap();
                if let Some(ctx) = body.strip_prefix(' ') {
                    let src = src_lines
                        .get(cursor)
                        .ok_or_else(|| NaiveErr("context line past end of source".into()))?;
                    if src.trim_end_matches('\n') != ctx.trim_end_matches('\n') {
                        return Err(NaiveErr(format!("context mismatch at line {}", cursor + 1)));
                    }
                    out.push((*src).to_string());
                    cursor += 1;
                } else if let Some(removed) = body.strip_prefix('-') {
                    let src = src_lines
                        .get(cursor)
                        .ok_or_else(|| NaiveErr("delete past end of source".into()))?;
                    if src.trim_end_matches('\n') != removed.trim_end_matches('\n') {
                        return Err(NaiveErr(format!("delete mismatch at line {}", cursor + 1)));
                    }
                    cursor += 1;
                } else if let Some(added) = body.strip_prefix('+') {
                    out.push(format!("{added}\n"));
                } else if body.starts_with("\\ ") {
                    if let Some(last) = out.last_mut() {
                        if last.ends_with('\n') {
                            last.pop();
                        }
                    }
                } else if body.is_empty() {
                    let src = src_lines
                        .get(cursor)
                        .ok_or_else(|| NaiveErr("empty context past end of source".into()))?;
                    if !src.trim_end_matches('\n').is_empty() {
                        return Err(NaiveErr(format!(
                            "empty context mismatch at line {}",
                            cursor + 1
                        )));
                    }
                    out.push((*src).to_string());
                    cursor += 1;
                } else {
                    return Err(NaiveErr(format!("unrecognized line prefix: {body:?}")));
                }
            }
        } else if saw_hunk {
            return Err(NaiveErr(format!("unexpected line outside hunk: {raw:?}")));
        }
    }

    if !saw_hunk {
        return Err(NaiveErr("no hunks found".into()));
    }
    for l in &src_lines[cursor..] {
        out.push((*l).to_string());
    }
    Ok(out.concat())
}

// ---------------------------------------------------------------------------
// Synthetic fixture: ~10 MB file + 1000-line unified-diff patch.
// ---------------------------------------------------------------------------

const LINES: usize = 10_000; // file lines
const LINE_BYTES: usize = 1000; // line payload length (~1 KB each → ~10 MB file)
const PATCH_LINES: usize = 1000; // edits scattered across the file

fn build_file() -> String {
    let mut s = String::with_capacity(LINES * (LINE_BYTES + 16));
    for i in 0..LINES {
        // Distinctive per-line content so context verification is meaningful.
        let payload = format!("line-{i:07}-");
        s.push_str(&payload);
        let filler = LINE_BYTES.saturating_sub(payload.len());
        for _ in 0..filler {
            s.push('x');
        }
        s.push('\n');
    }
    s
}

/// Build a hand-rolled unified-diff that replaces every 10th line in the first
/// 10_000 lines (so `PATCH_LINES` replacements total). Hand-rolled to avoid
/// pulling an off-workspace dep and to stay deterministic.
fn build_patch(original: &str) -> String {
    let lines: Vec<&str> = original.lines().collect();
    assert!(lines.len() >= LINES);
    let stride = LINES / PATCH_LINES;
    assert!(stride >= 1);

    let mut patch = String::with_capacity(PATCH_LINES * (LINE_BYTES + 32));
    for edit_idx in 0..PATCH_LINES {
        let line_no_0 = edit_idx * stride; // 0-indexed
        let line_no_1 = line_no_0 + 1; // 1-indexed for hunk header
                                       // Single-line hunk: delete one line, add one line, no context.
        patch.push_str(&format!("@@ -{line_no_1},1 +{line_no_1},1 @@\n"));
        patch.push('-');
        patch.push_str(lines[line_no_0]);
        patch.push('\n');
        patch.push('+');
        patch.push_str(lines[line_no_0]);
        patch.push_str("-EDITED\n");
    }
    patch
}

// ---------------------------------------------------------------------------
// Allocation-budget guard. Runs once at bench startup; panics on regression.
// ---------------------------------------------------------------------------

/// Ratio the DoD requires: optimized impl must do at most `naive / RATIO`
/// allocations on the 1000-line patch.
const REQUIRED_RATIO: usize = 5;

fn assert_allocation_budget(original: &str, patch: &str) {
    // Warm caches, drop any lazily-initialized statics, etc. — we only care
    // about the steady-state allocations of the two impls. Also cross-check
    // that both impls produce byte-identical output so the allocation-ratio
    // guarantee is meaningful (a "faster" impl that drops lines would
    // otherwise pass silently).
    let warm_opt =
        apply_unified_diff(original, patch).expect("optimized impl should apply cleanly");
    let warm_naive =
        apply_unified_diff_naive(original, patch).expect("naive impl should apply cleanly");
    assert_eq!(
        warm_opt, warm_naive,
        "optimized and naive apply_unified_diff disagreed on 1000-line patch output"
    );
    drop(warm_opt);
    drop(warm_naive);

    let before = snapshot();
    let optimized = apply_unified_diff(original, patch).unwrap();
    let (opt_allocs, opt_bytes) = delta(before);
    drop(optimized);

    let before = snapshot();
    let naive = apply_unified_diff_naive(original, patch).unwrap();
    let (naive_allocs, naive_bytes) = delta(before);
    drop(naive);

    eprintln!(
        "F-111 allocation budget: optimized={opt_allocs} allocs ({opt_bytes} bytes), \
         naive={naive_allocs} allocs ({naive_bytes} bytes), \
         ratio={:.2}x",
        naive_allocs as f64 / opt_allocs.max(1) as f64
    );

    assert!(
        opt_allocs * REQUIRED_RATIO <= naive_allocs,
        "F-111 allocation budget regression: optimized impl did {opt_allocs} allocations, \
         naive did {naive_allocs}; required ≥{REQUIRED_RATIO}× reduction \
         (i.e. optimized ≤ {} allocations)",
        naive_allocs / REQUIRED_RATIO
    );
}

// ---------------------------------------------------------------------------
// Criterion benchmarks — report wall-clock deltas.
// ---------------------------------------------------------------------------

fn bench_apply_unified_diff(c: &mut Criterion) {
    let original = build_file();
    let patch = build_patch(&original);

    // Gate: if we regress on allocations, fail the bench binary loudly before
    // criterion prints numbers.
    assert_allocation_budget(&original, &patch);

    let mut group = c.benchmark_group("apply_unified_diff");
    group.sample_size(10);
    group.bench_function("optimized", |b| {
        b.iter(|| {
            let out = apply_unified_diff(black_box(&original), black_box(&patch)).unwrap();
            black_box(out);
        })
    });
    group.bench_function("naive_pre_f111", |b| {
        b.iter(|| {
            let out = apply_unified_diff_naive(black_box(&original), black_box(&patch)).unwrap();
            black_box(out);
        })
    });
    group.finish();
}

criterion_group!(benches, bench_apply_unified_diff);
criterion_main!(benches);
