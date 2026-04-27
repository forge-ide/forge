//! F-586: orchestrator subscription to mid-session provider swap.
//!
//! Demonstrates that an `Arc<SwappableProvider>` plumbed through
//! `serve_with_session` honors a swap between turns: the next
//! `SendUserMessage` arrives at the new inner without restarting the
//! session.
//!
//! The current turn's stream is captured against the old inner before the
//! swap (per F-586 spec: "the current turn finishes on the old provider").
//! A second `SendUserMessage` after the swap exercises the new inner.
//!
//! This is the in-process counterpart to the dashboard wiring: in
//! production the shell's `set_active_provider` IPC command emits a
//! `provider:changed` Tauri event; a session-window listener forwards it
//! over the per-session UDS, the connection loop picks it up and calls
//! `SwappableProvider::swap`. This test exercises the same primitive
//! (`SwappableProvider::swap`) directly so the orchestrator-side contract
//! is pinned without a Tauri runtime.

use forge_core::Event;
use forge_ipc::{
    ClientInfo, Hello, IpcEvent, IpcMessage, SendUserMessage, Subscribe, PROTO_VERSION,
};
use forge_providers::{MockProvider, RuntimeProvider, SwappableProvider};
use forge_session::{server::serve_with_session, session::Session};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::UnixStream;

async fn connect_with_retry(path: &std::path::PathBuf) -> UnixStream {
    for _ in 0..50 {
        match UnixStream::connect(path).await {
            Ok(s) => return s,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }
    UnixStream::connect(path)
        .await
        .expect("server did not start in time")
}

async fn do_handshake(stream: &mut UnixStream) {
    let hello = IpcMessage::Hello(Hello {
        proto: PROTO_VERSION,
        client: ClientInfo {
            kind: "test".into(),
            pid: std::process::id(),
            user: "tester".into(),
        },
    });
    forge_ipc::write_frame(stream, &hello).await.unwrap();
    let response = forge_ipc::read_frame(stream).await.unwrap();
    assert!(
        matches!(response, IpcMessage::HelloAck(_)),
        "expected HelloAck"
    );
}

fn extract_event(msg: &IpcMessage) -> Option<Event> {
    if let IpcMessage::Event(IpcEvent { event, .. }) = msg {
        Some(event.clone())
    } else {
        None
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn next_turn_uses_new_provider_after_swap() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let sock_path = dir.path().join("test.sock");

    // Build two distinct mocks. Each emits a single token then `done` so
    // the assistant reply text equals exactly the labeled token.
    let mock_before = Arc::new(
        MockProvider::from_responses(vec![
            "{\"delta\":\"BEFORE\"}\n{\"done\":\"end_turn\"}\n".into()
        ])
        .unwrap(),
    );
    let mock_after = Arc::new(
        MockProvider::from_responses(vec![
            "{\"delta\":\"AFTER\"}\n{\"done\":\"end_turn\"}\n".into()
        ])
        .unwrap(),
    );

    // Wire the SwappableProvider through `serve_with_session`. The daemon
    // calls `chat()` on it per turn — the snapshot is taken at chat-call
    // time, so a swap between turns takes effect on the next call.
    let swap = Arc::new(SwappableProvider::new(RuntimeProvider::Mock(Arc::clone(
        &mock_before,
    ))));

    let session = Arc::new(Session::create(log_path).await.unwrap());

    let server_session = Arc::clone(&session);
    let server_provider = Arc::clone(&swap);
    let server_sock = sock_path.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            server_session,
            server_provider,
            false, // auto_approve
            false, // ephemeral
            None,
            None,
            None, // F-587 keyless
            None, // F-601 active_agent: memory off
        )
        .await
        .ok();
    });

    let mut stream = connect_with_retry(&sock_path).await;
    do_handshake(&mut stream).await;

    forge_ipc::write_frame(&mut stream, &IpcMessage::Subscribe(Subscribe { since: 0 }))
        .await
        .unwrap();

    // ── Turn 1 — should hit `mock_before` ────────────────────────────────
    forge_ipc::write_frame(
        &mut stream,
        &IpcMessage::SendUserMessage(SendUserMessage {
            text: "first turn".into(),
        }),
    )
    .await
    .unwrap();

    loop {
        let frame = forge_ipc::read_frame(&mut stream).await.unwrap();
        let Some(ev) = extract_event(&frame) else {
            continue;
        };
        if let Event::AssistantMessage {
            text,
            stream_finalised: true,
            ..
        } = ev
        {
            assert_eq!(&*text, "BEFORE");
            break;
        }
    }

    // ── Swap mid-session ────────────────────────────────────────────────
    swap.swap(RuntimeProvider::Mock(Arc::clone(&mock_after)));
    assert_eq!(swap.active_id(), "mock");

    // ── Turn 2 — must hit `mock_after` ──────────────────────────────────
    forge_ipc::write_frame(
        &mut stream,
        &IpcMessage::SendUserMessage(SendUserMessage {
            text: "second turn".into(),
        }),
    )
    .await
    .unwrap();

    loop {
        let frame = forge_ipc::read_frame(&mut stream).await.unwrap();
        let Some(ev) = extract_event(&frame) else {
            continue;
        };
        if let Event::AssistantMessage {
            text,
            stream_finalised: true,
            ..
        } = ev
        {
            assert_eq!(
                &*text, "AFTER",
                "post-swap turn must reach the new provider"
            );
            break;
        }
    }

    // Sanity: each mock saw exactly one chat() call. The "before" mock
    // received the first turn's request; the "after" mock received the
    // second's. If swap had been ignored, mock_before would record both
    // turns and mock_after would record none.
    assert_eq!(
        mock_before.recorded_requests().len(),
        1,
        "first turn went to the original provider"
    );
    assert_eq!(
        mock_after.recorded_requests().len(),
        1,
        "second turn went to the new provider after swap"
    );
}
