use crate::{
    AgentId, ProviderId, Result, SessionId, SessionPersistence, SessionState, WorkspaceId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<ProviderId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub state: SessionState,
    pub persistence: SessionPersistence,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub cost_usd: f64,
    pub pid: u32,
    pub socket_path: PathBuf,
}

pub async fn write_meta(path: &Path, meta: &SessionMeta) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let contents = toml::to_string(meta).map_err(|e| anyhow::anyhow!(e))?;
    tokio::fs::write(path, contents).await?;
    Ok(())
}

pub async fn read_meta(path: &Path) -> Result<SessionMeta> {
    let contents = tokio::fs::read_to_string(path).await?;
    let meta = toml::from_str(&contents).map_err(|e| anyhow::anyhow!(e))?;
    Ok(meta)
}
