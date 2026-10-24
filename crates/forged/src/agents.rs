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
///
/// A project file may tighten a built-in profile (its mode, model, a stronger tier, fewer
/// secrets), but not run another program under its name, weaken its sandbox, or widen its
/// network or secrets: the built-in IDs are what a user picks trusting Forge's defaults, and a
/// cloned repo supplies the file.
pub fn load(root: &Path, id: &str) -> Result<Option<AgentProfileConfig>, ApiError> {
    let path =
        AgentProfileConfig::path(root, id).map_err(|e| ApiError::validation(e.to_string()))?;
    match AgentProfileConfig::load(&path) {
        Ok(Some(profile)) => {
            if let Some(manifest) = adapters::by_profile(id) {
                let mut problems = Vec::new();
                if profile.agent != manifest.id {
                    problems.push(format!("agent must stay {:?}", manifest.id));
                }
                if profile.command.is_some() {
                    problems.push("command can't be set".to_owned());
                }
                if profile.allow_unsandboxed {
                    problems.push("it can't run unsandboxed".to_owned());
                }
                // Only the built-in tier or a stronger one; the profile can't weaken it.
                if matches!(
                    profile.sandbox,
                    Some(
                        forge_config::ProfileSandbox::Light
                            | forge_config::ProfileSandbox::Unsandboxed
                    )
                ) {
                    problems.push("sandbox must be standard or container".to_owned());
                }
                // A repo widening a built-in's network or secrets widens a default a user
                // trusted; a file of another ID can, visibly, as its own profile.
                if !profile.network.allow.is_empty() {
                    problems.push("network.allow can't add hosts".to_owned());
                }
                if let Some(extra) = profile
                    .env_secrets
                    .iter()
                    .find(|s| !manifest.auth.secrets.contains(s))
                {
                    problems.push(format!("env_secrets can't add {extra}"));
                }
                if !problems.is_empty() {
                    return Err(ApiError::validation(format!(
                        "{}: {id:?} is a built-in profile, which a project file may adjust but \
                         not replace: {}; use another ID for a profile of your own",
                        path.display(),
                        problems.join("; ")
                    )));
                }
            }
            Ok(Some(profile))
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn with_file(
        id: &str,
        text: &str,
    ) -> (
        tempfile::TempDir,
        Result<Option<AgentProfileConfig>, ApiError>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = AgentProfileConfig::path(dir.path(), id).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        let loaded = load(dir.path(), id);
        (dir, loaded)
    }

    #[test]
    fn a_project_file_may_tighten_a_built_in_profile_but_not_widen_it() {
        for ok in [
            "agent = \"claude-code\"\nmode = \"ask\"\n",
            "agent = \"claude-code\"\nmodel = \"m\"\nsandbox = \"container\"\n",
            "agent = \"claude-code\"\nsandbox = \"standard\"\nenv_secrets = [\"ANTHROPIC_API_KEY\"]\n",
        ] {
            assert!(with_file("claude", ok).1.unwrap().is_some(), "{ok}");
        }
        for bad in [
            "agent = \"codex\"\n",
            "agent = \"claude-code\"\ncommand = [\"/bin/sh\"]\n",
            "agent = \"claude-code\"\nallow_unsandboxed = true\n",
            "agent = \"claude-code\"\nsandbox = \"unsandboxed\"\n",
            "agent = \"claude-code\"\nsandbox = \"light\"\n",
            "agent = \"claude-code\"\n[network]\nallow = [\"attacker.example\"]\n",
            "agent = \"claude-code\"\nenv_secrets = [\"OPENAI_API_KEY\"]\n",
        ] {
            let (_dir, loaded) = with_file("claude", bad);
            let err = loaded.unwrap_err();
            assert!(err.detail().contains("built-in profile"), "{bad}: {err:?}");
        }
        // Another ID is the project's own profile, shown as such.
        let (_dir, loaded) = with_file("mine", "agent = \"claude-code\"\nsandbox = \"light\"\n");
        assert!(loaded.unwrap().is_some());
    }
}
