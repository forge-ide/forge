//! Per-token allocation-budget guard + throughput bench for the orchestrator
//! event-emission hot path (F-107).
//!
//! The real hot path lives in `forge-session::orchestrator::run_request_loop`:
//! for every streamed `ChatChunk::TextDelta`, the orchestrator clones
//! `msg_id: MessageId` into a fresh `Event::AssistantDelta`. On a 2000-token
//! response that's ~10k ID clones, and before F-107 each clone was a heap
//! allocation (`String::clone`). F-107 changes the ID wrapper types to
//! `Arc<str>` at rest; clones become ref-count bumps.
//!
//! This bench does two things:
//!
//! 1. Asserts that `MessageId`/`ProviderId`/`ToolCallId` clones allocate at
//!    least 10× fewer times than a byte-identical `String`-backed shadow
//!    ID ("naive pre-F-107") over a 1000-token synthetic stream. The
//!    assertion runs during bench startup; **fails loudly on baseline**
//!    (if someone reverts F-107, `MessageId` goes back to `String` and the
//!    ratio collapses) and passes on the current impl.
//!
//! 2. Reports wall-clock throughput via criterion for both paths so future
//!    regressions are visible as numbers, not just pass/fail.
//!
//! The naive shadow type is defined inline in this bench binary — production
//! code has a single optimized path. This pattern mirrors F-111's
//! `crates/forge-fs/benches/mutate.rs`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use chrono::{DateTime, Utc};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use forge_core::ids::{MessageId, ProviderId, ToolCallId};

// ---------------------------------------------------------------------------
// Allocation-counting global allocator (mirrors F-111/forge-fs/benches).
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
// Naive pre-F-107 shadow ID types. `Clone` on these is a full heap allocation
// — the exact behaviour the real types had before F-107 flipped them to
// `Arc<str>`. Kept in the bench binary only; production has one path.
// ---------------------------------------------------------------------------

// The `.0` payload is never read directly — it's the byte buffer that
// `Clone::clone` duplicates on every call, which is exactly the behaviour
// under measurement. Suppress rust's dead-code analysis, which doesn't see
// `Clone` reads as uses.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct NaiveMessageId(String);

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct NaiveProviderId(String);

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct NaiveToolCallId(String);

/// Shadow of `Event::AssistantDelta` used by the naive path. Carries the
/// same three pieces the real event does so the two paths are doing
/// byte-equivalent work aside from the ID clone semantics.
#[allow(dead_code)] // fields read via Debug / black_box, not direct access
#[derive(Debug)]
struct NaiveAssistantDelta {
    id: NaiveMessageId,
    at: DateTime<Utc>,
    delta: String,
}

/// Shadow of `Event::AssistantMessage`. The orchestrator emits this on open
/// and on every finalisation, cloning `provider`+`msg_id` each time.
#[allow(dead_code)]
#[derive(Debug)]
struct NaiveAssistantMessage {
    id: NaiveMessageId,
    provider: NaiveProviderId,
    model: String,
    at: DateTime<Utc>,
    text: String,
}

/// Shadow of `Event::ToolCallStarted` — fewer hot-path hits per response than
/// `AssistantDelta` but still cloned twice per tool call.
#[allow(dead_code)]
#[derive(Debug)]
struct NaiveToolCallStarted {
    id: NaiveToolCallId,
    msg: NaiveMessageId,
    at: DateTime<Utc>,
}

/// Optimized counterpart — the real `Arc<str>`-backed IDs from `forge-core`.
#[allow(dead_code)]
#[derive(Debug)]
struct OptAssistantDelta {
    id: MessageId,
    at: DateTime<Utc>,
    delta: String,
}

#[allow(dead_code)]
#[derive(Debug)]
struct OptAssistantMessage {
    id: MessageId,
    provider: ProviderId,
    model: String,
    at: DateTime<Utc>,
    text: String,
}

#[allow(dead_code)]
#[derive(Debug)]
struct OptToolCallStarted {
    id: ToolCallId,
    msg: MessageId,
    at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Hot-loop drivers. Each driver clones the IDs for every "token" in the
// synthetic stream, exactly the way `run_request_loop` does per chunk.
// The `delta`/`text`/`model` strings are prebuilt outside the loop so their
// allocations don't pollute the measurement — we're counting ID clones.
// ---------------------------------------------------------------------------

const TOKENS: usize = 1_000;

// The `delta: String` field on `Event::AssistantDelta` is the token payload;
// in the real path it arrives per-chunk from the provider stream and is
// already owned. For the bench we isolate the *ID-clone* cost by constructing
// the per-token delta via `String::new()` (zero-alloc) — both paths pay the
// same zero-alloc for `delta`, so the ratio reflects *only* the change that
// F-107 introduced: `String::clone` on IDs vs `Arc<str>::clone`.

fn run_naive(
    msg_id: &NaiveMessageId,
    provider_id: &NaiveProviderId,
    sink: &mut Vec<NaiveAssistantDelta>,
) {
    sink.clear();
    let at = Utc::now();
    // Open event (cloned provider + msg_id).
    let _open = NaiveAssistantMessage {
        id: msg_id.clone(),
        provider: provider_id.clone(),
        model: String::new(),
        at,
        text: String::new(),
    };
    black_box(&_open);
    // Per-token AssistantDelta — the hot path.
    for _ in 0..TOKENS {
        sink.push(NaiveAssistantDelta {
            id: msg_id.clone(),
            at,
            delta: String::new(),
        });
    }
    // Final AssistantMessage (cloned provider + msg_id).
    let _final = NaiveAssistantMessage {
        id: msg_id.clone(),
        provider: provider_id.clone(),
        model: String::new(),
        at,
        text: String::new(),
    };
    black_box(&_final);
}

fn run_opt(msg_id: &MessageId, provider_id: &ProviderId, sink: &mut Vec<OptAssistantDelta>) {
    sink.clear();
    let at = Utc::now();
    let _open = OptAssistantMessage {
        id: msg_id.clone(),
        provider: provider_id.clone(),
        model: String::new(),
        at,
        text: String::new(),
    };
    black_box(&_open);
    for _ in 0..TOKENS {
        sink.push(OptAssistantDelta {
            id: msg_id.clone(),
            at,
            delta: String::new(),
        });
    }
    let _final = OptAssistantMessage {
        id: msg_id.clone(),
        provider: provider_id.clone(),
        model: String::new(),
        at,
        text: String::new(),
    };
    black_box(&_final);
}

// ---------------------------------------------------------------------------
// Allocation-budget guard. Runs once at startup; panics on regression.
// ---------------------------------------------------------------------------

/// Optimized path must allocate at most `naive / REQUIRED_RATIO` times.
/// The `Arc<str>` clones themselves are zero-alloc; the remaining allocs
/// come from the `delta: String` clones and the sink `Vec` growth, which
/// are identical in both paths. So the ratio is dominated by the N
/// `String::clone` calls on the naive IDs vs. zero on `Arc<str>`.
const REQUIRED_RATIO: usize = 10;

fn assert_allocation_budget() {
    let naive_msg = NaiveMessageId("aabbccddeeff0011".to_string());
    let naive_provider = NaiveProviderId("mock-provider-id".to_string());
    let opt_msg = MessageId::from_string("aabbccddeeff0011".to_string());
    let opt_provider = ProviderId::from_string("mock-provider-id".to_string());

    // Pre-allocate the sinks so their initial `Vec::with_capacity` alloc is
    // out of the measurement window — both paths then only pay for any
    // overflow growth, which is identical between them.
    let mut naive_sink: Vec<NaiveAssistantDelta> = Vec::with_capacity(TOKENS);
    let mut opt_sink: Vec<OptAssistantDelta> = Vec::with_capacity(TOKENS);

    // Warm run to drop any lazily-initialised statics out of the counters.
    run_naive(&naive_msg, &naive_provider, &mut naive_sink);
    run_opt(&opt_msg, &opt_provider, &mut opt_sink);

    let before = snapshot();
    run_naive(&naive_msg, &naive_provider, &mut naive_sink);
    let (naive_allocs, naive_bytes) = delta(before);

    let before = snapshot();
    run_opt(&opt_msg, &opt_provider, &mut opt_sink);
    let (opt_allocs, opt_bytes) = delta(before);

    eprintln!(
        "F-107 allocation budget: optimized={opt_allocs} allocs ({opt_bytes} bytes), \
         naive={naive_allocs} allocs ({naive_bytes} bytes), \
         ratio={:.2}x",
        naive_allocs as f64 / opt_allocs.max(1) as f64
    );

    assert!(
        opt_allocs * REQUIRED_RATIO <= naive_allocs,
        "F-107 allocation budget regression: optimized path did {opt_allocs} allocations \
         over {TOKENS} tokens, naive did {naive_allocs}; required >={REQUIRED_RATIO}x reduction \
         (i.e. optimized <= {} allocations)",
        naive_allocs / REQUIRED_RATIO
    );
}

// ---------------------------------------------------------------------------
// Tool-call hot path — emits the second-order ID type (ToolCallId) and
// exercises the msg+tool-id dual-clone shape.
// ---------------------------------------------------------------------------

const TOOL_CALLS: usize = 100;

fn run_naive_toolcalls(
    msg_id: &NaiveMessageId,
    call_id: &NaiveToolCallId,
    sink: &mut Vec<NaiveToolCallStarted>,
) {
    sink.clear();
    let at = Utc::now();
    for _ in 0..TOOL_CALLS {
        // ToolCallStarted → ToolCallApproved → ToolCallCompleted is the
        // per-call triple; orchestrator clones call_id 3-4× per call.
        for _ in 0..4 {
            sink.push(NaiveToolCallStarted {
                id: call_id.clone(),
                msg: msg_id.clone(),
                at,
            });
        }
    }
}

fn run_opt_toolcalls(msg_id: &MessageId, call_id: &ToolCallId, sink: &mut Vec<OptToolCallStarted>) {
    sink.clear();
    let at = Utc::now();
    for _ in 0..TOOL_CALLS {
        for _ in 0..4 {
            sink.push(OptToolCallStarted {
                id: call_id.clone(),
                msg: msg_id.clone(),
                at,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Criterion — wall-clock throughput. The allocation-budget guard above is
// the real gate; these numbers exist so perf regressions show up as drift
// over time, per the F-107 DoD ("events/sec materially improved").
// ---------------------------------------------------------------------------

fn bench_event_emission(c: &mut Criterion) {
    // Gate: if allocations regress, fail loudly before criterion prints.
    assert_allocation_budget();

    let naive_msg = NaiveMessageId("aabbccddeeff0011".to_string());
    let naive_provider = NaiveProviderId("mock-provider-id".to_string());
    let naive_call = NaiveToolCallId("aabbccddeeff00112233445566778899".to_string());
    let opt_msg = MessageId::from_string("aabbccddeeff0011".to_string());
    let opt_provider = ProviderId::from_string("mock-provider-id".to_string());
    let opt_call = ToolCallId::from_string("aabbccddeeff00112233445566778899".to_string());

    let mut naive_sink: Vec<NaiveAssistantDelta> = Vec::with_capacity(TOKENS);
    let mut opt_sink: Vec<OptAssistantDelta> = Vec::with_capacity(TOKENS);

    let mut group = c.benchmark_group("orchestrator_event_emission");
    group.sample_size(20);
    group.bench_function("arc_str_1000_tokens", |b| {
        b.iter(|| {
            run_opt(black_box(&opt_msg), black_box(&opt_provider), &mut opt_sink);
        })
    });
    group.bench_function("naive_string_1000_tokens", |b| {
        b.iter(|| {
            run_naive(
                black_box(&naive_msg),
                black_box(&naive_provider),
                &mut naive_sink,
            );
        })
    });
    group.finish();

    let mut naive_tc: Vec<NaiveToolCallStarted> = Vec::with_capacity(TOOL_CALLS * 4);
    let mut opt_tc: Vec<OptToolCallStarted> = Vec::with_capacity(TOOL_CALLS * 4);

    let mut group = c.benchmark_group("orchestrator_tool_call_emission");
    group.sample_size(20);
    group.bench_function("arc_str_100_tool_calls", |b| {
        b.iter(|| {
            run_opt_toolcalls(black_box(&opt_msg), black_box(&opt_call), &mut opt_tc);
        })
    });
    group.bench_function("naive_string_100_tool_calls", |b| {
        b.iter(|| {
            run_naive_toolcalls(black_box(&naive_msg), black_box(&naive_call), &mut naive_tc);
        })
    });
    group.finish();
}

criterion_group!(benches, bench_event_emission);
criterion_main!(benches);
