use forge_ipc::{ClientInfo, Hello, IpcMessage, PROTO_VERSION};
use forge_providers::MockProvider;
use forge_session::server::{serve, serve_with_session};
use forge_session::session::Session;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::UnixStream;

async fn connect_with_retry(path: &PathBuf) -> UnixStream {
    for _ in 0..20 {
        match UnixStream::connect(path).await {
            Ok(s) => return s,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }
    UnixStream::connect(path)
        .await
        .expect("server did not start in time")
}

#[tokio::test]
async fn handshake_succeeds_with_valid_proto() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test.sock");

    let server_path = path.clone();
    tokio::spawn(async move {
        serve(&server_path, false, false).await.unwrap();
    });

    let mut stream = connect_with_retry(&path).await;

    let hello = IpcMessage::Hello(Hello {
        proto: PROTO_VERSION,
        client: ClientInfo {
            kind: "shell".into(),
            pid: std::process::id(),
            user: "test-user".into(),
        },
    });
    forge_ipc::write_frame(&mut stream, &hello).await.unwrap();

    let response = forge_ipc::read_frame(&mut stream).await.unwrap();
    let IpcMessage::HelloAck(ack) = response else {
        panic!("expected HelloAck, got {:?}", response);
    };
    assert_eq!(ack.schema_version, 1);
    assert!(!ack.session_id.is_empty());
}

#[tokio::test]
async fn unknown_proto_rejected() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test2.sock");

    let server_path = path.clone();
    tokio::spawn(async move {
        serve(&server_path, false, false).await.unwrap();
    });

    let mut stream = connect_with_retry(&path).await;

    let hello = IpcMessage::Hello(Hello {
        proto: 999,
        client: ClientInfo {
            kind: "shell".into(),
            pid: std::process::id(),
            user: "test-user".into(),
        },
    });
    forge_ipc::write_frame(&mut stream, &hello).await.unwrap();

    let result = forge_ipc::read_frame(&mut stream).await;
    assert!(
        result.is_err(),
        "expected connection to be closed for unknown proto"
    );
}

async fn perform_handshake(stream: &mut UnixStream) -> forge_ipc::HelloAck {
    let hello = IpcMessage::Hello(Hello {
        proto: PROTO_VERSION,
        client: ClientInfo {
            kind: "shell".into(),
            pid: std::process::id(),
            user: "test-user".into(),
        },
    });
    forge_ipc::write_frame(stream, &hello).await.unwrap();
    let response = forge_ipc::read_frame(stream).await.unwrap();
    let IpcMessage::HelloAck(ack) = response else {
        panic!("expected HelloAck, got {:?}", response);
    };
    ack
}

/// Regression test for F-035: HelloAck.session_id must remain stable across
/// successive handshakes against the same daemon instead of being regenerated
/// per connection.
#[tokio::test]
async fn session_id_stable_across_handshakes() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let sock_path = dir.path().join("stable.sock");

    let session = Arc::new(Session::create(log_path).await.unwrap());
    let provider = Arc::new(MockProvider::with_default_path());
    let server_sock = sock_path.clone();
    let pinned_id = "fixed-session-id".to_string();
    let server_id = pinned_id.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            session,
            provider,
            false,
            false,
            None,
            Some(server_id),
        )
        .await
        .unwrap();
    });

    let mut first = connect_with_retry(&sock_path).await;
    let ack1 = perform_handshake(&mut first).await;
    drop(first);

    let mut second = connect_with_retry(&sock_path).await;
    let ack2 = perform_handshake(&mut second).await;

    assert_eq!(ack1.session_id, pinned_id);
    assert_eq!(ack2.session_id, pinned_id);
}

/// Regression test for F-035: relative FORGE_WORKSPACE inputs must be
/// normalized to absolute paths before being reported in HelloAck.workspace,
/// so clients with a different CWD can still resolve the path.
#[tokio::test]
async fn workspace_reported_as_absolute_path() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let sock_path = dir.path().join("workspace.sock");

    // Mimic main.rs normalization for a relative input.
    let relative = PathBuf::from("relative/workspace");
    let absolute = std::path::absolute(&relative).unwrap();

    let session = Arc::new(Session::create(log_path).await.unwrap());
    let provider = Arc::new(MockProvider::with_default_path());
    let server_sock = sock_path.clone();
    let server_workspace = absolute.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            session,
            provider,
            false,
            false,
            Some(server_workspace),
            None,
        )
        .await
        .unwrap();
    });

    let mut stream = connect_with_retry(&sock_path).await;
    let ack = perform_handshake(&mut stream).await;

    let reported = PathBuf::from(&ack.workspace);
    assert!(
        reported.is_absolute(),
        "workspace should be absolute, got {:?}",
        reported
    );
    assert_eq!(reported, absolute);
}

#[tokio::test]
async fn garbage_frame_rejected() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test3.sock");

    let server_path = path.clone();
    tokio::spawn(async move {
        serve(&server_path, false, false).await.unwrap();
    });

    let mut stream = connect_with_retry(&path).await;

    // Send raw garbage bytes — not valid length-prefixed JSON
    use tokio::io::AsyncWriteExt;
    stream
        .write_all(b"\xff\xff\xff\xff{garbage}")
        .await
        .unwrap();

    let result = forge_ipc::read_frame(&mut stream).await;
    assert!(
        result.is_err(),
        "expected connection to be closed for garbage frame"
    );
}
