use chrono::Utc;
use forge_core::{
    meta::{read_meta, write_meta, SessionMeta},
    AgentId, ProviderId, SessionId, SessionPersistence, SessionState, WorkspaceId,
};
use std::path::PathBuf;
use tempfile::TempDir;

#[tokio::test]
async fn meta_toml_round_trip_full() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("meta.toml");

    let started = Utc::now().with_nanosecond(0).unwrap();
    let ended = Utc::now().with_nanosecond(0).unwrap();

    let meta = SessionMeta {
        id: SessionId::new(),
        workspace_id: WorkspaceId::new(),
        name: "refactor-payment-service".to_string(),
        agent: Some(AgentId::new()),
        provider_id: None,
        model: None,
        state: SessionState::Active,
        persistence: SessionPersistence::Persist,
        started_at: started,
        ended_at: Some(ended),
        tokens_in: 48200,
        tokens_out: 12100,
        cost_usd: 0.23,
        pid: 48211,
        socket_path: PathBuf::from("/tmp/forge-1000/sessions/a3f1b2c4.sock"),
    };

    write_meta(&path, &meta).await.unwrap();
    let loaded = read_meta(&path).await.unwrap();

    assert_eq!(meta.id, loaded.id);
    assert_eq!(meta.workspace_id, loaded.workspace_id);
    assert_eq!(meta.name, loaded.name);
    assert_eq!(meta.agent, loaded.agent);
    assert_eq!(meta.provider_id, loaded.provider_id);
    assert_eq!(meta.model, loaded.model);
    assert_eq!(meta.state, loaded.state);
    assert_eq!(meta.persistence, loaded.persistence);
    assert_eq!(meta.started_at, loaded.started_at);
    assert_eq!(meta.ended_at, loaded.ended_at);
    assert_eq!(meta.tokens_in, loaded.tokens_in);
    assert_eq!(meta.tokens_out, loaded.tokens_out);
    assert_eq!(meta.cost_usd, loaded.cost_usd);
    assert_eq!(meta.pid, loaded.pid);
    assert_eq!(meta.socket_path, loaded.socket_path);
}

#[tokio::test]
async fn meta_toml_round_trip_bare_provider() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("meta.toml");

    let meta = SessionMeta {
        id: SessionId::new(),
        workspace_id: WorkspaceId::new(),
        name: "bare-session".to_string(),
        agent: None,
        provider_id: Some(ProviderId::new()),
        model: Some("sonnet-4.5".to_string()),
        state: SessionState::Ended,
        persistence: SessionPersistence::Ephemeral,
        started_at: Utc::now().with_nanosecond(0).unwrap(),
        ended_at: None,
        tokens_in: 100,
        tokens_out: 50,
        cost_usd: 0.01,
        pid: 99999,
        socket_path: PathBuf::from("/tmp/forge-1000/sessions/bare.sock"),
    };

    write_meta(&path, &meta).await.unwrap();
    let loaded = read_meta(&path).await.unwrap();

    assert_eq!(meta.agent, loaded.agent);
    assert_eq!(meta.provider_id, loaded.provider_id);
    assert_eq!(meta.model, loaded.model);
    assert_eq!(meta.state, loaded.state);
    assert_eq!(meta.persistence, loaded.persistence);
    assert_eq!(meta.ended_at, loaded.ended_at);
}

#[tokio::test]
async fn meta_toml_creates_parent_dirs() {
    let dir = TempDir::new().unwrap();
    let path = dir
        .path()
        .join(".forge")
        .join("sessions")
        .join("abc123")
        .join("meta.toml");

    let meta = SessionMeta {
        id: SessionId::new(),
        workspace_id: WorkspaceId::new(),
        name: "test".to_string(),
        agent: None,
        provider_id: None,
        model: None,
        state: SessionState::Active,
        persistence: SessionPersistence::Persist,
        started_at: Utc::now().with_nanosecond(0).unwrap(),
        ended_at: None,
        tokens_in: 0,
        tokens_out: 0,
        cost_usd: 0.0,
        pid: 1,
        socket_path: PathBuf::from("/tmp/test.sock"),
    };

    write_meta(&path, &meta).await.unwrap();
    assert!(path.exists());
}

trait WithNanosecond {
    fn with_nanosecond(self, ns: u32) -> Option<Self>
    where
        Self: Sized;
}

impl WithNanosecond for chrono::DateTime<chrono::Utc> {
    fn with_nanosecond(self, ns: u32) -> Option<Self> {
        chrono::Timelike::with_nanosecond(&self, ns)
    }
}
