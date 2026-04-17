use crate::{Result, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceEntry {
    pub id: WorkspaceId,
    pub path: PathBuf,
    pub name: String,
    pub last_opened: DateTime<Utc>,
    pub pinned: bool,
}

#[derive(Serialize, Deserialize)]
struct WorkspacesFile {
    #[serde(default)]
    workspaces: Vec<WorkspaceEntry>,
}

pub async fn write_workspaces(path: &Path, entries: &[WorkspaceEntry]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let file = WorkspacesFile {
        workspaces: entries.to_vec(),
    };
    let contents = toml::to_string(&file).map_err(|e| anyhow::anyhow!(e))?;
    tokio::fs::write(path, contents).await?;
    Ok(())
}

pub async fn read_workspaces(path: &Path) -> Result<Vec<WorkspaceEntry>> {
    let contents = tokio::fs::read_to_string(path).await?;
    let file: WorkspacesFile = toml::from_str(&contents).map_err(|e| anyhow::anyhow!(e))?;
    Ok(file.workspaces)
}
