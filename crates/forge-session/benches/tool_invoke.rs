//! Cross-session jitter guard for the F-106 `spawn_blocking` fix.
//!
//! This bench does two things:
//!
//! 1. Asserts that wrapping the synchronous `forge_fs::read_file` call in
//!    `tokio::task::spawn_blocking` preserves progress in sibling async
//!    tasks sharing a single tokio worker. It measures the counter
//!    advanced by N cooperative "streaming" tasks while one task performs
//!    a ~10 MB file read. Without the wrap, the blocking read monopolises
//!    the sole worker for ~50-100 ms and the counter tasks stall during
//!    that window; with the wrap, the blocking work runs on the
//!    `spawn_blocking` pool and the counter tasks keep making progress.
//!    The assertion is a ratio (`fixed / naive >= REQUIRED_RATIO`) so the
//!    guard scales with hardware and does not flake on varied CI hosts.
//!
//! 2. Reports wall-clock timing via criterion for both variants so
//!    regressions show up as numbers, not just pass/fail.
//!
//! Single-worker runtime is load-bearing: on a multi-worker runtime, a
//! blocked worker does not visibly starve tasks running on the others
//! and the naive-vs-fixed delta disappears. All scenarios in this bench
//! build a runtime with `worker_threads = 1`.
//!
//! Scope note: the bench calls `forge_fs::read_file` directly rather
//! than going through `FsReadTool::invoke`. It tests the *principle*
//! the fix embodies (sync blocking fs call at an async/sync boundary),
//! not the specific call-site wiring in `fs_read.rs` / `fs_write.rs` /
//! `fs_edit.rs`. A regression that removes `spawn_blocking` from one
//! of those tools won't trip this bench — code review is the catch for
//! that.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use criterion::{criterion_group, criterion_main, Criterion};
use tokio::runtime::Builder;

// ---------------------------------------------------------------------------
// Fixture: a ~50 MB file on disk. The finding cites 10 MB as the minimum
// "hostile" size (~50-100 ms per read), but on warm page cache + SSD a
// 10 MB read can drop to ~5-10 ms. 50 MB pushes each read into the
// 30-60 ms range even under favourable cache conditions, which — combined
// with a loop of `READ_ITERATIONS` — keeps the worker blocked for the
// bulk of the measurement window on the naive variant. The fixture also
// overrides `forge_fs::Limits::max_read_bytes` so the read itself does
// not trip the 10 MiB production cap.
// ---------------------------------------------------------------------------

const FILE_BYTES: usize = 50 * 1024 * 1024;

fn bench_limits() -> forge_fs::Limits {
    forge_fs::Limits {
        max_read_bytes: (FILE_BYTES as u64) + 1,
        max_write_bytes: (FILE_BYTES as u64) + 1,
    }
}

fn make_fixture() -> (tempfile::TempDir, String, Vec<String>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("big.bin");
    std::fs::write(&path, vec![b'x'; FILE_BYTES]).expect("write fixture");
    let canonical = std::fs::canonicalize(&path).expect("canonicalize");
    let path_str = canonical.to_str().unwrap().to_string();
    let allowed = vec![canonical.to_str().unwrap().to_string()];
    (dir, path_str, allowed)
}

// ---------------------------------------------------------------------------
// Streaming-counter tasks. Mirrors a provider stream yielding chunks to the
// runtime: increment a shared atomic, yield to the scheduler, repeat.
// `yield_now()` is load-bearing — without it, a tight loop would monopolise
// the single worker and starve the blocking read of even a chance to start.
// ---------------------------------------------------------------------------

const STREAMING_TASKS: usize = 4;
const MEASUREMENT_MS: u64 = 1000;
/// Number of back-to-back `read_file` calls the blocking task issues.
/// Even at 50 MB per read, a warm page cache can service a single read in
/// ~30-60 ms — short enough that a single read would leave most of the
/// measurement window free for counter progress on the naive variant.
/// Looping keeps the worker blocked for the bulk of the window and
/// surfaces the naive-vs-fixed delta as a stable ≥5× ratio (the required
/// guard is a looser 3× to stay robust against busy CI hosts).
const READ_ITERATIONS: usize = 20;

async fn run_streaming_tasks_for(
    counter: Arc<AtomicU64>,
    duration: Duration,
) -> Vec<tokio::task::JoinHandle<()>> {
    let deadline = tokio::time::Instant::now() + duration;
    let mut handles = Vec::with_capacity(STREAMING_TASKS);
    for _ in 0..STREAMING_TASKS {
        let c = counter.clone();
        handles.push(tokio::spawn(async move {
            while tokio::time::Instant::now() < deadline {
                c.fetch_add(1, Ordering::Relaxed);
                tokio::task::yield_now().await;
            }
        }));
    }
    handles
}

// ---------------------------------------------------------------------------
// Scenario A (naive / pre-F-106): call `forge_fs::read_file` synchronously
// from the async context. Blocks the sole worker for the duration of the
// read. Returns the counter value accumulated during the measurement window.
// ---------------------------------------------------------------------------

fn scenario_naive(path: &str, allowed: &[String]) -> u64 {
    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("build runtime");

    let counter = Arc::new(AtomicU64::new(0));
    let counter_in = counter.clone();
    let path = path.to_string();
    let allowed = allowed.to_vec();

    rt.block_on(async move {
        let handles =
            run_streaming_tasks_for(counter_in, Duration::from_millis(MEASUREMENT_MS)).await;

        // Spawn the blocking fs read as an async task on the same single
        // worker. This is the pre-F-106 shape — no `spawn_blocking`.
        let limits = bench_limits();
        let read_handle = tokio::spawn(async move {
            // Short delay so streaming tasks get to register progress
            // before the blocking read takes over the worker.
            tokio::time::sleep(Duration::from_millis(20)).await;
            for _ in 0..READ_ITERATIONS {
                let _ = forge_fs::read_file(&path, &allowed, &limits);
            }
        });

        for h in handles {
            let _ = h.await;
        }
        let _ = read_handle.await;
    });

    counter.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Scenario B (F-106 fix): wrap the synchronous read in `spawn_blocking`.
// The blocking work moves to the blocking pool, leaving the worker free to
// drive the streaming tasks — they should keep making progress throughout.
// ---------------------------------------------------------------------------

fn scenario_fixed(path: &str, allowed: &[String]) -> u64 {
    let rt = Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("build runtime");

    let counter = Arc::new(AtomicU64::new(0));
    let counter_in = counter.clone();
    let path = path.to_string();
    let allowed = allowed.to_vec();

    rt.block_on(async move {
        let handles =
            run_streaming_tasks_for(counter_in, Duration::from_millis(MEASUREMENT_MS)).await;

        let limits = bench_limits();
        let read_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            for _ in 0..READ_ITERATIONS {
                let path = path.clone();
                let allowed = allowed.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    forge_fs::read_file(&path, &allowed, &limits)
                })
                .await;
            }
        });

        for h in handles {
            let _ = h.await;
        }
        let _ = read_handle.await;
    });

    counter.load(Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// Budget guard. Runs once at bench startup; panics on regression. The ratio
// is deliberately conservative — on a healthy fix we've observed ≥10×; we
// require only 3× to stay robust against busy CI hosts while still catching
// the regression where the blocking call monopolises the worker.
// ---------------------------------------------------------------------------

const REQUIRED_RATIO: u64 = 3;

fn assert_jitter_budget(path: &str, allowed: &[String]) {
    // Warm up — first run pays page-cache / runtime-setup costs that would
    // otherwise skew the measurement.
    let _ = scenario_naive(path, allowed);
    let _ = scenario_fixed(path, allowed);

    let naive = scenario_naive(path, allowed);
    let fixed = scenario_fixed(path, allowed);

    eprintln!(
        "F-106 jitter budget: naive={naive} increments, fixed={fixed} increments, \
         ratio={:.2}x (required ≥{REQUIRED_RATIO}x)",
        fixed as f64 / naive.max(1) as f64
    );

    assert!(
        fixed >= naive.saturating_mul(REQUIRED_RATIO),
        "F-106 jitter regression: fixed-variant counter only reached {fixed} \
         increments vs naive {naive}; required ≥{REQUIRED_RATIO}× improvement. \
         A blocking fs call is stalling the tokio worker — check that each \
         fs tool wraps its forge_fs call in tokio::task::spawn_blocking."
    );
}

// ---------------------------------------------------------------------------
// Criterion benchmarks — report wall-clock counter throughput per variant.
// ---------------------------------------------------------------------------

fn bench_tool_invoke(c: &mut Criterion) {
    let (_dir, path, allowed) = make_fixture();

    // Gate: if we regress on jitter, fail the bench binary loudly before
    // criterion prints numbers.
    assert_jitter_budget(&path, &allowed);

    let mut group = c.benchmark_group("tool_invoke_jitter");
    // Each iteration builds a fresh single-worker runtime and runs for
    // ~150 ms, so keep the sample size small to bound wall-clock.
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("naive_pre_f106", |b| {
        b.iter(|| scenario_naive(&path, &allowed));
    });
    group.bench_function("fixed_spawn_blocking", |b| {
        b.iter(|| scenario_fixed(&path, &allowed));
    });
    group.finish();
}

criterion_group!(benches, bench_tool_invoke);
criterion_main!(benches);
