use crate::{Result, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceEntry {
    pub id: WorkspaceId,
    pub path: PathBuf,
    pub name: String,
    pub last_opened: DateTime<Utc>,
    pub pinned: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    // First-run path: the registry file is created lazily on the first
    // session spawn. Treat a missing file as an empty registry so the
    // new-session flow doesn't fail before it has a chance to write one.
    let contents = match tokio::fs::read_to_string(path).await {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.into()),
    };
    let file: WorkspacesFile = toml::from_str(&contents).map_err(|e| anyhow::anyhow!(e))?;
    Ok(file.workspaces)
}

/// Append a new entry for `canonical_path` to the registry if no entry
/// for that path already exists. Idempotent: returns `Ok(())` with no
/// write when an entry with the same canonical path is already present.
///
/// `canonical_path` MUST already be canonicalized by the caller — the
/// existence check canonicalizes each registered entry on read so a
/// symlink-equivalent entry counts as a match.
///
/// Used by the dashboard-window `session_start` flow to self-bootstrap
/// the registry the first time a workspace is opened via the file
/// picker. The F-349 trust boundary stays intact: session-window
/// callers never reach this path; they resolve through the
/// `cached_workspace_root` cache populated by `session_hello`.
pub async fn register_workspace_if_missing(
    registry_path: &Path,
    canonical_path: &Path,
) -> Result<()> {
    register_or_get_workspace_id(registry_path, canonical_path)
        .await
        .map(|_| ())
}

/// Register `canonical_path` if absent and return the workspace's stable
/// `WorkspaceId`. The id is the value persisted in the registry — callers
/// (the shell's window manager and session-start IPC) use it to construct
/// the workspace-keyed Tauri window label (`workspace-<id>`).
///
/// Idempotent: a second call for the same canonical path returns the
/// existing id without rewriting the registry. `canonical_path` MUST
/// already be canonicalized by the caller — the existence check
/// canonicalizes each registered entry on read so a symlink-equivalent
/// entry still counts as a match.
pub async fn register_or_get_workspace_id(
    registry_path: &Path,
    canonical_path: &Path,
) -> Result<WorkspaceId> {
    let mut entries = read_workspaces(registry_path).await?;
    if let Some(existing) = entries.iter().find(|e| {
        e.path
            .canonicalize()
            .map(|c| c == canonical_path)
            .unwrap_or(false)
    }) {
        return Ok(existing.id.clone());
    }
    let id = WorkspaceId::new();
    let name = canonical_path
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| canonical_path.display().to_string());
    entries.push(WorkspaceEntry {
        id: id.clone(),
        path: canonical_path.to_path_buf(),
        name,
        last_opened: Utc::now(),
        pinned: false,
    });
    write_workspaces(registry_path, &entries).await?;
    Ok(id)
}

/// Look up the `WorkspaceId` for a canonical path without registering on
/// miss. Returns `None` when no entry matches. Used by the dashboard's
/// `open_session` path: a session's meta.toml already carries the
/// workspace_id, but cross-checking it against the registry catches a
/// drifted meta where the registry has been rebuilt without the entry.
pub async fn lookup_workspace_id(
    registry_path: &Path,
    canonical_path: &Path,
) -> Result<Option<WorkspaceId>> {
    let entries = read_workspaces(registry_path).await?;
    Ok(entries
        .into_iter()
        .find(|e| {
            e.path
                .canonicalize()
                .map(|c| c == canonical_path)
                .unwrap_or(false)
        })
        .map(|e| e.id))
}
