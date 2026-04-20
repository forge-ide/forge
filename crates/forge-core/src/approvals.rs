//! Persistent approval whitelist (F-036).
//!
//! Extends the session-only whitelist with two persistent tiers — **workspace**
//! (`<root>/.forge/approvals.toml`) and **user** (`{config_dir}/forge/approvals.toml`).
//! On session start both files are loaded and seed the in-memory whitelist so
//! previously-approved calls auto-approve without prompting.
//!
//! Shape mirrors the frontend `ApprovalWhitelist.entries` record: each entry
//! stores the deterministic `scope_key` (e.g. `file:fs.write:/src/foo.ts`), the
//! `tool_name` the key was derived from, and a short human `label` used by the
//! `WhitelistedPill` ("this file", "this tool", "pattern /src/*").
//!
//! Atomic writes: per DoD, `save_*` writes to `<path>.tmp` then renames. The
//! rename is atomic on POSIX for same-filesystem targets, so a partially-
//! written `.tmp` never becomes the visible config.
//!
//! Both `load_*` return an empty config when the file is absent — missing
//! configs are the common first-run case, not an error.
//!
//! Merge precedence: workspace wins on `scope_key` collision with user, mirror-
//! ing `forge-mcp::config::load_merged`. The shell surfaces both tiers to the
//! frontend so the pill can show provenance, but for auto-approval a single
//! winning entry per key is what the whitelist stores.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::Result;

/// Single persisted approval entry. Wire-compatible with the frontend
/// whitelist record (one `scope_key` → `{label, level}`) after the shell layer
/// tacks on a `level` discriminator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct ApprovalEntry {
    pub scope_key: String,
    pub tool_name: String,
    pub label: String,
}

/// Top-level TOML shape. A bare `ApprovalConfig` at serialization time becomes
/// `entries = [...]` — a simple array of tables.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct ApprovalConfig {
    #[serde(default)]
    pub entries: Vec<ApprovalEntry>,
}

/// Path of the workspace-scoped approvals file under `<root>/.forge/approvals.toml`.
pub fn workspace_config_path(workspace_root: &Path) -> PathBuf {
    workspace_root.join(".forge").join("approvals.toml")
}

/// Path of the user-scoped approvals file under `{config_dir}/forge/approvals.toml`.
///
/// Uses the platform-appropriate XDG/Known-Folder config dir via the `dirs`
/// crate (same resolution `forge-shell::dashboard_sessions` uses for
/// `workspaces.toml`). Callers that already resolved a config dir can instead
/// use [`user_config_path_in`].
pub fn user_config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|base| base.join("forge").join("approvals.toml"))
}

/// Test seam / caller-supplied variant of [`user_config_path`].
pub fn user_config_path_in(config_dir: &Path) -> PathBuf {
    config_dir.join("forge").join("approvals.toml")
}

/// Read the user-scoped config. Returns an empty `ApprovalConfig` if the file
/// is absent. Errors only when the file exists but is unreadable or malformed.
pub async fn load_user_config() -> Result<ApprovalConfig> {
    match user_config_path() {
        Some(p) => load_from_path(&p).await,
        None => Ok(ApprovalConfig::default()),
    }
}

/// Test-friendly variant of [`load_user_config`] that takes an explicit config
/// dir (so tests can point at a tempdir).
pub async fn load_user_config_in(config_dir: &Path) -> Result<ApprovalConfig> {
    load_from_path(&user_config_path_in(config_dir)).await
}

/// Read the workspace-scoped config from `<root>/.forge/approvals.toml`.
/// Returns an empty config if the file is absent.
pub async fn load_workspace_config(workspace_root: &Path) -> Result<ApprovalConfig> {
    load_from_path(&workspace_config_path(workspace_root)).await
}

/// Write the user-scoped config atomically.
pub async fn save_user_config(config: &ApprovalConfig) -> Result<()> {
    let path = user_config_path()
        .ok_or_else(|| anyhow::anyhow!("could not resolve user config directory"))?;
    save_to_path(&path, config).await
}

/// Test-friendly variant of [`save_user_config`].
pub async fn save_user_config_in(config_dir: &Path, config: &ApprovalConfig) -> Result<()> {
    save_to_path(&user_config_path_in(config_dir), config).await
}

/// Write the workspace-scoped config atomically under `<root>/.forge/approvals.toml`.
pub async fn save_workspace_config(workspace_root: &Path, config: &ApprovalConfig) -> Result<()> {
    save_to_path(&workspace_config_path(workspace_root), config).await
}

async fn load_from_path(path: &Path) -> Result<ApprovalConfig> {
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
            let cfg: ApprovalConfig = toml::from_str(&contents).map_err(|e| anyhow::anyhow!(e))?;
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ApprovalConfig::default()),
        Err(e) => Err(e.into()),
    }
}

/// Atomically write `config` to `path`. Per DoD: write to `<path>.tmp`, then
/// rename. The rename is atomic on POSIX for same-filesystem targets, and
/// same-directory by construction here, so a partially-written tmp file never
/// becomes the visible config.
async fn save_to_path(path: &Path, config: &ApprovalConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let body = toml::to_string(config).map_err(|e| anyhow::anyhow!(e))?;

    let tmp = tmp_path_for(path);
    tokio::fs::write(&tmp, body).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    file_name.push(".tmp");
    match path.parent() {
        Some(parent) => parent.join(file_name),
        None => PathBuf::from(file_name),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn entry(key: &str, tool: &str, label: &str) -> ApprovalEntry {
        ApprovalEntry {
            scope_key: key.into(),
            tool_name: tool.into(),
            label: label.into(),
        }
    }

    #[test]
    fn toml_roundtrip_preserves_entries() {
        let cfg = ApprovalConfig {
            entries: vec![
                entry("file:fs.write:/src/foo.ts", "fs.write", "this file"),
                entry("tool:shell.exec", "shell.exec", "this tool"),
                entry("pattern:fs.edit:/src/*", "fs.edit", "pattern /src/*"),
            ],
        };
        let body = toml::to_string(&cfg).unwrap();
        let decoded: ApprovalConfig = toml::from_str(&body).unwrap();
        assert_eq!(cfg, decoded);
    }

    #[test]
    fn empty_config_roundtrips() {
        let cfg = ApprovalConfig::default();
        let body = toml::to_string(&cfg).unwrap();
        let decoded: ApprovalConfig = toml::from_str(&body).unwrap();
        assert_eq!(cfg, decoded);
        assert!(decoded.entries.is_empty());
    }

    #[test]
    fn deserialization_rejects_unknown_fields() {
        let body =
            "[[entries]]\nscope_key = \"k\"\ntool_name = \"t\"\nlabel = \"l\"\nmystery = \"x\"\n";
        let res: std::result::Result<ApprovalConfig, _> = toml::from_str(body);
        assert!(res.is_err(), "unknown fields must fail");
    }

    #[tokio::test]
    async fn load_workspace_missing_returns_empty() {
        let dir = TempDir::new().unwrap();
        let cfg = load_workspace_config(dir.path()).await.unwrap();
        assert_eq!(cfg, ApprovalConfig::default());
    }

    #[tokio::test]
    async fn load_user_missing_returns_empty() {
        let dir = TempDir::new().unwrap();
        let cfg = load_user_config_in(dir.path()).await.unwrap();
        assert_eq!(cfg, ApprovalConfig::default());
    }

    #[tokio::test]
    async fn save_then_load_workspace_roundtrips() {
        let dir = TempDir::new().unwrap();
        let cfg = ApprovalConfig {
            entries: vec![entry("tool:fs.write", "fs.write", "this tool")],
        };
        save_workspace_config(dir.path(), &cfg).await.unwrap();
        let loaded = load_workspace_config(dir.path()).await.unwrap();
        assert_eq!(loaded, cfg);
    }

    #[tokio::test]
    async fn save_then_load_user_roundtrips() {
        let dir = TempDir::new().unwrap();
        let cfg = ApprovalConfig {
            entries: vec![entry("file:fs.edit:/a.ts", "fs.edit", "this file")],
        };
        save_user_config_in(dir.path(), &cfg).await.unwrap();
        let loaded = load_user_config_in(dir.path()).await.unwrap();
        assert_eq!(loaded, cfg);
    }

    #[tokio::test]
    async fn save_workspace_creates_dot_forge_dir() {
        let dir = TempDir::new().unwrap();
        // `.forge` does not yet exist; save must create it.
        assert!(!dir.path().join(".forge").exists());
        save_workspace_config(dir.path(), &ApprovalConfig::default())
            .await
            .unwrap();
        assert!(dir.path().join(".forge").join("approvals.toml").exists());
    }

    #[tokio::test]
    async fn save_is_atomic_via_tmp_and_rename() {
        // After save, no `.tmp` residue should remain, the real file must exist,
        // and intermediate reads during write must observe either the old body
        // or the new body — never a half-written file. We check the residue +
        // final state as a proxy for the rename sequencing.
        let dir = TempDir::new().unwrap();
        let cfg = ApprovalConfig {
            entries: vec![entry("tool:x", "x", "this tool")],
        };
        save_workspace_config(dir.path(), &cfg).await.unwrap();

        let final_path = dir.path().join(".forge").join("approvals.toml");
        let tmp_path = dir.path().join(".forge").join("approvals.toml.tmp");
        assert!(final_path.exists(), "final file must exist");
        assert!(!tmp_path.exists(), "tmp must be renamed, not left behind");

        // Overwrite with fresh contents and re-verify no residue.
        let cfg2 = ApprovalConfig::default();
        save_workspace_config(dir.path(), &cfg2).await.unwrap();
        assert!(final_path.exists());
        assert!(!tmp_path.exists());
        let loaded = load_workspace_config(dir.path()).await.unwrap();
        assert_eq!(loaded, cfg2);
    }

    #[test]
    fn workspace_config_path_is_under_dot_forge() {
        let p = workspace_config_path(Path::new("/repo"));
        assert_eq!(p, Path::new("/repo/.forge/approvals.toml"));
    }

    #[test]
    fn user_config_path_in_nests_under_forge() {
        let p = user_config_path_in(Path::new("/xdg"));
        assert_eq!(p, Path::new("/xdg/forge/approvals.toml"));
    }
}
