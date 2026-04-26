//! F-572: benches for `EventLog::append` under flusher contention and
//! `apply_superseded` over synthetic event logs.
//!
//! ## What these benches measure
//!
//! 1. `event_log/append_under_flush_contention/{post_fix,baseline}` —
//!    wall time to append `APPEND_BATCH` `AssistantDelta` events to a
//!    live writer while a background flusher ticks every 50ms. The
//!    `baseline` arm reproduces the pre-F-572 sequence (`to_string` +
//!    two `write_all` awaits under the Mutex); `post_fix` exercises
//!    `EventLog::append` (single `write_all` under the Mutex). The
//!    criterion report under
//!    `target/criterion/event_log/append_under_flush_contention/`
//!    surfaces the percentile distribution. Note: Session::emit
//!    serialises every append through its own outer Mutex and flushes
//!    immediately, so the only contender for the EventLog-internal
//!    Mutex in production is the 50ms background flusher — the per-
//!    iter wall-time delta in this bench is the contention envelope.
//!
//! 2. `apply_superseded/{0,10,100}_branch_deleted_in_10k` — wall time
//!    of `apply_superseded` over a synthetic 10k-event log with the
//!    given number of `BranchDeleted` markers. Pre-F-572 each marker
//!    triggered a full O(N) scan (so 100 markers = O(N×K) ≈ 1M
//!    iterations). Post-F-572 the BranchDeleted resolution is O(1)
//!    against a pre-built index; total work is O(N + K).
//!
//! ## Allocator instrumentation (deferred)
//!
//! PR #583 introduces a counting allocator pattern in
//! `crates/forge-ipc/benches/frame.rs` for tracking peak RSS deltas.
//! That PR has not landed on this branch; rather than fork the helper
//! into forge-core and incur the merge cost, this bench reports
//! wall-time only. Once #583 lands, extend `apply_superseded_bench`
//! to wrap the criterion runs with the counting allocator and report
//! peak bytes alongside wall time. The post-F-572 implementation
//! drops one log-sized Vec allocation (`into_iter().filter().collect()`
//! → `Vec::retain`), so the alloc-bytes win should be visible in that
//! follow-up.
//!
//! Run: `cargo bench -p forge-core --bench event_log_and_superseded`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use forge_core::{apply_superseded, Event, EventLog, MessageId, ProviderId};
use tempfile::TempDir;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;

// Realistic per-token delta size — same envelope as the F-112 IPC bench.
const REALISTIC_DELTA: &str =
    "The quick brown fox jumps over the lazy dog — streaming token payload.";

fn make_delta(id: &MessageId) -> Event {
    Event::AssistantDelta {
        id: id.clone(),
        at: Utc::now(),
        delta: Arc::from(REALISTIC_DELTA),
    }
}

fn make_assistant(id: &MessageId, parent: Option<&MessageId>, idx: u32) -> Event {
    Event::AssistantMessage {
        id: id.clone(),
        provider: ProviderId::new(),
        model: "mock".into(),
        at: Utc::now(),
        stream_finalised: true,
        text: Arc::from("synthetic"),
        branch_parent: parent.cloned(),
        branch_variant_index: idx,
    }
}

// ── Bench 1: EventLog append under flush contention ───────────────────────
//
// `post_fix` exercises the production `EventLog::append` (single
// `write_all` under the Mutex). `baseline` runs the pre-F-572 shape:
// `serde_json::to_string` then two `write_all` awaits while holding the
// Mutex. Both share the same flush-task-on-50ms-tick contention the bug
// report cites. Compare the per-iter times to see the contention cost.

// Sized so the append batch spans many flusher ticks (50ms each) under
// `current_thread`, exposing the Mutex contention the bug report cites.
// The pre-F-572 path holds the Mutex across two `write_all` awaits, so
// each flusher tick that fires during a tight append loop has to wait
// out both writes — not one.
const APPEND_BATCH: usize = 50_000;

async fn run_post_fix_append(log_path: std::path::PathBuf, events: Vec<Event>) -> Duration {
    let mut log = EventLog::create(&log_path).await.expect("create");
    let start = Instant::now();
    for ev in &events {
        log.append(ev).await.expect("append");
    }
    let elapsed = start.elapsed();
    log.close().await.expect("close");
    elapsed
}

async fn run_baseline_append(log_path: std::path::PathBuf, events: Vec<Event>) -> Duration {
    // Mirror EventLog's structure: BufWriter behind a Mutex with a 50ms
    // background flusher. The append body uses the pre-F-572 sequence:
    // `serde_json::to_string` then two `write_all` awaits under the Mutex.
    let file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&log_path)
        .await
        .expect("open");
    let writer = Arc::new(Mutex::new(BufWriter::new(file)));
    let flush_handle = {
        let writer = Arc::clone(&writer);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(50));
            loop {
                ticker.tick().await;
                let _ = writer.lock().await.flush().await;
            }
        })
    };

    let start = Instant::now();
    for ev in &events {
        let line = serde_json::to_string(ev).expect("serialize");
        let mut w = writer.lock().await;
        w.write_all(line.as_bytes()).await.expect("write body");
        w.write_all(b"\n").await.expect("write newline");
    }
    let elapsed = start.elapsed();

    flush_handle.abort();
    writer.lock().await.flush().await.expect("final flush");
    elapsed
}

fn bench_event_log_append(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");

    let mut group = c.benchmark_group("event_log/append_under_flush_contention");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(10));

    group.bench_function("post_fix", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let elapsed = rt.block_on(async {
                    let dir = TempDir::new().expect("tempdir");
                    let id = MessageId::new();
                    let events: Vec<Event> = (0..APPEND_BATCH).map(|_| make_delta(&id)).collect();
                    run_post_fix_append(dir.path().join("events.jsonl"), events).await
                });
                total += elapsed;
            }
            total
        });
    });

    group.bench_function("baseline", |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                let elapsed = rt.block_on(async {
                    let dir = TempDir::new().expect("tempdir");
                    let id = MessageId::new();
                    let events: Vec<Event> = (0..APPEND_BATCH).map(|_| make_delta(&id)).collect();
                    run_baseline_append(dir.path().join("events.jsonl"), events).await
                });
                total += elapsed;
            }
            total
        });
    });

    group.finish();
}

// ── Bench 2: apply_superseded over synthetic event log ────────────────────

/// Build a 10k-event log shaped like a realistic transcript:
///   - `n_messages` `AssistantMessage` events at the *tail* of the log
///     (so the pre-F-572 inner-loop scan walks the whole log before
///     finding its target — the worst case for the O(N×K) baseline)
///   - the rest filled with `AssistantDelta` events tied to a padding id
///
/// Then append `n_branch_deleted` `BranchDeleted` markers targeting
/// `(parent, variant_index=1)` for the first `n_branch_deleted` siblings.
///
/// The tail placement is realistic for the BranchDeleted use case: a user
/// hits Delete on a recent branch, which writes the marker after a long
/// existing transcript; the target sibling is also recent, sitting near
/// the tail. Pre-F-572 this hits the worst-case scan length.
fn build_synthetic_log(
    total_events: usize,
    n_messages: usize,
    n_branch_deleted: usize,
) -> Vec<(u64, Event)> {
    assert!(n_messages <= total_events);
    assert!(n_branch_deleted <= n_messages);

    let parents: Vec<MessageId> = (0..n_messages).map(|_| MessageId::new()).collect();
    let siblings: Vec<MessageId> = (0..n_branch_deleted).map(|_| MessageId::new()).collect();
    let pad_id = MessageId::new();

    let n_msg_events = parents.len() + siblings.len();
    let n_marker_events = n_branch_deleted;
    let n_pad = total_events
        .saturating_sub(n_msg_events)
        .saturating_sub(n_marker_events);

    let mut events: Vec<Event> = Vec::with_capacity(total_events);
    // Lead with delta padding so the AssistantMessage targets sit deep in
    // the vec — pre-F-572 inner-loop scan must walk past every padding
    // event before reaching the match.
    for _ in 0..n_pad {
        events.push(make_delta(&pad_id));
    }
    // Roots (variant 0)
    for p in &parents {
        events.push(make_assistant(p, None, 0));
    }
    // Sibling variant 1 messages for the to-be-deleted branches
    for (parent, sib) in parents.iter().zip(siblings.iter()) {
        events.push(make_assistant(sib, Some(parent), 1));
    }
    // Append BranchDeleted markers at the tail
    for parent in parents.iter().take(n_branch_deleted) {
        events.push(Event::BranchDeleted {
            parent: parent.clone(),
            variant_index: 1,
        });
    }

    events
        .into_iter()
        .enumerate()
        .map(|(i, e)| ((i + 1) as u64, e))
        .collect()
}

/// Pre-F-572 implementation, copied verbatim for back-to-back comparison.
/// Nested loop over events for each `BranchDeleted` marker; final filter
/// allocates a second log-sized Vec via `into_iter().filter().collect()`.
fn apply_superseded_baseline(events: Vec<(u64, Event)>) -> Vec<(u64, Event)> {
    let mut hidden_ids: HashSet<MessageId> = HashSet::new();
    for (_, ev) in &events {
        match ev {
            Event::MessageSuperseded { old_id, .. } => {
                hidden_ids.insert(old_id.clone());
            }
            Event::BranchDeleted {
                parent,
                variant_index,
            } => {
                for (_, cand) in &events {
                    if let Event::AssistantMessage {
                        id,
                        branch_parent,
                        branch_variant_index: idx,
                        ..
                    } = cand
                    {
                        let is_target = if *variant_index == 0 {
                            branch_parent.is_none() && id == parent
                        } else {
                            branch_parent.as_ref() == Some(parent) && idx == variant_index
                        };
                        if is_target {
                            hidden_ids.insert(id.clone());
                            break;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if hidden_ids.is_empty() {
        return events;
    }
    events
        .into_iter()
        .filter(|(_, ev)| match ev {
            Event::AssistantMessage { id, .. } | Event::AssistantDelta { id, .. } => {
                !hidden_ids.contains(id)
            }
            Event::MessageSuperseded { .. } | Event::BranchDeleted { .. } => false,
            _ => true,
        })
        .collect()
}

fn bench_apply_superseded(c: &mut Criterion) {
    let mut group = c.benchmark_group("apply_superseded");
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(8));

    for n_branch_deleted in [0usize, 10, 100] {
        let log = build_synthetic_log(10_000, 200, n_branch_deleted);
        group.bench_with_input(
            BenchmarkId::new("post_fix/branch_deleted_in_10k", n_branch_deleted),
            &log,
            |b, log| {
                b.iter_with_setup(
                    || log.clone(),
                    |events| {
                        let out = apply_superseded(black_box(events));
                        black_box(out);
                    },
                );
            },
        );
        group.bench_with_input(
            BenchmarkId::new("baseline/branch_deleted_in_10k", n_branch_deleted),
            &log,
            |b, log| {
                b.iter_with_setup(
                    || log.clone(),
                    |events| {
                        let out = apply_superseded_baseline(black_box(events));
                        black_box(out);
                    },
                );
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_event_log_append, bench_apply_superseded);
criterion_main!(benches);
