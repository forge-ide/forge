//! Agent profiles: the built-in one for each registry agent and a project's own files, and
//! `agent.list`, which says whether each is installed and signed in.

use std::ffi::OsString;
use std::path::Path;

use forge_acp::adapters::{self, Manifest};
use forge_api::model::AgentProfile;
use forge_api::{ApiError, ProjectId};
use forge_config::agent::NetworkConfig;
use forge_config::{AGENTS_DIR, AgentProfileConfig, ProfileMode};
use forge_sandbox::secrets::{self, SecretError, SecretStore};

use crate::rpc::{HandlerError, State};

/// The profile a registry agent runs with when the project has no file of the same ID:
/// Accept edits in the Standard tier, no network beyond its own API, signed in with its
/// secrets from the keyring.
pub fn builtin(manifest: &Manifest) -> AgentProfileConfig {
    AgentProfileConfig {
        agent: manifest.id.clone(),
        command: None,
        model: None,
        mode: Some(ProfileMode::AcceptEdits),
        sandbox: None,
        allow_unsandboxed: false,
        env_secrets: manifest.auth.secrets.clone(),
        network: NetworkConfig::default(),
    }
}

/// Profile `id` for the project at `root`: its `.forge/agents/<id>.toml`, else the built-in
/// profile of that ID. `Ok(None)` if neither exists. Blocking.
pub fn load(root: &Path, id: &str) -> Result<Option<AgentProfileConfig>, ApiError> {
    let path =
        AgentProfileConfig::path(root, id).map_err(|e| ApiError::validation(e.to_string()))?;
    match AgentProfileConfig::load(&path) {
        Ok(Some(profile)) => Ok(Some(profile)),
        Ok(None) => Ok(adapters::by_profile(id).map(builtin)),
        Err(e) => Err(ApiError::validation(e.to_string())),
    }
}

/// The command a profile starts its agent with: its own, else its registry agent's.
pub fn command(profile: &AgentProfileConfig) -> Option<Vec<String>> {
    profile
        .command
        .clone()
        .or_else(|| adapters::manifest(&profile.agent).map(Manifest::default_command))
}

pub async fn list(
    state: &State,
    project: Option<ProjectId>,
) -> Result<Vec<AgentProfile>, HandlerError> {
    let root = match project {
        Some(id) => Some(
            state
                .db
                .get_project(id.clone())
                .await?
                .ok_or_else(|| ApiError::not_found(format!("project {id}")))?
                .root,
        ),
        None => None,
    };
    let store = state.secrets.clone();
    let path = std::env::var_os("PATH");
    let listed = tokio::task::spawn_blocking(move || {
        let mut ids: Vec<String> = adapters::manifests()
            .iter()
            .map(|m| m.profile.clone())
            .collect();
        if let Some(root) = &root {
            ids.extend(project_ids(Path::new(root)));
        }
        ids.sort();
        ids.dedup();
        ids.into_iter()
            .map(|id| {
                let loaded = match &root {
                    Some(root) => load(Path::new(root), &id),
                    None => Ok(adapters::by_profile(&id).map(builtin)),
                };
                match loaded {
                    Ok(Some(profile)) => status(&id, &profile, store.as_ref(), path.clone()),
                    Ok(None) => unreachable!("listed profiles exist"),
                    Err(e) => AgentProfile {
                        id,
                        agent: String::new(),
                        installed: false,
                        authenticated: false,
                        version: None,
                        name: None,
                        supported_version: None,
                        install_command: None,
                        sign_in: None,
                        problem: Some(e.detail().to_owned()),
                    },
                }
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(anyhow::Error::from)?;
    Ok(listed)
}

/// The IDs of the profile files under `<root>/.forge/agents/`, skipping names no profile ID
/// can have.
fn project_ids(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root.join(AGENTS_DIR)) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            let id = name.strip_suffix(".toml")?;
            AgentProfileConfig::path(root, id).ok()?;
            Some(id.to_owned())
        })
        .collect()
}

/// Whether profile `id` can start, found without running anything: a profile file can name
/// any command, and nothing runs outside the sandbox on its word.
fn status(
    id: &str,
    profile: &AgentProfileConfig,
    store: &dyn SecretStore,
    path: Option<OsString>,
) -> AgentProfile {
    let manifest = adapters::manifest(&profile.agent);
    let mut listed = AgentProfile {
        id: id.to_owned(),
        agent: profile.agent.clone(),
        installed: false,
        authenticated: false,
        version: None,
        name: manifest.map(|m| m.name.clone()),
        supported_version: manifest.map(|m| m.package.version.clone()),
        install_command: None,
        sign_in: None,
        problem: None,
    };
    let install = manifest.map(|m| m.install_command().join(" "));
    match command(profile) {
        None => {
            listed.problem = Some(format!(
                "agent {:?} is not in Forge's registry, so the profile must set command",
                profile.agent
            ));
        }
        Some(command) => match adapters::plan(manifest, &command, path.as_deref()) {
            Ok(plan) => {
                listed.installed = true;
                let outdated = matches!(
                    (&plan.version, manifest),
                    (Some(v), Some(m)) if *v != m.package.version
                );
                if outdated {
                    listed.install_command = install;
                }
                listed.version = plan.version;
            }
            Err(e) => {
                listed.install_command = install;
                listed.problem = Some(e.to_string());
            }
        },
    }
    let (authenticated, sign_in) = sign_in(profile, manifest, store);
    listed.authenticated = authenticated;
    listed.sign_in = sign_in;
    listed
}

/// Whether every secret the profile signs in with is in the keyring, and if not, how to put
/// it there. A registry agent's profile must name at least one, since the sandbox hides the
/// agent's own sign-in.
fn sign_in(
    profile: &AgentProfileConfig,
    manifest: Option<&Manifest>,
    store: &dyn SecretStore,
) -> (bool, Option<String>) {
    let mut fixes = Vec::new();
    if profile.env_secrets.is_empty()
        && let Some(m) = manifest
    {
        fixes.push(format!(
            "add env_secrets = {:?} to the profile; {} signs in with an API key from the keyring",
            m.auth.secrets, m.name
        ));
    }
    for name in &profile.env_secrets {
        if secrets::known(name).is_none() {
            fixes.push(format!(
                "Forge can't inject {name:?}; it knows {}",
                secrets::known_names()
            ));
            continue;
        }
        match store.get(name) {
            Ok(_) => {}
            Err(SecretError::Missing { fix, .. }) => fixes.push(fix),
            Err(e) => fixes.push(e.to_string()),
        }
    }
    if fixes.is_empty() {
        (true, None)
    } else {
        (false, Some(fixes.join("; ")))
    }
}
