use forge_ipc::{ClientInfo, Hello, IpcMessage, PROTO_VERSION};
use forge_session::server::serve;
use std::path::PathBuf;
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
