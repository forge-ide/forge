//! Session methods: agent profiles, sandbox tiers, and branch names, in front of the manager.

use std::path::{Path, PathBuf};

use forge_acp::AgentCommand;
use forge_api::methods::{BranchPreview, BranchPreviewParams, SessionCreateParams, SessionOutcome};
use forge_api::model::{Project, SandboxTier, Session};
use forge_api::{ApiError, ErrorKind, ProjectId};
use forge_config::{
    AgentProfileConfig, BranchRules, ConfigError, ProfileSandbox, ProjectConfig, UserConfig,
};
use forge_session::{AgentResolver, CreateRequest, SessionError};

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
    let sandbox = sandbox_tier(params.sandbox, &profile, &params.agent_profile)?;
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
) -> Result<SandboxTier, ApiError> {
    let tier = requested.unwrap_or(match profile.sandbox.unwrap_or_default() {
        ProfileSandbox::Light => SandboxTier::Light,
        ProfileSandbox::Standard => SandboxTier::Standard,
        ProfileSandbox::Container => SandboxTier::Container,
        ProfileSandbox::Unsandboxed => SandboxTier::Unsandboxed,
    });
    match tier {
        SandboxTier::Unsandboxed if profile.allow_unsandboxed => Ok(tier),
        SandboxTier::Unsandboxed => Err(ApiError::new(
            ErrorKind::SandboxUnavailable,
            format!(
                "agent profile {id:?} does not allow running unsandboxed; set allow_unsandboxed = true in it to opt in"
            ),
        )),
        other => Err(ApiError::new(
            ErrorKind::SandboxUnavailable,
            format!(
                "the {} sandbox tier is not available on this machine yet; server.hello lists the tiers that are",
                serde_json::to_value(other)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default()
            ),
        )),
    }
}

/// Finds a session's agent command from its profile, rechecking that its tier is allowed.
pub struct ProfileResolver {
    pub db: Db,
}

impl AgentResolver for ProfileResolver {
    fn command(&self, session: &Session) -> Result<AgentCommand, ApiError> {
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
        sandbox_tier(Some(session.sandbox), &profile, id)?;
        let argv = profile.command.ok_or_else(|| {
            ApiError::new(
                ErrorKind::AgentUnavailable,
                format!("agent profile {id:?} has no command"),
            )
        })?;
        Ok(AgentCommand { argv, env: vec![] })
    }
}
