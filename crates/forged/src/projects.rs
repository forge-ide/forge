//! `project.open`: resolve a project root, load or create its `forge.toml`, and record it.

use std::path::{Path, PathBuf};

use forge_api::ApiError;
use forge_api::model::{Project, Repo};
use forge_config::{ConfigError, PROJECT_CONFIG_FILE, ProjectConfig};
use tokio::process::Command;

use crate::db::Db;
use crate::rpc::HandlerError;

pub async fn open(db: &Db, path: &str) -> Result<Project, HandlerError> {
    let requested = PathBuf::from(path);
    if !requested.is_absolute() {
        return Err(ApiError::validation(format!("path {path:?} must be absolute")).into());
    }
    let root = match tokio::fs::canonicalize(&requested).await {
        Ok(root) => root,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ApiError::not_found(format!("no such directory: {path}")).into());
        }
        Err(e) => return Err(ApiError::validation(format!("{path}: {e}")).into()),
    };
    if !tokio::fs::metadata(&root)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
    {
        return Err(ApiError::validation(format!("{path} is not a directory")).into());
    }
    let Some(root_str) = root.to_str().map(str::to_owned) else {
        return Err(ApiError::validation(format!("{path} is not valid UTF-8")).into());
    };

    let config = load_or_create(&root).await?;
    for repo in &config.repos {
        if !tokio::fs::metadata(root.join(&repo.path))
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            return Err(ApiError::validation(format!(
                "{PROJECT_CONFIG_FILE}: repo {:?} path {:?} is not a directory",
                repo.name, repo.path
            ))
            .into());
        }
    }

    let repos = config
        .repos
        .into_iter()
        .map(|r| Repo {
            name: r.name,
            path: r.path,
            base_branch: r.base_branch,
        })
        .collect();
    Ok(db
        .upsert_project(root_str, config.project.name, repos)
        .await?)
}

async fn load_or_create(root: &Path) -> Result<ProjectConfig, HandlerError> {
    let dir = root.to_path_buf();
    let loaded = tokio::task::spawn_blocking(move || ProjectConfig::load(&dir))
        .await
        .map_err(anyhow::Error::from)?;
    match loaded {
        Ok(Some(config)) => return Ok(config),
        Ok(None) => {}
        Err(e) => return Err(ApiError::validation(e.to_string()).into()),
    }

    let config = default_config(root).await?;
    let dir = root.to_path_buf();
    match tokio::task::spawn_blocking(move || write_or_load(&dir, config))
        .await
        .map_err(anyhow::Error::from)?
    {
        Ok(config) => Ok(config),
        Err(e @ ConfigError::Io { .. }) => {
            Err(anyhow::anyhow!("writing {PROJECT_CONFIG_FILE}: {e}").into())
        }
        Err(e) => Err(ApiError::validation(e.to_string()).into()),
    }
}

/// Write the default config, or, if another `project.open` wrote one first, use that.
fn write_or_load(root: &Path, config: ProjectConfig) -> Result<ProjectConfig, ConfigError> {
    match config.write_new(root) {
        Ok(()) => {
            tracing::info!(root = %root.display(), "wrote default {PROJECT_CONFIG_FILE}");
            Ok(config)
        }
        Err(ConfigError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            let path = root.join(PROJECT_CONFIG_FILE);
            ProjectConfig::load(root)?.ok_or_else(|| ConfigError::Io {
                path,
                source: std::io::Error::other("disappeared while opening the project"),
            })
        }
        Err(e) => Err(e),
    }
}

/// A single-repo config for a git repository root, on its current branch.
async fn default_config(root: &Path) -> Result<ProjectConfig, HandlerError> {
    let no_config = format!("{} has no {PROJECT_CONFIG_FILE}", root.display());
    let toplevel = git(root, &["rev-parse", "--show-toplevel"]).await.ok_or_else(|| {
        ApiError::validation(format!(
            "{no_config} and is not a git repository; run git init or write {PROJECT_CONFIG_FILE} by hand"
        ))
    })?;
    let toplevel = tokio::fs::canonicalize(&toplevel)
        .await
        .map_err(|e| anyhow::anyhow!("resolving git toplevel {toplevel}: {e}"))?;
    if toplevel != root {
        return Err(ApiError::validation(format!(
            "{no_config} and is inside the git repository at {}; open the repository root",
            toplevel.display()
        ))
        .into());
    }
    let branch = git(root, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        .await
        .ok_or_else(|| {
            ApiError::validation(format!(
                "{no_config} and HEAD is detached; check out a branch or write {PROJECT_CONFIG_FILE} by hand"
            ))
        })?;
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "project".to_owned());
    Ok(ProjectConfig::single_repo(&name, &branch))
}

/// Run a read-only git command in `dir`; its trimmed stdout on success.
async fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let out = String::from_utf8(output.stdout).ok()?;
    let out = out.trim();
    (!out.is_empty()).then(|| out.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_a_config_written_concurrently() {
        let dir = tempfile::tempdir().unwrap();
        let theirs = ProjectConfig::single_repo("theirs", "trunk");
        theirs.write_new(dir.path()).unwrap();

        let ours = ProjectConfig::single_repo("ours", "main");
        assert_eq!(write_or_load(dir.path(), ours).unwrap(), theirs);
    }

    #[test]
    fn writes_when_absent() {
        let dir = tempfile::tempdir().unwrap();
        let ours = ProjectConfig::single_repo("ours", "main");
        assert_eq!(write_or_load(dir.path(), ours.clone()).unwrap(), ours);
        assert_eq!(ProjectConfig::load(dir.path()).unwrap(), Some(ours));
    }
}
