//! F-570: PTY-reader hot-path throughput + allocation-count bench.
//!
//! The `forge-term` reader thread reads from a PTY into a fixed buffer,
//! tees the chunk into the ghostty-vt driver, and forwards it on a tokio
//! `mpsc` channel. Before F-570 each iteration ran two heap allocations
//! (`Vec::to_vec` + `Vec::clone`) per chunk plus a 4 KiB read buffer; the
//! fix swaps to `bytes::Bytes` (one heap copy + a refcount bump for the
//! tee) and bumps the buffer to 16 KiB.
//!
//! ## Why this bench measures allocations, not wall-time
//!
//! The actual `TerminalSession::spawn` path runs a real PTY + child
//! process, which is too noisy on shared CI hardware to give a stable
//! throughput number. Instead we model the reader-loop hot section
//! directly — same shape, same channel, same `Bytes::copy_from_slice` —
//! and measure heap-allocation count via a global counting allocator.
//! Allocation-count is the proxy metric flagged in the F-570 PR body:
//! the issue's bottleneck *is* per-chunk allocation pressure, and a
//! drop in alloc count translates 1:1 to less allocator-lock contention
//! and lower CPU on the reader task.
//!
//! Run with: `cargo bench -p forge-term --bench throughput`.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use bytes::Bytes;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use tokio::sync::mpsc;

/// Tracks every successful `alloc` call. Reset between bench iterations.
struct CountingAllocator(System);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = self.0.alloc(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.0.dealloc(ptr, layout)
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = self.0.alloc_zeroed(layout);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = self.0.realloc(ptr, layout, new_size);
        if !p.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator(System);

/// Synthetic build-output payload size (matches the F-570 DoD scenario).
const TOTAL_BYTES: usize = 64 * 1024 * 1024;

/// Pre-fix shape: per-chunk `Vec::to_vec` + `Vec::clone` for the tee.
/// Mirrors the legacy hot-path exactly so the comparison is faithful.
fn legacy_iter(chunk_size: usize, total: usize) -> usize {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async move {
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(128);
        let (tee_tx, tee_rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(256);

        // Drain both channels so backpressure shape matches reality.
        let drain = tokio::task::spawn(async move {
            let mut sunk = 0usize;
            while let Some(b) = rx.recv().await {
                sunk += b.len();
                black_box(&b);
            }
            sunk
        });
        let tee_drain = std::thread::spawn(move || {
            let mut sunk = 0usize;
            while let Ok(b) = tee_rx.recv() {
                sunk += b.len();
                black_box(&b);
            }
            sunk
        });

        let buf = vec![0xAAu8; chunk_size];
        let mut sent = 0usize;
        while sent < total {
            let n = chunk_size.min(total - sent);
            let chunk = buf[..n].to_vec();
            let _ = tee_tx.send(chunk.clone());
            tx.send(chunk).await.unwrap();
            sent += n;
        }
        drop(tx);
        drop(tee_tx);
        let _ = tee_drain.join();
        drain.await.unwrap()
    })
}

/// F-570 shape: one `Bytes::copy_from_slice`, refcount bump for the tee.
fn bytes_iter(chunk_size: usize, total: usize) -> usize {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    rt.block_on(async move {
        let (tx, mut rx) = mpsc::channel::<Bytes>(128);
        let (tee_tx, tee_rx) = std::sync::mpsc::sync_channel::<Bytes>(256);

        let drain = tokio::task::spawn(async move {
            let mut sunk = 0usize;
            while let Some(b) = rx.recv().await {
                sunk += b.len();
                black_box(&b);
            }
            sunk
        });
        let tee_drain = std::thread::spawn(move || {
            let mut sunk = 0usize;
            while let Ok(b) = tee_rx.recv() {
                sunk += b.len();
                black_box(&b);
            }
            sunk
        });

        let buf = vec![0xAAu8; chunk_size];
        let mut sent = 0usize;
        while sent < total {
            let n = chunk_size.min(total - sent);
            let chunk = Bytes::copy_from_slice(&buf[..n]);
            let _ = tee_tx.send(chunk.clone());
            tx.send(chunk).await.unwrap();
            sent += n;
        }
        drop(tx);
        drop(tee_tx);
        let _ = tee_drain.join();
        drain.await.unwrap()
    })
}

fn bench_throughput(c: &mut Criterion) {
    let mut group = c.benchmark_group("pty_reader_hot_path");
    group.throughput(Throughput::Bytes(TOTAL_BYTES as u64));
    group.sample_size(10);

    // 4 KiB chunks: legacy buffer size. Worst case for the legacy path —
    // 16k allocations per MiB of throughput.
    group.bench_with_input(
        BenchmarkId::new("legacy_vec_clone", "4KiB"),
        &(4 * 1024, TOTAL_BYTES),
        |b, &(chunk, total)| b.iter(|| legacy_iter(black_box(chunk), black_box(total))),
    );
    group.bench_with_input(
        BenchmarkId::new("bytes_refcount", "4KiB"),
        &(4 * 1024, TOTAL_BYTES),
        |b, &(chunk, total)| b.iter(|| bytes_iter(black_box(chunk), black_box(total))),
    );

    // 16 KiB chunks: the F-570 buffer size. Both paths drop their
    // syscall-equivalent count by 4×; the `Bytes` path additionally
    // halves the per-chunk alloc count vs. legacy.
    group.bench_with_input(
        BenchmarkId::new("legacy_vec_clone", "16KiB"),
        &(16 * 1024, TOTAL_BYTES),
        |b, &(chunk, total)| b.iter(|| legacy_iter(black_box(chunk), black_box(total))),
    );
    group.bench_with_input(
        BenchmarkId::new("bytes_refcount", "16KiB"),
        &(16 * 1024, TOTAL_BYTES),
        |b, &(chunk, total)| b.iter(|| bytes_iter(black_box(chunk), black_box(total))),
    );

    group.finish();
}

/// Allocation-count comparison: the headline F-570 metric. Run as a
/// dedicated bench so criterion records it separately from wall-time.
fn bench_alloc_count(c: &mut Criterion) {
    // Single fixed input; we only care about the alloc-count delta, not
    // the timing distribution. `sample_size(10)` keeps wall-clock low.
    let mut group = c.benchmark_group("pty_reader_alloc_count");
    group.sample_size(10);
    let chunk_size = 16 * 1024;
    let total = 4 * 1024 * 1024; // 4 MiB keeps the bench under a second.

    group.bench_function("legacy_4MiB_16KiB", |b| {
        b.iter_custom(|iters| {
            let mut total_allocs = 0u64;
            let start = std::time::Instant::now();
            for _ in 0..iters {
                ALLOCS.store(0, Ordering::Relaxed);
                let _ = legacy_iter(chunk_size, total);
                total_allocs += ALLOCS.load(Ordering::Relaxed) as u64;
            }
            let elapsed = start.elapsed();
            // Stash the alloc-count via stderr so it shows up alongside
            // the criterion report. Criterion still measures wall-time;
            // the eprintln is the F-570 evidence.
            eprintln!(
                "[F-570] legacy_4MiB_16KiB: total_allocs={} avg_per_iter={}",
                total_allocs,
                total_allocs / iters,
            );
            elapsed
        });
    });

    group.bench_function("bytes_4MiB_16KiB", |b| {
        b.iter_custom(|iters| {
            let mut total_allocs = 0u64;
            let start = std::time::Instant::now();
            for _ in 0..iters {
                ALLOCS.store(0, Ordering::Relaxed);
                let _ = bytes_iter(chunk_size, total);
                total_allocs += ALLOCS.load(Ordering::Relaxed) as u64;
            }
            let elapsed = start.elapsed();
            eprintln!(
                "[F-570] bytes_4MiB_16KiB:  total_allocs={} avg_per_iter={}",
                total_allocs,
                total_allocs / iters,
            );
            elapsed
        });
    });

    group.finish();
}

criterion_group!(benches, bench_throughput, bench_alloc_count);
criterion_main!(benches);
