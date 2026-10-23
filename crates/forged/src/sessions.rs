//! Session methods: agent profiles, sandbox tiers, and branch names, in front of the manager.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_acp::AgentCommand;
use forge_api::methods::{BranchPreview, BranchPreviewParams, SessionCreateParams, SessionOutcome};
use forge_api::model::{Project, SandboxTier, Session};
use forge_api::{ApiError, ErrorKind, ProjectId};
use forge_config::{
    AgentProfileConfig, BranchRules, ConfigError, Paths, ProfileSandbox, ProjectConfig, UserConfig,
};
use forge_sandbox::proxy::{Allowlist, SecretRoute};
use forge_sandbox::secrets::{self, SecretStore};
use forge_sandbox::{Sandbox, TierStatus};
use forge_session::{AgentLaunch, AgentResolver, CreateRequest, SessionError};

use crate::db::Db;
use crate::rpc::{HandlerError, State};

impl From<SessionError> for HandlerError {
    fn from(e: SessionError) -> Self {
        match e {
            SessionError::Api(e) => HandlerError::Api(e),
            SessionError::Internal(e) => HandlerError::Internal(anyhow::anyhow!(e)),
        }
    }
}

pub async fn create(state: &State, params: SessionCreateParams) -> Result<Session, HandlerError> {
    let project = project(&state.db, &params.project_id).await?;
    let root = PathBuf::from(&project.root);
    let profile = load_profile(&root, &params.agent_profile).await?;
    let sandbox = sandbox_tier(
        params.sandbox,
        &profile,
        &params.agent_profile,
        &state.sandbox.tiers(),
    )?;
    allowlist(&profile, &params.agent_profile)?;
    known_secrets(&profile, &params.agent_profile)?;
    let (config, rules) = branch_rules(state, &root).await?;
    let branch = match params.branch {
        Some(branch) => branch,
        None => forge_git::derive_branch(&params.title, &rules)
            .map_err(|e| ApiError::validation(e.to_string()))?,
    };
    let request = CreateRequest {
        project,
        title: params.title,
        prompt: params.prompt,
        agent_profile: params.agent_profile,
        mode: params.mode,
        sandbox,
        branch,
        on_collision: params.on_collision,
        max_concurrent: config.sessions.max_concurrent,
    };
    Ok(state.sessions.create(request).await?)
}

pub async fn branch_preview(
    state: &State,
    params: BranchPreviewParams,
) -> Result<BranchPreview, HandlerError> {
    let project = project(&state.db, &params.project_id).await?;
    let (_, rules) = branch_rules(state, Path::new(&project.root)).await?;
    let branch = match params.branch {
        Some(branch) => branch,
        None => forge_git::derive_branch(&params.title, &rules)
            .map_err(|e| ApiError::validation(e.to_string()))?,
    };
    let collisions = state.sessions.preview(&project, &branch).await?;
    Ok(BranchPreview { branch, collisions })
}

pub fn close_reason(outcome: SessionOutcome) -> &'static str {
    match outcome {
        SessionOutcome::Shipped => "shipped",
        SessionOutcome::Abandoned => "abandoned",
    }
}

async fn project(db: &Db, id: &ProjectId) -> Result<Project, HandlerError> {
    db.get_project(id.clone())
        .await?
        .ok_or_else(|| ApiError::not_found(format!("project {id}")).into())
}

/// The project config and the branch rules from it layered over the user's.
async fn branch_rules(
    state: &State,
    root: &Path,
) -> Result<(ProjectConfig, BranchRules), HandlerError> {
    let (root, user_path) = (root.to_path_buf(), state.paths.user_config());
    let loaded = tokio::task::spawn_blocking(move || {
        let user = UserConfig::load(&user_path)?;
        let project = ProjectConfig::load(&root)?;
        Ok::<_, ConfigError>((user, project))
    })
    .await
    .map_err(anyhow::Error::from)?;
    let (user, project) = loaded.map_err(|e| ApiError::validation(e.to_string()))?;
    let project = project.ok_or_else(|| {
        ApiError::validation(format!(
            "{} is gone; reopen the project",
            forge_config::PROJECT_CONFIG_FILE
        ))
    })?;
    let rules = project.branches.resolve(&user.branches);
    Ok((project, rules))
}

async fn load_profile(root: &Path, id: &str) -> Result<AgentProfileConfig, HandlerError> {
    let path =
        AgentProfileConfig::path(root, id).map_err(|e| ApiError::validation(e.to_string()))?;
    let file = path.clone();
    let loaded = tokio::task::spawn_blocking(move || AgentProfileConfig::load(&file))
        .await
        .map_err(anyhow::Error::from)?;
    match loaded {
        Ok(Some(profile)) => Ok(profile),
        Ok(None) => {
            Err(ApiError::not_found(format!("agent profile {id:?}: no {}", path.display())).into())
        }
        Err(e) => Err(ApiError::validation(e.to_string()).into()),
    }
}

/// The tier a session runs in. Only tiers this machine can run are accepted, and running
/// unsandboxed needs the profile's explicit opt-in; nothing falls back to a weaker tier.
fn sandbox_tier(
    requested: Option<SandboxTier>,
    profile: &AgentProfileConfig,
    id: &str,
    tiers: &[TierStatus],
) -> Result<SandboxTier, ApiError> {
    let tier = requested.unwrap_or(match profile.sandbox.unwrap_or_default() {
        ProfileSandbox::Light => SandboxTier::Light,
        ProfileSandbox::Standard => SandboxTier::Standard,
        ProfileSandbox::Container => SandboxTier::Container,
        ProfileSandbox::Unsandboxed => SandboxTier::Unsandboxed,
    });
    if tier == SandboxTier::Unsandboxed && !profile.allow_unsandboxed {
        return Err(ApiError::new(
            ErrorKind::SandboxUnavailable,
            format!(
                "agent profile {id:?} does not allow running unsandboxed; set allow_unsandboxed = true in it to opt in"
            ),
        ));
    }
    let name = forge_sandbox::tier_name(tier);
    match tiers.iter().find(|t| t.tier == tier) {
        Some(status) if status.is_available() => Ok(tier),
        Some(TierStatus {
            unavailable: Some(why),
            ..
        }) => Err(ApiError::new(
            ErrorKind::SandboxUnavailable,
            format!("the {name} sandbox tier is not available on this machine: {why}"),
        )),
        _ => Err(ApiError::new(
            ErrorKind::SandboxUnavailable,
            format!(
                "the {name} sandbox tier is not available on this machine; server.hello lists the tiers that are"
            ),
        )),
    }
}

/// Finds how to start a session's agent from its profile, rechecking that its tier is allowed.
pub struct ProfileResolver {
    pub db: Db,
    pub sandbox: Arc<dyn Sandbox>,
    pub paths: Paths,
    pub home: PathBuf,
    pub secrets: Arc<dyn SecretStore>,
}

impl AgentResolver for ProfileResolver {
    fn launch(&self, session: &Session) -> Result<AgentLaunch, ApiError> {
        let root = self
            .db
            .project_root(&session.project_id)
            .map_err(|e| ApiError::new(ErrorKind::AgentUnavailable, e.to_string()))?
            .ok_or_else(|| ApiError::not_found(format!("project {}", session.project_id)))?;
        let id = &session.agent_profile;
        let path =
            AgentProfileConfig::path(&root, id).map_err(|e| ApiError::validation(e.to_string()))?;
        let profile = AgentProfileConfig::load(&path)
            .map_err(|e| ApiError::validation(e.to_string()))?
            .ok_or_else(|| ApiError::not_found(format!("agent profile {id:?}")))?;
        sandbox_tier(Some(session.sandbox), &profile, id, &self.sandbox.tiers())?;
        let network = allowlist(&profile, id)?;
        let secrets = known_secrets(&profile, id)?
            .into_iter()
            .map(|secret| {
                let value = self.secrets.get(secret.name).map_err(|e| {
                    ApiError::new(
                        ErrorKind::AgentUnavailable,
                        format!("agent profile {id:?}: {e}"),
                    )
                })?;
                Ok(SecretRoute { secret, value })
            })
            .collect::<Result<Vec<_>, ApiError>>()?;
        let mut argv = profile.command.ok_or_else(|| {
            ApiError::new(
                ErrorKind::AgentUnavailable,
                format!("agent profile {id:?} has no command"),
            )
        })?;
        let user = UserConfig::load(&self.paths.user_config())
            .map_err(|e| ApiError::validation(e.to_string()))?;
        let mut read_only = checked_read_only(
            &user.sandbox.read_only,
            &[&self.home, &self.paths.data_dir, &self.paths.runtime_dir],
        )?;
        // The agent program must be visible inside the sandbox; run it by its real path, since
        // a symlink to it may sit in a hidden directory.
        let program = resolve_program(&argv[0]).ok_or_else(|| {
            ApiError::new(
                ErrorKind::AgentUnavailable,
                format!("agent profile {id:?}: {} is not installed", argv[0]),
            )
        })?;
        argv[0] = program.display().to_string();
        read_only.push(program);
        Ok(AgentLaunch {
            command: AgentCommand { argv, env: vec![] },
            read_only,
            network,
            secrets,
        })
    }
}

/// The profile's `[network] allow`, as the egress proxy's allowlist.
fn allowlist(profile: &AgentProfileConfig, id: &str) -> Result<Allowlist, ApiError> {
    Allowlist::new(&profile.network.allow)
        .map_err(|e| ApiError::validation(format!("agent profile {id:?}: network.allow: {e}")))
}

/// The secrets the profile's `env_secrets` names; each must be one Forge knows how to inject.
fn known_secrets(
    profile: &AgentProfileConfig,
    id: &str,
) -> Result<Vec<&'static secrets::KnownSecret>, ApiError> {
    let mut seen = std::collections::HashSet::new();
    profile
        .env_secrets
        .iter()
        .filter(|name| seen.insert(name.as_str()))
        .map(|name| {
            secrets::known(name).ok_or_else(|| {
                ApiError::validation(format!(
                    "agent profile {id:?}: env_secrets: Forge can't inject {name:?}; it knows {}",
                    secrets::known_names()
                ))
            })
        })
        .collect()
}

/// The user config's extra read-only paths, refusing any that would expose a protected dir:
/// the home dir itself, or anything containing Forge's data or runtime dir.
fn checked_read_only(paths: &[PathBuf], protected: &[&Path]) -> Result<Vec<PathBuf>, ApiError> {
    for path in paths {
        let resolved = resolve(path);
        // Compare as written and as resolved, so neither a symlink nor a missing path slips by.
        let covers = |p: &&Path| p.starts_with(path) || resolve(p).starts_with(&resolved);
        if protected.iter().any(covers) {
            return Err(ApiError::validation(format!(
                "sandbox.read_only in the user config: {} would expose the home dir or Forge's own data",
                path.display()
            )));
        }
    }
    Ok(paths.to_vec())
}

/// `path` with its deepest existing ancestor canonicalized and the rest appended, so paths
/// that don't exist yet compare consistently with ones that do (`/home` is an automount on
/// macOS; `/var` is `/private/var`).
fn resolve(path: &Path) -> PathBuf {
    let mut rest = Vec::new();
    let mut current = path;
    loop {
        if let Ok(real) = std::fs::canonicalize(current) {
            return rest.iter().rev().fold(real, |acc, c| acc.join(c));
        }
        match (current.parent(), current.file_name()) {
            (Some(parent), Some(name)) => {
                rest.push(name.to_owned());
                current = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// `program` as an absolute, canonical path, looked up in `PATH` when it has no slash.
fn resolve_program(program: &str) -> Option<PathBuf> {
    let found = if program.contains('/') {
        PathBuf::from(program)
    } else {
        let path = std::env::var_os("PATH")?;
        std::env::split_paths(&path)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())?
    };
    std::fs::canonicalize(found).ok().filter(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_paths_may_not_cover_protected_dirs() {
        let home = Path::new("/home/u");
        let data = Path::new("/home/u/.local/share/forge");
        let ok = vec![
            PathBuf::from("/home/u/.local/share/agents"),
            PathBuf::from("/opt/x"),
        ];
        assert_eq!(checked_read_only(&ok, &[home, data]).unwrap(), ok);
        for bad in [
            "/home/u",
            "/home",
            "/home/u/.local",
            "/home/u/.local/share/forge",
        ] {
            assert!(
                checked_read_only(&[PathBuf::from(bad)], &[home, data]).is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn missing_paths_resolve_through_their_existing_ancestors() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().canonicalize().unwrap();
        let link = real.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert_eq!(resolve(&link.join("a/b")), real.join("a/b"));
        assert_eq!(resolve(Path::new("/")), PathBuf::from("/"));
        // Protecting a missing dir behind a symlink still refuses its resolved parent.
        let protected = link.join("data/forge");
        assert!(checked_read_only(&[real.join("data")], &[&protected]).is_err());
    }

    #[test]
    fn programs_resolve_through_path_to_their_real_file() {
        let sh = resolve_program("sh").unwrap();
        assert!(sh.is_absolute() && sh.is_file());
        assert_eq!(resolve_program(sh.to_str().unwrap()), Some(sh));
        assert_eq!(resolve_program("forge-no-such-program"), None);
        assert_eq!(resolve_program("/"), None);
    }
}
