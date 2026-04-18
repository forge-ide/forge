//! End-to-end bridge tests: spawn a real `forge-session` daemon, drive it
//! through [`SessionBridge`], and assert the full command + event flow.
//!
//! These tests satisfy the integration requirement in F-020's DoD: the
//! webview → Tauri command → UDS → daemon path is exercised with the real
//! session server. A capturing [`EventSink`] stands in for the Tauri
//! `AppHandle::emit` call; the Tauri command wrappers in [`forge_shell::ipc`]
//! delegate to the same bridge code tested here.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use forge_core::Event;
use forge_providers::MockProvider;
use forge_session::server::serve_with_session;
use forge_session::session::Session;
use forge_shell::bridge::{EventSink, SessionBridge, SessionConnections, SessionEventPayload};
use tempfile::TempDir;
use tokio::sync::mpsc;

/// Test sink that forwards every emitted payload through an mpsc channel so
/// the test can await delivery with a timeout.
struct ChannelSink {
    tx: mpsc::UnboundedSender<SessionEventPayload>,
}

impl EventSink for ChannelSink {
    fn emit(&self, payload: SessionEventPayload) {
        let _ = self.tx.send(payload);
    }
}

async fn spawn_daemon(path: &Path, session_id: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let session = Arc::new(Session::create(log_path).await.unwrap());
    let provider = Arc::new(MockProvider::with_default_path());
    let sock = path.to_path_buf();
    let sid = session_id.to_string();
    tokio::spawn(async move {
        serve_with_session(&sock, session, provider, true, false, None, Some(sid))
            .await
            .unwrap();
    });
    // Wait for the server to bind its socket.
    for _ in 0..50 {
        if path.exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    dir
}

#[tokio::test]
async fn hello_handshake_against_real_daemon() {
    let sock_dir = TempDir::new().unwrap();
    let sock = sock_dir.path().join("hello.sock");
    let _daemon = spawn_daemon(&sock, "fixed-id").await;

    let bridge = SessionBridge::new(SessionConnections::new());
    let ack = bridge
        .hello("fixed-id", Some(&sock))
        .await
        .expect("hello succeeds");

    assert_eq!(ack.session_id, "fixed-id");
    assert_eq!(ack.schema_version, 1);
    assert_eq!(bridge.connections().len().await, 1);
    assert!(bridge.connections().contains("fixed-id").await);
}

#[tokio::test]
async fn send_message_forwards_events_end_to_end() {
    let sock_dir = TempDir::new().unwrap();
    let sock = sock_dir.path().join("send.sock");
    let _daemon = spawn_daemon(&sock, "send-session").await;

    let bridge = SessionBridge::new(SessionConnections::new());
    bridge
        .hello("send-session", Some(&sock))
        .await
        .expect("hello");

    let (tx, mut rx) = mpsc::unbounded_channel();
    let sink = Arc::new(ChannelSink { tx });
    bridge
        .subscribe("send-session", 0, sink)
        .await
        .expect("subscribe");

    bridge
        .send_message("send-session", "hello world".to_string())
        .await
        .expect("send_message");

    // Drain events until we see a UserMessageAdded whose text matches.
    let mut saw_user_message = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match tokio::time::timeout(remaining, rx.recv()).await {
            Ok(Some(payload)) => {
                assert_eq!(payload.session_id, "send-session");
                let event: Event = serde_json::from_value(payload.event).unwrap();
                if let Event::UserMessage { text, .. } = event {
                    assert_eq!(text, "hello world");
                    saw_user_message = true;
                    break;
                }
            }
            _ => break,
        }
    }
    assert!(saw_user_message, "expected UserMessageAdded event");
}

#[tokio::test]
async fn approve_tool_proxies_to_daemon() {
    // Without a tool call in flight, approve is a no-op on the daemon side
    // (the pending-approvals map has no entry). We assert the frame is
    // written without error — i.e. the bridge routes the command through
    // the same writer it uses for send_message.
    let sock_dir = TempDir::new().unwrap();
    let sock = sock_dir.path().join("approve.sock");
    let _daemon = spawn_daemon(&sock, "approve-session").await;

    let bridge = SessionBridge::new(SessionConnections::new());
    bridge
        .hello("approve-session", Some(&sock))
        .await
        .expect("hello");

    let (tx, _rx) = mpsc::unbounded_channel();
    bridge
        .subscribe("approve-session", 0, Arc::new(ChannelSink { tx }))
        .await
        .expect("subscribe");

    bridge
        .approve_tool("approve-session", "call-123".into(), "Once".into())
        .await
        .expect("approve_tool writes frame");

    bridge
        .reject_tool("approve-session", "call-456".into(), Some("nope".into()))
        .await
        .expect("reject_tool writes frame");
}

#[tokio::test]
async fn hello_twice_for_same_session_is_rejected() {
    let sock_dir = TempDir::new().unwrap();
    let sock = sock_dir.path().join("dup.sock");
    let _daemon = spawn_daemon(&sock, "dup-session").await;

    let bridge = SessionBridge::new(SessionConnections::new());
    bridge
        .hello("dup-session", Some(&sock))
        .await
        .expect("first hello");
    let err = bridge
        .hello("dup-session", Some(&sock))
        .await
        .expect_err("second hello must be rejected");
    assert!(err.to_string().contains("already connected"));
}
