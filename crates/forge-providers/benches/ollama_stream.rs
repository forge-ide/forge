//! F-108 criterion bench — Ollama NDJSON parse_line allocation budget.
//!
//! The NDJSON decode in `ollama::parse_line` is the hottest per-token path in
//! the application: every streamed assistant token flows through it. The old
//! two-step decode (`serde_json::from_str::<Value>` + `.as_str().to_string()`)
//! allocated three Strings per text-delta token, producing allocator pressure
//! and jitter on long responses.
//!
//! This bench feeds 1000 mock NDJSON lines through both the old (baseline)
//! and new (typed) implementations and reports two numbers per pass:
//! - wall-time (via criterion's sampler)
//! - heap allocations (via a counting global allocator)
//!
//! The baseline implementation (`legacy_parse_line`) is inlined below so the
//! bench can compare against it without polluting the production module. The
//! comparison is the DoD's "baseline shows high allocation count, fix shows
//! reduction" — asserted at the end of the bench run.
//!
//! Run locally with `cargo bench -p forge-providers`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};

use criterion::{criterion_group, criterion_main, Criterion};
use forge_providers::ollama::parse_line;
use forge_providers::ChatChunk;

// ── Counting allocator ────────────────────────────────────────────────────────
//
// Wraps the system allocator with a pair of atomic counters so the bench can
// snapshot allocation deltas around a block of work. The counters are
// process-global: the wrapper is installed as `#[global_allocator]` below,
// and `record()` opts in to counting for the duration of a closure. Counting
// is disabled by default so criterion's own warmup / sampler noise doesn't
// accumulate into the reported numbers.

struct CountingAllocator;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static ENABLED: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) != 0 {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        // SAFETY: delegating to the system allocator with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: delegating to the system allocator with the caller's ptr/layout.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static A: CountingAllocator = CountingAllocator;

/// Run `f`, returning `(allocations, bytes)` observed during the call. Only
/// one scope at a time is meaningful; criterion runs benches serially within
/// a group, so that's fine for our purposes.
fn record<F: FnOnce()>(f: F) -> (usize, usize) {
    ALLOCS.store(0, Ordering::Relaxed);
    BYTES.store(0, Ordering::Relaxed);
    ENABLED.store(1, Ordering::Relaxed);
    f();
    ENABLED.store(0, Ordering::Relaxed);
    (
        ALLOCS.load(Ordering::Relaxed),
        BYTES.load(Ordering::Relaxed),
    )
}

// ── Baseline (pre-F-108) implementation ───────────────────────────────────────
//
// Verbatim copy of the pre-fix `parse_line`. Lives here — not in the
// production module — so the production binary doesn't carry both. Kept only
// for the reduction assertion at the end of the bench. If this ever drifts
// from the fixed version's behavior, the bench still serves its purpose
// (measuring the old allocation shape); the correctness contract lives in
// the src/ollama.rs unit tests.
fn legacy_parse_line(line: &str) -> Option<ChatChunk> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;

    if value.get("done").and_then(|d| d.as_bool()) == Some(true) {
        let reason = value
            .get("done_reason")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        return Some(ChatChunk::Done(reason));
    }

    if let Some(first_call) = value
        .get("message")
        .and_then(|m| m.get("tool_calls"))
        .and_then(|tc| tc.as_array())
        .and_then(|arr| arr.first())
    {
        if let Some(func) = first_call.get("function") {
            let name = func.get("name").and_then(|n| n.as_str())?.to_string();
            let args = func
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            return Some(ChatChunk::ToolCall { name, args });
        }
    }

    if let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
    {
        if !content.is_empty() {
            return Some(ChatChunk::TextDelta(content.to_string()));
        }
    }

    None
}

// ── Mock NDJSON corpus ────────────────────────────────────────────────────────
//
// 1000 lines shaped like a realistic Ollama response: mostly text-delta tokens
// (the hot path), with periodic tool-call chunks and a terminal `done` frame.
// Token strings are short (1–6 ASCII chars, no JSON escapes) so the typed
// parser hits its Cow::Borrowed fast path on the delta fields — the common
// case the DoD budgets for.

fn mock_ndjson_lines() -> Vec<String> {
    let text_tokens = [
        "the ", "quick", " brown", " fox ", "jumps", " over ", "a ", "lazy ", "dog", ".", " it ",
        "was ", "a ", "sunny", " day", " in ", "spring", ".",
    ];

    let mut out = Vec::with_capacity(1000);
    for i in 0..999 {
        if i % 97 == 96 {
            // Tool-call chunk — exercises the args-move path.
            out.push(
                r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"fs.read","arguments":{"path":"/tmp/x"}}}]},"done":false}"#
                    .to_string(),
            );
        } else {
            let tok = text_tokens[i % text_tokens.len()];
            out.push(format!(
                r#"{{"message":{{"role":"assistant","content":"{tok}"}},"done":false}}"#
            ));
        }
    }
    out.push(r#"{"model":"llama3","done":true,"done_reason":"stop"}"#.to_string());
    out
}

// ── Benchmarks ────────────────────────────────────────────────────────────────

fn bench_parse_line(c: &mut Criterion) {
    let lines = mock_ndjson_lines();

    let mut group = c.benchmark_group("ollama_parse_line");
    group.throughput(criterion::Throughput::Elements(lines.len() as u64));

    group.bench_function("legacy (Value-tree)", |b| {
        b.iter(|| {
            for line in &lines {
                black_box(legacy_parse_line(black_box(line)));
            }
        });
    });

    group.bench_function("typed (F-108)", |b| {
        b.iter(|| {
            for line in &lines {
                black_box(parse_line(black_box(line)));
            }
        });
    });

    group.finish();
}

// ── Allocation-count assertion ────────────────────────────────────────────────
//
// The criterion time bench covers wall-time. The DoD additionally requires a
// recorded allocation count per 1000 lines. This runs once per bench invocation,
// prints both numbers, and panics if the typed path is not a strict reduction —
// serving as the "bench shows reduction" evidence in the DoD.
fn assert_alloc_reduction(_c: &mut Criterion) {
    let lines = mock_ndjson_lines();

    // Warm up: allocator counters are noisy on first touch (page-in costs).
    for line in &lines {
        black_box(legacy_parse_line(black_box(line)));
        black_box(parse_line(black_box(line)));
    }

    let (legacy_allocs, legacy_bytes) = record(|| {
        for line in &lines {
            black_box(legacy_parse_line(black_box(line)));
        }
    });

    let (typed_allocs, typed_bytes) = record(|| {
        for line in &lines {
            black_box(parse_line(black_box(line)));
        }
    });

    println!(
        "F-108 allocation budget (1000 lines):\n  \
         legacy Value-tree: {legacy_allocs} allocations, {legacy_bytes} bytes\n  \
         typed (F-108):     {typed_allocs} allocations, {typed_bytes} bytes"
    );

    assert!(
        typed_allocs < legacy_allocs,
        "F-108 DoD: typed path must allocate less than the legacy Value-tree \
         path — measured legacy={legacy_allocs} typed={typed_allocs}"
    );

    // The text-delta common case carries no JSON escapes in the corpus, so the
    // Cow field borrows from the source buffer and only the emission String
    // allocates. 1000 lines ≈ 1000 text-delta emissions + ~10 tool-call
    // branches. Anything meaningfully above ~1100 allocations means the Cow
    // path is silently owning when it shouldn't.
    assert!(
        typed_allocs < 2 * lines.len(),
        "F-108 DoD: typed path exceeded ~1 allocation per text-delta token — \
         measured {typed_allocs} for {} lines; Cow<'_, str> is likely falling \
         back to Owned on a path that could borrow",
        lines.len()
    );
}

criterion_group!(benches, bench_parse_line, assert_alloc_reduction);
criterion_main!(benches);
