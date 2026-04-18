//! Integration tests for `forge_shell::dashboard_sessions::collect_sessions`.
//!
//! The `FakePinger` lets us exercise the active/stopped split without any
//! live UDS traffic. Real UDS liveness is the responsibility of the
//! production `UdsPinger`, which is exercised by the `cargo check` build and
//! manual smoke tests — not unit tests.

use std::path::Path;
use std::sync::Mutex;

use async_trait::async_trait;
use chrono::Utc;
use forge_core::meta::{write_meta, SessionMeta};
use forge_core::workspaces::{write_workspaces, WorkspaceEntry};
use forge_core::{SessionId, SessionPersistence, SessionState, WorkspaceId};
use forge_shell::dashboard_sessions::{collect_sessions, Pinger, SessionSummary};
use tempfile::TempDir;

/// A `Pinger` that always answers the same boolean.
struct FakePinger(bool);

#[async_trait]
impl Pinger for FakePinger {
    async fn ping(&self, _socket: &Path) -> bool {
        self.0
    }
}

/// A `Pinger` that records every socket it was asked about.
#[allow(dead_code)]
struct RecordingPinger {
    answer: bool,
    seen: Mutex<Vec<std::path::PathBuf>>,
}

#[async_trait]
impl Pinger for RecordingPinger {
    async fn ping(&self, socket: &Path) -> bool {
        self.seen.lock().unwrap().push(socket.to_path_buf());
        self.answer
    }
}

fn make_meta(id: &SessionId, workspace_id: &WorkspaceId, socket: &Path) -> SessionMeta {
    SessionMeta {
        id: id.clone(),
        workspace_id: workspace_id.clone(),
        name: format!("session-{id}"),
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
        socket_path: socket.to_path_buf(),
    }
}

async fn seed_session(session_dir: &Path, workspace_id: &WorkspaceId, socket: &Path) -> SessionId {
    std::fs::create_dir_all(session_dir).unwrap();
    std::fs::write(session_dir.join("events.jsonl"), "{}\n").unwrap();
    let id = SessionId::new();
    let meta = make_meta(&id, workspace_id, socket);
    write_meta(&session_dir.join("meta.toml"), &meta)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn missing_workspaces_toml_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");
    assert!(!workspaces_toml.exists());

    let pinger = FakePinger(true);
    let result = collect_sessions(&workspaces_toml, &pinger).await.unwrap();

    assert_eq!(result, Vec::<SessionSummary>::new());
}

/// Registers `workspace_root` in `workspaces_toml` under an auto-generated
/// `WorkspaceId`. Returns that id for session seeding.
async fn register_workspace(workspaces_toml: &Path, workspace_root: &Path) -> WorkspaceId {
    let id = WorkspaceId::new();
    let entry = WorkspaceEntry {
        id: id.clone(),
        path: workspace_root.to_path_buf(),
        name: workspace_root
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("ws")
            .to_string(),
        last_opened: Utc::now(),
        pinned: false,
    };
    write_workspaces(workspaces_toml, std::slice::from_ref(&entry))
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn empty_registry_returns_empty() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");
    write_workspaces(&workspaces_toml, &[]).await.unwrap();

    let pinger = FakePinger(true);
    let result = collect_sessions(&workspaces_toml, &pinger).await.unwrap();

    assert_eq!(result, Vec::<SessionSummary>::new());
}

#[tokio::test]
async fn active_session_with_live_socket_reports_active() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");
    let workspace_root = tmp.path().join("alpha");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace_id = register_workspace(&workspaces_toml, &workspace_root).await;

    let sessions_dir = workspace_root.join(".forge").join("sessions");
    let session_dir = sessions_dir.join("s0");
    let socket = tmp.path().join("s0.sock");
    let id = seed_session(&session_dir, &workspace_id, &socket).await;

    let pinger = FakePinger(true);
    let summaries = collect_sessions(&workspaces_toml, &pinger).await.unwrap();

    assert_eq!(summaries.len(), 1);
    let s = &summaries[0];
    assert_eq!(s.id, id.to_string());
    assert_eq!(s.subject, format!("session-{id}"));
    assert_eq!(s.state, "active");
    assert_eq!(s.persistence, "persist");
    assert!(!s.created_at.is_empty());
    assert!(!s.last_event_at.is_empty());
}

#[tokio::test]
async fn only_archived_sessions_are_reported_archived() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");
    let workspace_root = tmp.path().join("beta");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace_id = register_workspace(&workspaces_toml, &workspace_root).await;

    let archived_dir = workspace_root
        .join(".forge")
        .join("sessions")
        .join("archived")
        .join("arc0");
    let socket = tmp.path().join("arc0.sock");
    seed_session(&archived_dir, &workspace_id, &socket).await;

    let pinger = FakePinger(false);
    let summaries = collect_sessions(&workspaces_toml, &pinger).await.unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].state, "archived");
}

#[tokio::test]
async fn stale_active_socket_reports_stopped() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");
    let workspace_root = tmp.path().join("gamma");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace_id = register_workspace(&workspaces_toml, &workspace_root).await;

    let session_dir = workspace_root
        .join(".forge")
        .join("sessions")
        .join("stale0");
    let socket = tmp.path().join("stale0.sock");
    seed_session(&session_dir, &workspace_id, &socket).await;

    // Pinger returns false: socket exists on disk but server is dead.
    let pinger = FakePinger(false);
    let summaries = collect_sessions(&workspaces_toml, &pinger).await.unwrap();

    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].state, "stopped");
}

#[tokio::test]
async fn mixed_active_and_archived_across_workspaces() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");

    let ws1 = tmp.path().join("ws-one");
    let ws2 = tmp.path().join("ws-two");
    std::fs::create_dir_all(&ws1).unwrap();
    std::fs::create_dir_all(&ws2).unwrap();
    let id1 = WorkspaceId::new();
    let id2 = WorkspaceId::new();
    let entries = vec![
        WorkspaceEntry {
            id: id1.clone(),
            path: ws1.clone(),
            name: "ws-one".to_string(),
            last_opened: Utc::now(),
            pinned: false,
        },
        WorkspaceEntry {
            id: id2.clone(),
            path: ws2.clone(),
            name: "ws-two".to_string(),
            last_opened: Utc::now(),
            pinned: false,
        },
    ];
    write_workspaces(&workspaces_toml, &entries).await.unwrap();

    // ws1: one active session (alive)
    seed_session(
        &ws1.join(".forge").join("sessions").join("a1"),
        &id1,
        &tmp.path().join("a1.sock"),
    )
    .await;
    // ws2: one archived session
    seed_session(
        &ws2.join(".forge")
            .join("sessions")
            .join("archived")
            .join("ar1"),
        &id2,
        &tmp.path().join("ar1.sock"),
    )
    .await;

    let pinger = FakePinger(true);
    let summaries = collect_sessions(&workspaces_toml, &pinger).await.unwrap();

    assert_eq!(summaries.len(), 2);
    let states: Vec<_> = summaries.iter().map(|s| s.state.as_str()).collect();
    assert!(states.contains(&"active"));
    assert!(states.contains(&"archived"));
}

#[tokio::test]
async fn session_dir_without_meta_is_skipped() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");
    let workspace_root = tmp.path().join("delta");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let _ = register_workspace(&workspaces_toml, &workspace_root).await;

    // Directory exists but has no meta.toml — must be ignored, not fail.
    let orphan = workspace_root
        .join(".forge")
        .join("sessions")
        .join("orphan");
    std::fs::create_dir_all(&orphan).unwrap();

    let pinger = FakePinger(true);
    let summaries = collect_sessions(&workspaces_toml, &pinger).await.unwrap();
    assert!(summaries.is_empty());
}

#[tokio::test]
async fn pinger_receives_socket_path_from_meta() {
    let tmp = TempDir::new().unwrap();
    let workspaces_toml = tmp.path().join("workspaces.toml");
    let workspace_root = tmp.path().join("ep");
    std::fs::create_dir_all(&workspace_root).unwrap();
    let workspace_id = register_workspace(&workspaces_toml, &workspace_root).await;

    let session_dir = workspace_root
        .join(".forge")
        .join("sessions")
        .join("probe0");
    let socket = tmp.path().join("probe0.sock");
    seed_session(&session_dir, &workspace_id, &socket).await;

    let pinger = RecordingPinger {
        answer: true,
        seen: Mutex::new(Vec::new()),
    };
    let _ = collect_sessions(&workspaces_toml, &pinger).await.unwrap();
    let seen = pinger.seen.lock().unwrap().clone();
    assert_eq!(seen, vec![socket]);
}
