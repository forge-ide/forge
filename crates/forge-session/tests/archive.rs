use chrono::Utc;
use forge_core::meta::{read_meta, write_meta, SessionMeta};
use forge_core::{SessionId, SessionPersistence, SessionState, WorkspaceId};
use forge_session::archive::archive_or_purge;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn make_meta(socket: PathBuf) -> SessionMeta {
    SessionMeta {
        id: SessionId::new(),
        workspace_id: WorkspaceId::new(),
        name: "test-session".to_string(),
        agent: None,
        provider_id: None,
        model: None,
        state: SessionState::Active,
        persistence: SessionPersistence::Persist,
        started_at: Utc::now(),
        ended_at: None,
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0.0,
        pid: 1234,
        socket_path: socket,
    }
}

async fn seed_persist_session(session_dir: &Path, socket_path: &Path) {
    std::fs::create_dir_all(session_dir).unwrap();
    std::fs::write(session_dir.join("events.jsonl"), "{}\n").unwrap();
    let meta = make_meta(socket_path.to_path_buf());
    write_meta(&session_dir.join("meta.toml"), &meta)
        .await
        .unwrap();
}

#[tokio::test]
async fn ephemeral_removes_session_dir_and_socket() {
    let tmp = TempDir::new().unwrap();
    let session_dir = tmp.path().join("sessions").join("abc123");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("events.jsonl"), "{}\n").unwrap();

    let sock_dir = tmp.path().join("run");
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket_path = sock_dir.join("abc123.sock");
    std::fs::write(&socket_path, "").unwrap();

    archive_or_purge(&session_dir, SessionPersistence::Ephemeral, &socket_path)
        .await
        .unwrap();

    assert!(!session_dir.exists(), "session dir must be removed");
    assert!(!socket_path.exists(), "socket must be removed");
}

#[tokio::test]
async fn persist_moves_to_archived_and_updates_meta() {
    let tmp = TempDir::new().unwrap();
    let sessions_root = tmp.path().join("sessions");
    let session_id = "persist123";
    let session_dir = sessions_root.join(session_id);

    let sock_dir = tmp.path().join("run");
    std::fs::create_dir_all(&sock_dir).unwrap();
    let socket_path = sock_dir.join(format!("{session_id}.sock"));
    std::fs::write(&socket_path, "").unwrap();

    seed_persist_session(&session_dir, &socket_path).await;

    archive_or_purge(&session_dir, SessionPersistence::Persist, &socket_path)
        .await
        .unwrap();

    let archived_dir = sessions_root.join("archived").join(session_id);
    assert!(!session_dir.exists(), "original session dir must be gone");
    assert!(archived_dir.exists(), "archived dir must exist");
    assert!(
        archived_dir.join("events.jsonl").exists(),
        "event log must be moved"
    );
    assert!(!socket_path.exists(), "socket must be removed");

    let meta = read_meta(&archived_dir.join("meta.toml")).await.unwrap();
    assert_eq!(meta.state, SessionState::Archived);
    assert!(meta.ended_at.is_some(), "ended_at must be set");
}

#[tokio::test]
async fn missing_socket_is_tolerated_on_ephemeral() {
    let tmp = TempDir::new().unwrap();
    let session_dir = tmp.path().join("sessions").join("abc");
    std::fs::create_dir_all(&session_dir).unwrap();

    let socket_path = tmp.path().join("run").join("abc.sock");
    assert!(!socket_path.exists());

    archive_or_purge(&session_dir, SessionPersistence::Ephemeral, &socket_path)
        .await
        .expect("must tolerate missing socket");
    assert!(!session_dir.exists());
}

#[tokio::test]
async fn missing_socket_is_tolerated_on_persist() {
    let tmp = TempDir::new().unwrap();
    let sessions_root = tmp.path().join("sessions");
    let session_id = "no-sock";
    let session_dir = sessions_root.join(session_id);

    let socket_path = tmp.path().join("run").join(format!("{session_id}.sock"));
    seed_persist_session(&session_dir, &socket_path).await;
    assert!(!socket_path.exists());

    archive_or_purge(&session_dir, SessionPersistence::Persist, &socket_path)
        .await
        .expect("must tolerate missing socket");

    assert!(sessions_root.join("archived").join(session_id).exists());
}

#[tokio::test]
async fn copy_dir_all_fallback_preserves_tree() {
    use forge_session::archive::copy_dir_all_for_test;

    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(src.join("nested")).unwrap();
    std::fs::write(src.join("top.txt"), "one").unwrap();
    std::fs::write(src.join("nested").join("inner.txt"), "two").unwrap();

    copy_dir_all_for_test(&src, &dst).await.unwrap();

    assert_eq!(std::fs::read_to_string(dst.join("top.txt")).unwrap(), "one");
    assert_eq!(
        std::fs::read_to_string(dst.join("nested").join("inner.txt")).unwrap(),
        "two"
    );
}

#[tokio::test]
async fn move_dir_falls_back_when_rename_crosses_devices() {
    use forge_session::archive::move_dir_with_rename_for_test;

    let tmp = TempDir::new().unwrap();
    let src = tmp.path().join("src");
    let dst = tmp.path().join("dst");
    std::fs::create_dir_all(src.join("nested")).unwrap();
    std::fs::write(src.join("a.txt"), "a").unwrap();
    std::fs::write(src.join("nested").join("b.txt"), "b").unwrap();

    // Force rename to fail with EXDEV (simulated cross-device).
    let exdev = std::io::Error::from_raw_os_error(libc::EXDEV);
    move_dir_with_rename_for_test(&src, &dst, move |_, _| Err(exdev))
        .await
        .unwrap();

    assert!(!src.exists(), "src must be removed after fallback");
    assert_eq!(std::fs::read_to_string(dst.join("a.txt")).unwrap(), "a");
    assert_eq!(
        std::fs::read_to_string(dst.join("nested").join("b.txt")).unwrap(),
        "b"
    );
}

#[tokio::test]
async fn persist_tolerates_missing_meta_toml() {
    let tmp = TempDir::new().unwrap();
    let sessions_root = tmp.path().join("sessions");
    let session_id = "no-meta";
    let session_dir = sessions_root.join(session_id);
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::write(session_dir.join("events.jsonl"), "{}\n").unwrap();
    // deliberately no meta.toml

    let socket_path = tmp.path().join("run").join(format!("{session_id}.sock"));
    std::fs::create_dir_all(socket_path.parent().unwrap()).unwrap();
    std::fs::write(&socket_path, "").unwrap();

    archive_or_purge(&session_dir, SessionPersistence::Persist, &socket_path)
        .await
        .expect("must tolerate missing meta.toml");

    let archived_dir = sessions_root.join("archived").join(session_id);
    assert!(archived_dir.exists());
    assert!(!archived_dir.join("meta.toml").exists());
    assert!(!socket_path.exists());
}
