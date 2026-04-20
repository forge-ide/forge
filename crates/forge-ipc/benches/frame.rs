//! F-112: per-frame serialization bench.
//!
//! The DoD for F-112 requires ≥30% reduction in per-frame wall-time between
//! the pre-fix path (`Event -> serde_json::Value -> IpcEvent{event:Value} -> bytes`)
//! and the typed path (`Event -> IpcEvent{event:Event} -> bytes`). The
//! `assistant_delta_*` benches measure the single-subscriber case; the
//! `fanout_*` benches measure the two-subscriber case that surfaces the
//! `Arc<str>` cheap-clone win on top of the Value-elimination win.
//!
//! Run `cargo bench -p forge-ipc` to produce criterion reports under
//! `target/criterion/`. The first two reports (single-subscriber) are the
//! headline numbers referenced by the DoD checklist.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeZone, Utc};
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use forge_core::{Event, MessageId};
use serde::{Deserialize, Serialize};

// The pre-fix shape of `IpcEvent` — retained here so the baseline bench can
// exercise the exact path that existed before F-112. Kept as a local type so
// this bench file is self-contained: the real `IpcEvent` carries `Event`
// directly after F-112.
#[derive(Debug, Serialize, Deserialize)]
struct IpcEventValue {
    seq: u64,
    event: serde_json::Value,
}

// The post-fix shape — matches `forge_ipc::IpcEvent` exactly. Using a local
// mirror keeps the bench self-contained and lets us write both codepaths
// against identical `Event` inputs.
#[derive(Debug, Serialize, Deserialize)]
struct IpcEventTyped {
    seq: u64,
    event: Event,
}

fn fixed_time() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 4, 18, 10, 0, 0).unwrap()
}

fn message_id(s: &str) -> MessageId {
    serde_json::from_value(serde_json::Value::String(s.to_string())).unwrap()
}

// Realistic per-token delta size. LLM streaming tokens are typically
// 2-12 bytes for text, up to ~80 bytes for long punctuation/unicode runs.
// We bench the upper envelope so the measurement reflects the worst-case
// per-frame cost a user actually sees on an active stream.
const REALISTIC_DELTA: &str =
    "The quick brown fox jumps over the lazy dog — streaming token payload.";

fn make_delta(delta: Arc<str>) -> Event {
    Event::AssistantDelta {
        id: message_id("mid-bench"),
        at: fixed_time(),
        delta,
    }
}

// ── Baseline: Event -> Value -> IpcEvent -> bytes ─────────────────────────

fn baseline_event_to_bytes_via_value(event: &Event) -> Vec<u8> {
    let value = serde_json::to_value(event).expect("to_value");
    let frame = IpcEventValue {
        seq: 1,
        event: value,
    };
    serde_json::to_vec(&frame).expect("to_vec")
}

// ── Proposed: Event -> IpcEvent -> bytes (single traversal) ──────────────

fn typed_event_to_bytes(event: &Event) -> Vec<u8> {
    // Clone is a single `Arc::clone` for the `delta` field (no heap copy);
    // the struct clone itself is a small memcpy of the stack fields.
    let frame = IpcEventTyped {
        seq: 1,
        event: event.clone(),
    };
    serde_json::to_vec(&frame).expect("to_vec")
}

// ── Fanout variants: same delta, two subscribers ─────────────────────────
//
// The realistic production hot path fans a single delta out to N connected
// webview clients (dashboard + active session window at minimum; add more
// when additional panes subscribe). The `Arc<str>` cheap-clone shows its
// dominant win here — each fanout is a ref-count bump rather than a fresh
// `String::clone` heap allocation.

fn baseline_fanout_two(event: &Event) -> (Vec<u8>, Vec<u8>) {
    // Simulate "two subscribers each get their own frame" the way
    // `server.rs` pre-F-112 did: two fresh `to_value` walks, two fresh
    // `to_vec` walks.
    let v1 = baseline_event_to_bytes_via_value(event);
    let v2 = baseline_event_to_bytes_via_value(event);
    (v1, v2)
}

fn typed_fanout_two(event: &Event) -> (Vec<u8>, Vec<u8>) {
    let v1 = typed_event_to_bytes(event);
    let v2 = typed_event_to_bytes(event);
    (v1, v2)
}

fn bench_frame(c: &mut Criterion) {
    let delta: Arc<str> = Arc::from(REALISTIC_DELTA);
    let event = make_delta(Arc::clone(&delta));

    let mut group = c.benchmark_group("assistant_delta_frame");
    // Sample enough to make the 30% gap statistically detectable on noisy
    // developer laptops; criterion's default 100 samples is fine but the
    // measurement window at ~1µs/frame is tight, so use a longer window.
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("baseline_event_to_value_to_bytes", |b| {
        b.iter(|| {
            let bytes = baseline_event_to_bytes_via_value(black_box(&event));
            black_box(bytes);
        })
    });

    group.bench_function("typed_event_to_bytes", |b| {
        b.iter(|| {
            let bytes = typed_event_to_bytes(black_box(&event));
            black_box(bytes);
        })
    });

    group.finish();

    let mut group = c.benchmark_group("assistant_delta_fanout_two");
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("baseline_fanout_two", |b| {
        b.iter(|| {
            let out = baseline_fanout_two(black_box(&event));
            black_box(out);
        })
    });

    group.bench_function("typed_fanout_two", |b| {
        b.iter(|| {
            let out = typed_fanout_two(black_box(&event));
            black_box(out);
        })
    });

    group.finish();
}

criterion_group!(benches, bench_frame);
criterion_main!(benches);
