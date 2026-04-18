//! Integration test: orchestrator shutdown wires `archive_or_purge`.
//!
//! Spawns `forged --ephemeral` with an explicit workspace, drives a single
//! turn, waits for `SessionEnded`, and asserts the session directory and
//! socket are removed.

use forge_core::Event;
use forge_ipc::{
    ClientInfo, Hello, IpcEvent, IpcMessage, SendUserMessage, Subscribe, PROTO_VERSION,
};
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::net::UnixStream;

const FORGED: &str = env!("CARGO_BIN_EXE_forged");

async fn connect_with_retry(path: &std::path::Path) -> UnixStream {
    for _ in 0..50 {
        if let Ok(s) = UnixStream::connect(path).await {
            return s;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    UnixStream::connect(path)
        .await
        .expect("forged did not create socket in time")
}

fn extract_event(msg: &IpcMessage) -> Option<Event> {
    if let IpcMessage::Event(IpcEvent { event, .. }) = msg {
        Some(serde_json::from_value::<Event>(event.clone()).unwrap())
    } else {
        None
    }
}

#[tokio::test]
async fn ephemeral_shutdown_removes_session_dir_and_socket() {
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();

    let script = format!(
        "{}\n{}",
        serde_json::json!({"delta": "Hello."}),
        serde_json::json!({"done": "end_turn"}),
    );
    let mock_path = dir.path().join("mock.json");
    std::fs::write(&mock_path, serde_json::to_string(&vec![script]).unwrap()).unwrap();

    let session_id = "archive-shutdown-test";
    let sock_path = dir.path().join("session.sock");
    let session_dir = workspace.join(".forge").join("sessions").join(session_id);

    let mut child = tokio::process::Command::new(FORGED)
        .arg("--ephemeral")
        .env("FORGE_SESSION_ID", session_id)
        .env("FORGE_SOCKET_PATH", &sock_path)
        .env("FORGE_WORKSPACE", &workspace)
        .env("FORGE_MOCK_SEQUENCE_FILE", &mock_path)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn forged");

    let mut stream = connect_with_retry(&sock_path).await;

    forge_ipc::write_frame(
        &mut stream,
        &IpcMessage::Hello(Hello {
            proto: PROTO_VERSION,
            client: ClientInfo {
                kind: "test".into(),
                pid: std::process::id(),
                user: "tester".into(),
            },
        }),
    )
    .await
    .unwrap();

    let _ack = forge_ipc::read_frame(&mut stream).await.unwrap();

    forge_ipc::write_frame(&mut stream, &IpcMessage::Subscribe(Subscribe { since: 0 }))
        .await
        .unwrap();

    forge_ipc::write_frame(
        &mut stream,
        &IpcMessage::SendUserMessage(SendUserMessage {
            text: "hi".to_string(),
        }),
    )
    .await
    .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let frame = tokio::time::timeout_at(deadline, forge_ipc::read_frame(&mut stream))
            .await
            .expect("timed out waiting for SessionEnded");
        match frame {
            Ok(msg) => {
                if let Some(event) = extract_event(&msg) {
                    if matches!(event, Event::SessionEnded { .. }) {
                        break;
                    }
                }
            }
            Err(_) => break,
        }
    }

    let status = tokio::time::timeout(Duration::from_secs(5), child.wait())
        .await
        .expect("forged did not exit after SessionEnded")
        .expect("child.wait() failed");
    assert!(
        status.success(),
        "forged exited non-zero: {:?}",
        status.code()
    );

    assert!(
        !session_dir.exists(),
        "ephemeral session dir must be purged on shutdown: {}",
        session_dir.display()
    );
    assert!(
        !sock_path.exists(),
        "socket must be removed on shutdown: {}",
        sock_path.display()
    );
}
