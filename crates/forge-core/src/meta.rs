use crate::{
    AgentId, ProviderId, Result, SessionId, SessionPersistence, SessionState, WorkspaceId,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    // Atomic write: stage in a sibling temp file then rename in. A plain
    // `write` is truncate-then-write, so a concurrent reader (e.g. the
    // dashboard polling meta.toml) can otherwise observe a zero-byte or
    // half-written file. The rename is atomic on the same filesystem.
    let tmp = match path.file_name() {
        Some(name) => {
            let mut tmp_name = name.to_os_string();
            tmp_name.push(".tmp");
            path.with_file_name(tmp_name)
        }
        None => path.with_extension("toml.tmp"),
    };
    tokio::fs::write(&tmp, &contents).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

pub async fn read_meta(path: &Path) -> Result<SessionMeta> {
    let contents = tokio::fs::read_to_string(path).await?;
    let meta = toml::from_str(&contents).map_err(|e| anyhow::anyhow!(e))?;
    Ok(meta)
}
