//! F-569: HTTP-transport allocation benches.
//!
//! Two allocation hot paths in `crates/forge-mcp/src/transport/http.rs`:
//!
//! 1. **Per-server `reqwest::Client` construction.** `Http::connect` used
//!    to build a fresh `reqwest::Client` per MCP server — each one with
//!    its own DNS cache, TLS root store, and connection pool (≈ hundreds
//!    of KB of resident state). The fix shares a single process-wide
//!    `Client` (cloned cheaply via its internal `Arc`). The
//!    `connect_n_http_servers` group exercises that, reporting both
//!    allocation count *and* allocated bytes — the latter is the
//!    DoD's ≥50% RSS-at-10-servers proxy without an external profiler.
//!
//! 2. **SSE per-event accumulator drain.** `open_and_read_sse` used
//!    `buf.drain(..end.frame_end).collect::<Vec<u8>>()` per event — one
//!    allocation per event. The fix parses in place against the
//!    accumulator and then drains. The `sse_decode_10k_events` group
//!    walks 10 000 synthetic SSE events through both shapes and reports
//!    per-event alloc counts; the DoD asks for ≥30% reduction at 10k.
//!
//! Counting-allocator pattern follows `crates/forge-ipc/benches/frame.rs`
//! (PR #583): a `CountingAlloc` global wraps `System` and bumps two
//! atomics on every `alloc`. We sample those before/after each bench
//! body and print a one-shot before-criterion summary (criterion only
//! tracks wall time). The bench body itself is what criterion times.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};

// ── Counting allocator ───────────────────────────────────────────────────

struct CountingAlloc;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

fn alloc_count() -> usize {
    ALLOC_COUNT.load(Ordering::Relaxed)
}
fn alloc_bytes() -> usize {
    ALLOC_BYTES.load(Ordering::Relaxed)
}

// ── 1. connect_n_http_servers ────────────────────────────────────────────
//
// Baseline: every "server" gets its own `reqwest::Client::builder()` —
// the pre-F-569 shape of `Http::connect`. Measures the cost of N
// independent connection pools + DNS caches + TLS root stores.
//
// Shared: one `Client::builder()` build, then N `clone()`s. This is
// what the post-fix `shared_client()` singleton does (the singleton
// itself is `OnceLock`-amortised across the process; we model just the
// per-server work). Cloning a `Client` is an `Arc` bump.

fn baseline_build_n_clients(n: usize) -> Vec<reqwest::Client> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let c = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("client build");
        out.push(c);
    }
    out
}

fn shared_clone_n_handles(base: &reqwest::Client, n: usize) -> Vec<reqwest::Client> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(base.clone());
    }
    out
}

fn report_connect_alloc_reduction() {
    eprintln!("[F-569] connect_n_http_servers — alloc count + bytes per N (lower is better)");
    let base = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("shared base client");
    for n in [1usize, 4, 16] {
        // Baseline: N fresh `Client` builds.
        let n_before = alloc_count();
        let b_before = alloc_bytes();
        let baseline = baseline_build_n_clients(n);
        let baseline_n = alloc_count() - n_before;
        let baseline_b = alloc_bytes() - b_before;
        drop(baseline);

        // Shared: N `clone()`s of one already-built `Client`.
        let n_before = alloc_count();
        let b_before = alloc_bytes();
        let shared = shared_clone_n_handles(&base, n);
        let shared_n = alloc_count() - n_before;
        let shared_b = alloc_bytes() - b_before;
        drop(shared);

        let pct_n = if baseline_n == 0 {
            0.0
        } else {
            100.0 * (baseline_n.saturating_sub(shared_n)) as f64 / baseline_n as f64
        };
        let pct_b = if baseline_b == 0 {
            0.0
        } else {
            100.0 * (baseline_b.saturating_sub(shared_b)) as f64 / baseline_b as f64
        };
        eprintln!(
            "[F-569]   N={:>2}: count {:>6} -> {:>6} ({:.1}% red) | bytes {:>9} -> {:>9} ({:.1}% red)",
            n, baseline_n, shared_n, pct_n, baseline_b, shared_b, pct_b
        );
    }
}

fn bench_connect(c: &mut Criterion) {
    let mut group = c.benchmark_group("connect_n_http_servers");
    group.measurement_time(Duration::from_secs(4));

    // Bench input is the server count `n` we'd see in a `.mcp.json`. The
    // upper bound (16) covers a heavily-configured power-user setup.
    for n in [1usize, 4, 16] {
        let base = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .expect("base client");

        group.bench_with_input(
            BenchmarkId::new("baseline_per_server_build", n),
            &n,
            |b, &n| {
                b.iter(|| {
                    let v = baseline_build_n_clients(black_box(n));
                    black_box(v);
                });
            },
        );

        group.bench_with_input(BenchmarkId::new("shared_handle_clone", n), &n, |b, &n| {
            b.iter(|| {
                let v = shared_clone_n_handles(black_box(&base), black_box(n));
                black_box(v);
            });
        });
    }

    group.finish();
}

// ── 2. sse_decode_10k_events ─────────────────────────────────────────────
//
// Synthetic accumulator: 10 000 minimal SSE events back-to-back. Each
// event is `data: {"k":<i>}\n\n` — the JSON-RPC notification shape an
// MCP server emits for progress / `notifications/*`. Two paths:
//
// * `baseline_drain_collect`: the pre-fix shape — `drain(..frame_end)
//   .collect::<Vec<u8>>()` per event, then parse the owned bytes.
// * `inplace_drain`: post-fix — parse `&buf[..event_end]` first, then
//   `buf.drain(..frame_end)` to advance.
//
// We don't go through reqwest here — the bench is purely about the
// frame-extraction allocator behaviour. `parse_event_data` is the
// shared scanner used inside `open_and_read_sse`; we re-implement it
// in line below to avoid widening forge-mcp's public API just to
// expose it for benchmarking.

fn build_synthetic_sse(events: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(events * 24);
    for i in 0..events {
        // Realistic notification payload: small JSON object, LF-terminated
        // boundary (matches what the wiremock fixture in
        // `tests/http_roundtrip.rs::post_roundtrip_and_sse_notification`
        // produces).
        v.extend_from_slice(b"data: {\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"params\":{\"i\":");
        v.extend_from_slice(i.to_string().as_bytes());
        v.extend_from_slice(b"}}\n\n");
    }
    v
}

/// Re-implementation of the http transport's frame-boundary scanner.
/// Kept private to the bench because exposing the real one publicly
/// would widen forge-mcp's surface (see F-569 PR body).
fn find_event_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n");
    let lf = buf.windows(2).position(|w| w == b"\n\n");
    match (crlf, lf) {
        (Some(c), Some(l)) if c <= l => Some((c + 4, 4)),
        (_, Some(l)) => Some((l + 2, 2)),
        (Some(c), None) => Some((c + 4, 4)),
        (None, None) => None,
    }
}

/// Re-implementation of `parse_event_data` — see comment above.
fn parse_event_data(event_bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(event_bytes).ok()?;
    let mut data = String::new();
    let mut had_data = false;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("data:") {
            let rest = rest.strip_prefix(' ').unwrap_or(rest);
            if had_data {
                data.push('\n');
            }
            data.push_str(rest);
            had_data = true;
        }
    }
    if had_data {
        Some(data)
    } else {
        None
    }
}

fn baseline_drain_collect(buf: &mut Vec<u8>) -> usize {
    // Mirrors the pre-F-569 inner loop in `open_and_read_sse`: drain
    // each frame into a fresh `Vec<u8>` per event, parse the owned
    // bytes, advance.
    let mut emitted = 0;
    while let Some((frame_end, delim_len)) = find_event_boundary(buf) {
        let raw_event = buf.drain(..frame_end).collect::<Vec<u8>>();
        let event_bytes = &raw_event[..raw_event.len() - delim_len];
        if let Some(payload) = parse_event_data(event_bytes) {
            black_box(payload);
            emitted += 1;
        }
    }
    emitted
}

fn inplace_drain(buf: &mut Vec<u8>) -> usize {
    // Post-F-569 shape: parse against the accumulator, then drain.
    let mut emitted = 0;
    while let Some((frame_end, delim_len)) = find_event_boundary(buf) {
        let event_end = frame_end - delim_len;
        let payload = parse_event_data(&buf[..event_end]);
        buf.drain(..frame_end);
        if let Some(payload) = payload {
            black_box(payload);
            emitted += 1;
        }
    }
    emitted
}

fn report_sse_alloc_reduction() {
    const N: usize = 10_000;
    let stream = build_synthetic_sse(N);

    // Warm any one-shot allocations (vec backing, etc) so we're measuring
    // the steady-state per-event cost on the second pass.
    let mut warmup = stream.clone();
    let _ = baseline_drain_collect(&mut warmup);
    let mut warmup = stream.clone();
    let _ = inplace_drain(&mut warmup);

    let mut buf = stream.clone();
    let n_before = alloc_count();
    let b_before = alloc_bytes();
    let emitted = baseline_drain_collect(&mut buf);
    let baseline_n = alloc_count() - n_before;
    let baseline_b = alloc_bytes() - b_before;
    assert_eq!(emitted, N, "baseline must emit all {N} events");

    let mut buf = stream.clone();
    let n_before = alloc_count();
    let b_before = alloc_bytes();
    let emitted = inplace_drain(&mut buf);
    let inplace_n = alloc_count() - n_before;
    let inplace_b = alloc_bytes() - b_before;
    assert_eq!(emitted, N, "inplace must emit all {N} events");

    let pf_base = baseline_n as f64 / N as f64;
    let pf_inp = inplace_n as f64 / N as f64;
    let pct_n = if baseline_n == 0 {
        0.0
    } else {
        100.0 * (baseline_n.saturating_sub(inplace_n)) as f64 / baseline_n as f64
    };
    let pct_b = if baseline_b == 0 {
        0.0
    } else {
        100.0 * (baseline_b.saturating_sub(inplace_b)) as f64 / baseline_b as f64
    };
    eprintln!(
        "[F-569] sse_decode_{}_events: count {} -> {} ({:.2}->{:.2}/event, {:.1}% red) | \
         bytes {} -> {} ({:.1}% red)",
        N, baseline_n, inplace_n, pf_base, pf_inp, pct_n, baseline_b, inplace_b, pct_b
    );
}

fn bench_sse(c: &mut Criterion) {
    let stream_10k = build_synthetic_sse(10_000);
    let mut group = c.benchmark_group("sse_decode_10k_events");
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("baseline_drain_collect", |b| {
        b.iter(|| {
            let mut buf = stream_10k.clone();
            let n = baseline_drain_collect(&mut buf);
            black_box(n);
        });
    });

    group.bench_function("inplace_drain", |b| {
        b.iter(|| {
            let mut buf = stream_10k.clone();
            let n = inplace_drain(&mut buf);
            black_box(n);
        });
    });

    group.finish();
}

fn bench_root(c: &mut Criterion) {
    // Print one-shot reduction summaries before criterion's wall-time
    // benches so the DoD-relevant percentages land in the output of
    // every `cargo bench -p forge-mcp` run.
    report_connect_alloc_reduction();
    report_sse_alloc_reduction();

    bench_connect(c);
    bench_sse(c);
}

criterion_group!(benches, bench_root);
criterion_main!(benches);
