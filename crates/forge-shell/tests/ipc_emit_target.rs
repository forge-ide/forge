//! F-062 / M10 / T5: webview isolation for `session:event`.
//!
//! `AppHandleSink::emit` must deliver `session:event` only to the owning
//! session's webview window. Under the pre-fix broadcast behavior, a
//! listener registered on Window B would observe Session A's full event
//! stream (tool-call args, assistant turns, content excerpts). These tests
//! assert the trust boundary is enforced in Rust, not in the Solid store.
//!
//! Strategy: build a `mock_builder()` app with two `WebviewWindow`s labeled
//! `session-A` and `session-B`. Construct an `AppHandleSink` bound to
//! `session_id = "A"` via the `webview-test`-gated
//! [`forge_shell::ipc::make_app_handle_sink`]. Emit one payload with
//! `session_id = "A"`. Assert:
//!   - Window A's `session:event` listener fires exactly once.
//!   - Window B's `session:event` listener does not fire.

#![cfg(feature = "webview-test")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use forge_core::{EndReason, Event};
use forge_shell::bridge::{EventSink, SessionEventPayload};
use forge_shell::ipc::make_app_handle_sink;
use tauri::test::{mock_builder, mock_context, noop_assets};
use tauri::Listener;

fn make_app() -> tauri::App<tauri::test::MockRuntime> {
    mock_builder()
        .build(mock_context(noop_assets()))
        .expect("build mock Tauri app")
}

fn build_window(
    app: &tauri::App<tauri::test::MockRuntime>,
    label: &str,
) -> tauri::WebviewWindow<tauri::test::MockRuntime> {
    tauri::WebviewWindowBuilder::new(app, label, tauri::WebviewUrl::App("index.html".into()))
        .build()
        .expect("mock window")
}

#[test]
fn session_event_reaches_only_the_owning_window() {
    let app = make_app();
    let win_a = build_window(&app, "session-A");
    let win_b = build_window(&app, "session-B");

    let hits_a = Arc::new(AtomicUsize::new(0));
    let hits_b = Arc::new(AtomicUsize::new(0));

    let hits_a_cb = Arc::clone(&hits_a);
    win_a.listen("session:event", move |_event| {
        hits_a_cb.fetch_add(1, Ordering::SeqCst);
    });
    let hits_b_cb = Arc::clone(&hits_b);
    win_b.listen("session:event", move |_event| {
        hits_b_cb.fetch_add(1, Ordering::SeqCst);
    });

    // Sink bound to session A (mirrors production `session_subscribe` for
    // Window A's authenticated session).
    let sink: Arc<dyn EventSink> = make_app_handle_sink(app.handle().clone(), "A".to_string());

    // F-112: `event` is typed `Event` — pick any real variant (the test only
    // asserts delivery target, not payload contents).
    sink.emit(SessionEventPayload {
        session_id: "A".to_string(),
        seq: 1,
        event: Event::SessionEnded {
            at: chrono::Utc::now(),
            reason: EndReason::Completed,
            archived: false,
        },
    });

    // Listener dispatch is asynchronous on MockRuntime; give the event loop
    // a moment to drain before asserting.
    std::thread::sleep(Duration::from_millis(50));

    assert_eq!(
        hits_a.load(Ordering::SeqCst),
        1,
        "Window A must receive its own session's event",
    );
    assert_eq!(
        hits_b.load(Ordering::SeqCst),
        0,
        "Window B must NOT receive Session A's events (cross-session disclosure)",
    );
}
