//! Session methods: agent profiles, sandbox tiers, and branch names, in front of the manager.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_acp::{AgentCommand, adapters};
use forge_api::methods::{
    BranchPreview, BranchPreviewParams, SessionCreateParams, SessionOutcome, SessionSetModeParams,
};
use forge_api::model::{Project, SandboxTier, Session};
use forge_api::{ApiError, ErrorKind, ProjectId};
use forge_config::{
    AgentProfileConfig, BranchRules, ConfigError, HooksConfig, PROJECT_HOOKS_FILE,
    PROJECT_POLICY_FILE, Paths, PolicyConfig, PolicyRule, ProfileSandbox, ProjectConfig,
    RuleListName, UserConfig,
};
use forge_hooks::HookSet;
use forge_policy::{Layer, Rules};
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

/// Change a session's mode. Its sandbox tier is fixed when it starts, so asking for another is
/// refused.
pub async fn set_mode(
    state: &State,
    params: SessionSetModeParams,
) -> Result<Session, HandlerError> {
    let session = state.sessions.get(&params.session_id).await?;
    if let Some(tier) = params.sandbox
        && tier != session.sandbox
    {
        return Err(ApiError::validation(format!(
            "session {} runs in the {} sandbox tier, which can't change while it runs; start a \
             new session for another tier",
            session.id,
            forge_sandbox::tier_name(session.sandbox)
        ))
        .into());
    }
    Ok(state.sessions.set_mode(&session.id, params.mode).await?)
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
    let (dir, name) = (root.to_path_buf(), id.to_owned());
    let loaded = tokio::task::spawn_blocking(move || crate::agents::load(&dir, &name))
        .await
        .map_err(anyhow::Error::from)?;
    match loaded? {
        Some(profile) => Ok(profile),
        None => Err(ApiError::not_found(format!(
            "agent profile {id:?}: no {} and no built-in profile of that name",
            Path::new(forge_config::AGENTS_DIR)
                .join(format!("{id}.toml"))
                .display()
        ))
        .into()),
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

impl ProfileResolver {
    /// The session's project root and agent profile.
    fn profile(&self, session: &Session) -> Result<(PathBuf, AgentProfileConfig), ApiError> {
        let root = self
            .db
            .project_root(&session.project_id)
            .map_err(|e| ApiError::new(ErrorKind::AgentUnavailable, e.to_string()))?
            .ok_or_else(|| ApiError::not_found(format!("project {}", session.project_id)))?;
        let id = &session.agent_profile;
        let profile = crate::agents::load(&root, id)?
            .ok_or_else(|| ApiError::not_found(format!("agent profile {id:?}")))?;
        Ok((root, profile))
    }

    /// The secrets the profile injects, with their values from the keyring.
    fn routes(&self, profile: &AgentProfileConfig, id: &str) -> Result<Vec<SecretRoute>, ApiError> {
        known_secrets(profile, id)?
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
            .collect()
    }
}

impl AgentResolver for ProfileResolver {
    fn secret_values(&self, session: &Session) -> Result<Vec<String>, ApiError> {
        let (_, profile) = self.profile(session)?;
        let routes = self.routes(&profile, &session.agent_profile)?;
        Ok(routes.iter().map(|r| r.value.expose().to_owned()).collect())
    }

    fn launch(&self, session: &Session) -> Result<AgentLaunch, ApiError> {
        let (root, profile) = self.profile(session)?;
        let id = &session.agent_profile;
        sandbox_tier(Some(session.sandbox), &profile, id, &self.sandbox.tiers())?;
        let network = allowlist(&profile, id)?;
        let secrets = self.routes(&profile, id)?;
        let command = crate::agents::command(&profile).ok_or_else(|| {
            ApiError::new(
                ErrorKind::AgentUnavailable,
                format!(
                    "agent profile {id:?}: agent {:?} is not in Forge's registry, so the profile must set command",
                    profile.agent
                ),
            )
        })?;
        let user = UserConfig::load(&self.paths.user_config())
            .map_err(|e| ApiError::validation(e.to_string()))?;
        let mut read_only = checked_read_only(
            &user.sandbox.read_only,
            &[&self.home, &self.paths.data_dir, &self.paths.runtime_dir],
        )?;
        // The agent must be visible inside the sandbox: its program by its real path (a
        // symlink to it may sit in a hidden directory), and for an npm adapter, node and the
        // adapter's package.
        let manifest = adapters::manifest(&profile.agent);
        let plan = adapters::plan(manifest, &command, std::env::var_os("PATH").as_deref())
            .map_err(|e| {
                let install = manifest
                    .map(|m| format!("; install it with `{}`", m.install_command().join(" ")))
                    .unwrap_or_default();
                ApiError::new(
                    ErrorKind::AgentUnavailable,
                    format!("agent profile {id:?}: {e}{install}"),
                )
            })?;
        read_only.extend(plan.read_only);
        let argv = plan.argv;
        let rules = Arc::new(policy_rules(&self.paths, &root)?);
        let hooks = project_hooks(&root, &self.home)?;
        // Hook scripts come from the project root, shown read-only, so the agent can't change
        // them from its worktree.
        read_only.extend(hooks.dir().map(Path::to_path_buf));
        Ok(AgentLaunch {
            command: AgentCommand { argv, env: vec![] },
            read_only,
            network,
            secrets,
            rules,
            hooks: Arc::new(hooks),
            project_root: root,
        })
    }

    fn remember_in_project(
        &self,
        session: &Session,
        list: RuleListName,
        rule: PolicyRule,
    ) -> Result<(), ApiError> {
        let root = self
            .db
            .project_root(&session.project_id)
            .map_err(|e| ApiError::new(ErrorKind::NotFound, e.to_string()))?
            .ok_or_else(|| ApiError::not_found(format!("project {}", session.project_id)))?;
        forge_config::append_rule(&root.join(PROJECT_POLICY_FILE), list, &rule)
            .map_err(|e| ApiError::validation(e.to_string()))
    }
}

/// The project's hooks file, compiled; read from its root, never a session's worktree.
fn project_hooks(root: &Path, home: &Path) -> Result<HookSet, ApiError> {
    let config = HooksConfig::load(&root.join(PROJECT_HOOKS_FILE))
        .map_err(|e| ApiError::validation(e.to_string()))?;
    HookSet::compile(&config, root, home)
        .map_err(|e| ApiError::validation(format!("{PROJECT_HOOKS_FILE}: {e}")))
}

/// The managed, user, and project policy files, compiled. The project's is read from its root,
/// never a session's worktree, so an agent's edit to it counts only once merged.
fn policy_rules(paths: &Paths, root: &Path) -> Result<Rules, ApiError> {
    let files = [
        (Layer::Managed, paths.managed_policy.clone()),
        (Layer::User, paths.user_policy()),
        (Layer::Project, root.join(PROJECT_POLICY_FILE)),
    ];
    let mut layers = Vec::new();
    for (layer, path) in files {
        let config = PolicyConfig::load(&path)
            .map_err(|e| ApiError::validation(format!("{layer} policy: {e}")))?;
        layers.push((layer, config));
    }
    let layers: Vec<(Layer, &PolicyConfig)> = layers.iter().map(|(l, c)| (*l, c)).collect();
    Rules::compile(&layers).map_err(|e| ApiError::validation(e.to_string()))
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
}
