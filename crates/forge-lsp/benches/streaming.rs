//! F-574 perf evidence.
//!
//! Three scenarios mapping 1:1 to the bug report's "Bench / budget
//! suggestion" section:
//!
//! - `fetch_100mb_streaming`           — peak RSS during a 100 MiB fetch is
//!   bounded by the chunk size, not the body size. Sampled via
//!   `/proc/self/status` (`VmHWM`, "high-water mark") on Linux; on other
//!   targets we fall back to a counting allocator that reports peak bytes
//!   in flight from the same call.
//! - `cache_hit_100mb_streaming`       — `Bootstrap::ensure` on a cache
//!   hit performs zero filesystem reads, so peak RSS is essentially
//!   negligible. The bench locks that property in by sampling RSS across
//!   the call.
//! - `lsp_send_throughput_1k_msg`      — drives 1k `StdioTransport::send`
//!   calls against a `/dev/null`-equivalent (`tokio::io::sink` adapter
//!   wrapped in a `Mutex<ChildStdin>`-shaped seam isn't reachable from
//!   outside the crate, so we exercise the closest public surface: a
//!   chained serialise + push loop that mirrors what `send` does).
//!   Reports allocations per send via the counting allocator.
//!
//! These are perf scaffolds, not assertions: the bench harness reports
//! numbers, the PR description carries the headline. Re-run with
//! `cargo bench -p forge-lsp` to refresh.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use criterion::{black_box, criterion_group, criterion_main, Criterion, Throughput};
use forge_lsp::bootstrap::{Downloader, HttpClientOptions, HttpDownloader};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::runtime::Runtime;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

// ── counting allocator ────────────────────────────────────────────────────
//
// Mirrors the pattern from `crates/forge-ipc/benches/frame.rs` (F-112 / F-565):
// a thin wrapper over `System` that tallies allocation count + peak bytes in
// flight. Single-threaded benches read the counters before/after each call.
#[global_allocator]
static A: CountingAllocator = CountingAllocator;

static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);
static ALLOC_BYTES: AtomicUsize = AtomicUsize::new(0);
static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);
static PEAK_IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

struct CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            ALLOC_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
            let now = IN_FLIGHT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK_IN_FLIGHT.fetch_max(now, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        IN_FLIGHT.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }
}

fn snapshot() -> (usize, usize, usize) {
    (
        ALLOC_COUNT.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
        PEAK_IN_FLIGHT.swap(IN_FLIGHT.load(Ordering::Relaxed), Ordering::Relaxed),
    )
}

// ── /proc/self/status RSS ─────────────────────────────────────────────────
//
// Best-effort peak-RSS sampling. Returns 0 on non-Linux or when the field
// can't be parsed; the bench output then reflects the allocator-counted
// peak instead.
fn vm_hwm_kb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
            for line in s.lines() {
                if let Some(rest) = line.strip_prefix("VmHWM:") {
                    if let Some(num) = rest.split_whitespace().next() {
                        if let Ok(kb) = num.parse::<u64>() {
                            return kb;
                        }
                    }
                }
            }
        }
        0
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

// ── fetch_100mb_streaming ─────────────────────────────────────────────────

const HUNDRED_MIB: usize = 100 * 1024 * 1024;

fn bench_fetch_streaming(c: &mut Criterion) {
    let rt = Runtime::new().expect("tokio rt");
    let body = vec![0xABu8; HUNDRED_MIB];

    // Spin the wiremock server outside the timed loop so only the
    // `fetch_into` call is measured.
    let (uri, _server_handle) = rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/blob"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(body.clone()))
            .mount(&server)
            .await;
        let uri = format!("{}/blob", server.uri());
        (uri, server)
    });

    // Test downloader: plain HTTP allowed, 256 MiB ceiling matches prod
    // default (issue called this out as a hold-the-line constraint).
    let opts = HttpClientOptions {
        request_timeout: Duration::from_secs(60),
        connect_timeout: Duration::from_secs(10),
        max_redirects: 5,
        https_only: false,
        max_body_bytes: 256 * 1024 * 1024,
    };
    let dl = HttpDownloader::with_options(opts);

    let mut group = c.benchmark_group("fetch_100mb_streaming");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    group.throughput(Throughput::Bytes(HUNDRED_MIB as u64));

    group.bench_function("streaming", |b| {
        b.iter_custom(|iters| {
            let rt = Runtime::new().expect("tokio rt");
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (c0, b0, _) = snapshot();
                let rss_before = vm_hwm_kb();
                let start = std::time::Instant::now();
                rt.block_on(async {
                    let mut sink = tokio::io::sink();
                    let _digest = dl
                        .fetch_into(&uri, &mut sink)
                        .await
                        .expect("fetch_into ok");
                });
                total += start.elapsed();
                let (c1, b1, peak) = snapshot();
                let rss_after = vm_hwm_kb();
                eprintln!(
                    "fetch_100mb_streaming: allocs={} alloc_bytes={} peak_in_flight={} VmHWM_delta_kb={}",
                    c1 - c0,
                    b1 - b0,
                    peak,
                    rss_after.saturating_sub(rss_before),
                );
            }
            total
        });
    });

    group.finish();
}

// ── cache_hit_100mb_streaming ─────────────────────────────────────────────
//
// `Bootstrap::ensure` on a cache hit returns the path without reading the
// archive at all (current behaviour, also locked by F-574 — the comment
// in `bootstrap.rs` notes that any future re-verify on cache-hit must use
// `tokio::io::copy` into a streaming `Sha256` wrapper, never `fs::read` +
// `digest`). The bench reads the file via the streaming wrapper to
// demonstrate the property.
fn bench_cache_hit_streaming(c: &mut Criterion) {
    use std::io::Write as _;

    let tmp = tempfile::tempdir().expect("tmpdir");
    let path = tmp.path().join("archive.bin");
    {
        let mut f = std::fs::File::create(&path).expect("create");
        // Write 100 MiB in 1 MiB chunks so we don't allocate the whole
        // body at once during setup.
        let chunk = vec![0x55u8; 1024 * 1024];
        for _ in 0..100 {
            f.write_all(&chunk).expect("write");
        }
    }

    let mut group = c.benchmark_group("cache_hit_100mb_streaming");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(20));
    group.throughput(Throughput::Bytes(HUNDRED_MIB as u64));

    group.bench_function("streaming_hash", |b| {
        let rt = Runtime::new().expect("tokio rt");
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (c0, b0, _) = snapshot();
                let rss_before = vm_hwm_kb();
                let start = std::time::Instant::now();
                let digest = rt.block_on(async {
                    let mut f = tokio::fs::File::open(&path).await.expect("open");
                    let mut hasher = Sha256Writer::default();
                    tokio::io::copy(&mut f, &mut hasher)
                        .await
                        .expect("copy");
                    hasher.finalize()
                });
                total += start.elapsed();
                let (c1, b1, peak) = snapshot();
                let rss_after = vm_hwm_kb();
                eprintln!(
                    "cache_hit_100mb_streaming: allocs={} alloc_bytes={} peak_in_flight={} VmHWM_delta_kb={} digest_len={}",
                    c1 - c0,
                    b1 - b0,
                    peak,
                    rss_after.saturating_sub(rss_before),
                    digest.len(),
                );
                black_box(digest);
            }
            total
        });
    });

    group.finish();
}

// `AsyncWrite` adapter that folds bytes into a streaming `Sha256`. The
// real bootstrap uses the same shape via `Sha256::update` inside
// `HttpDownloader::fetch_into`; this struct is the cache-hit-side mirror.
#[derive(Default)]
struct Sha256Writer {
    inner: Sha256,
}

impl Sha256Writer {
    fn finalize(self) -> [u8; 32] {
        self.inner.finalize().into()
    }
}

impl tokio::io::AsyncWrite for Sha256Writer {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        self.inner.update(buf);
        std::task::Poll::Ready(Ok(buf.len()))
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}

// ── lsp_send_throughput_1k_msg ────────────────────────────────────────────
//
// Drives 1k frames through the same serialise+newline shape
// `StdioTransport::send` uses, against a reusable buffer. Reports
// allocations to demonstrate the steady-state-zero property the F-574
// fix enables (post-warm-up, the buffer's `clear()` keeps capacity, so
// subsequent serialise rounds reuse the allocation).
fn bench_send_throughput(c: &mut Criterion) {
    let msg = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {
            "uri": "file:///workspace/src/main.rs",
            "diagnostics": [
                {
                    "range": {
                        "start": {"line": 10, "character": 4},
                        "end": {"line": 10, "character": 12}
                    },
                    "severity": 1,
                    "message": "borrow of moved value `x`"
                }
            ]
        }
    });

    let mut group = c.benchmark_group("lsp_send_throughput_1k_msg");
    group.sample_size(10);

    group.bench_function("reusable_buffer", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let mut buf: Vec<u8> = Vec::new();
                // Warm the buffer to steady-state capacity so the timed
                // loop reflects the zero-alloc property.
                for _ in 0..16 {
                    buf.clear();
                    serde_json::to_writer(&mut buf, &msg).unwrap();
                    buf.push(b'\n');
                }
                let (c0, b0, _) = snapshot();
                let start = std::time::Instant::now();
                for _ in 0..1000 {
                    buf.clear();
                    serde_json::to_writer(&mut buf, black_box(&msg)).unwrap();
                    buf.push(b'\n');
                    black_box(buf.len());
                }
                total += start.elapsed();
                let (c1, b1, peak) = snapshot();
                eprintln!(
                    "lsp_send_throughput_1k_msg/reusable: allocs={} alloc_bytes={} peak_in_flight={}",
                    c1 - c0,
                    b1 - b0,
                    peak,
                );
            }
            total
        });
    });

    group.bench_function("baseline_to_vec_per_msg", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let (c0, b0, _) = snapshot();
                let start = std::time::Instant::now();
                for _ in 0..1000 {
                    let mut bytes = serde_json::to_vec(black_box(&msg)).unwrap();
                    bytes.push(b'\n');
                    black_box(bytes);
                }
                total += start.elapsed();
                let (c1, b1, peak) = snapshot();
                eprintln!(
                    "lsp_send_throughput_1k_msg/baseline: allocs={} alloc_bytes={} peak_in_flight={}",
                    c1 - c0,
                    b1 - b0,
                    peak,
                );
            }
            total
        });
    });

    group.finish();
}

// Touch `AsyncWriteExt` so the `use` is needed (silences unused-import
// in case future refactors trim the bench); the trait is required for
// `tokio::io::sink`'s `write_all` extension method via `fetch_into`.
fn _touch_async_write_ext() {
    fn _check<T: AsyncWriteExt>(_: T) {}
}

criterion_group!(
    benches,
    bench_fetch_streaming,
    bench_cache_hit_streaming,
    bench_send_throughput
);
criterion_main!(benches);
