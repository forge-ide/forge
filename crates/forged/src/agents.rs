//! Agent profiles: the built-in one for each registry agent and a project's own files, and
//! `agent.list`, which says whether each is installed and signed in.

use std::ffi::OsString;
use std::path::Path;

use forge_acp::adapters::{self, Manifest};
use forge_acp::logins::{self, Login};
use forge_api::model::AgentProfile;
use forge_api::{ApiError, ProjectId};
use forge_config::agent::NetworkConfig;
use forge_config::{AGENTS_DIR, AgentProfileConfig, ProfileMode};
use forge_sandbox::proxy::SecretRoute;
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
    let home = state.home.clone();
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
                    Ok(Some(profile)) => status(&id, &profile, store.as_ref(), &home, path.clone()),
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
    home: &Path,
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
    let (authenticated, sign_in) = sign_in(profile, manifest, store, home);
    listed.authenticated = authenticated;
    listed.sign_in = sign_in;
    listed
}

/// How a session signs in: the secrets the proxy injects, and the agent's own login when it
/// stands in for keyring secrets.
pub struct Credentials {
    pub routes: Vec<SecretRoute>,
    pub login: Option<Login>,
}

/// How a session with `profile` signs in: each secret it names from the keyring, or, for a
/// registry agent whose keyring secrets are all missing, its user's own sign-in under `home`
/// (the real home dir). An error lists what to do. Blocking.
pub fn credentials(
    profile: &AgentProfileConfig,
    manifest: Option<&Manifest>,
    store: &dyn SecretStore,
    home: &Path,
) -> Result<Credentials, Vec<String>> {
    let mut fixes = Vec::new();
    let mut routes = Vec::new();
    let mut missing = Vec::new();
    for name in &profile.env_secrets {
        let Some(secret) = secrets::known(name) else {
            fixes.push(format!(
                "Forge can't inject {name:?}; it knows {}",
                secrets::known_names()
            ));
            continue;
        };
        match store.get(name) {
            Ok(value) => routes.push(SecretRoute::fixed(secret, value)),
            Err(SecretError::Missing { fix, .. }) => missing.push((name.as_str(), fix)),
            Err(e) => fixes.push(e.to_string()),
        }
    }
    let login_kind = manifest.and_then(|m| m.auth.login);
    // The login stands in only for the registry's own secrets, and only when none is stored.
    let login_fits = routes.is_empty()
        && fixes.is_empty()
        && manifest.is_some_and(|m| {
            missing
                .iter()
                .all(|(name, _)| m.auth.secrets.iter().any(|s| s == name))
        });
    let mut login = None;
    match login_kind {
        Some(kind) if login_fits && (!missing.is_empty() || profile.env_secrets.is_empty()) => {
            match logins::find(kind, home) {
                Ok(Some(found)) => {
                    routes.push(found.route());
                    login = Some(found);
                    missing.clear();
                }
                Ok(None) => {
                    let keys = missing
                        .drain(..)
                        .map(|(_, fix)| fix)
                        .collect::<Vec<_>>()
                        .join("; ");
                    let sign_in = format!(
                        "sign in with `{}` ({})",
                        kind.sign_in_command(),
                        kind.label()
                    );
                    fixes.push(if keys.is_empty() {
                        sign_in
                    } else {
                        format!("{keys}; or {sign_in}")
                    });
                }
                Err(why) => {
                    missing.clear();
                    fixes.push(why);
                }
            }
        }
        _ => {}
    }
    fixes.extend(missing.into_iter().map(|(_, fix)| fix));
    // An agent that keeps its own login needs no secret; its shim checks the login is there.
    if routes.is_empty()
        && fixes.is_empty()
        && let Some(m) = manifest.filter(|m| m.auth.own_login.is_none())
    {
        fixes.push(format!(
            "add env_secrets = {:?} to the profile; {} signs in with an API key from the keyring",
            m.auth.secrets, m.name
        ));
    }
    if fixes.is_empty() {
        Ok(Credentials { routes, login })
    } else {
        Err(fixes)
    }
}

/// Whether the profile can sign in, and if not, how to.
fn sign_in(
    profile: &AgentProfileConfig,
    manifest: Option<&Manifest>,
    store: &dyn SecretStore,
    home: &Path,
) -> (bool, Option<String>) {
    // An agent that keeps its own login is signed in when it is there.
    if let Some((manifest, login)) = manifest.and_then(|m| Some((m, m.auth.own_login.as_ref()?)))
        && profile.env_secrets.is_empty()
    {
        return match home.join(login).is_file() {
            true => (true, None),
            false => (false, Some(format!("forge agent sign-in {}", manifest.id))),
        };
    }
    match credentials(profile, manifest, store, home) {
        Ok(_) => (true, None),
        Err(fixes) => (false, Some(fixes.join("; "))),
    }
}

/// Where an agent's output during `agent.signIn` goes, under the data dir.
pub fn sign_in_log(data_dir: &Path, agent: &str) -> std::path::PathBuf {
    data_dir.join("sign-in").join(format!("{agent}.log"))
}

/// How long an agent's own sign-in may take: the user finishes it in a browser.
const SIGN_IN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// The registry agent `agent`, as it runs outside the sandbox to sign in: its built-in command
/// (never a profile file's), its manifest's environment, and an empty working directory.
async fn sign_in_command(
    state: &State,
    agent: &str,
) -> Result<(forge_acp::AgentCommand, std::path::PathBuf), HandlerError> {
    let manifest = adapters::manifest(agent)
        .ok_or_else(|| ApiError::not_found(format!("no agent {agent:?} in Forge's registry")))?;
    let plan = adapters::plan(
        Some(manifest),
        &manifest.default_command(),
        std::env::var_os("PATH").as_deref(),
    )
    .map_err(|e| {
        ApiError::new(
            forge_api::ErrorKind::AgentUnavailable,
            format!(
                "{e}; install it with {}",
                manifest.install_command().join(" ")
            ),
        )
    })?;
    let cwd = state.paths.data_dir.join("sign-in");
    tokio::fs::create_dir_all(&cwd)
        .await
        .map_err(|e| HandlerError::Internal(e.into()))?;
    let env = manifest
        .launch
        .env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    Ok((
        forge_acp::AgentCommand {
            argv: plan.argv,
            env,
        },
        cwd,
    ))
}

fn sign_in_failed(agent: &str, e: forge_acp::AcpError) -> HandlerError {
    ApiError::new(
        forge_api::ErrorKind::AgentUnavailable,
        format!("{agent} sign-in: {e}"),
    )
    .into()
}

/// `agent.signInMethods`: the ways the agent offers to sign in, asked of the agent itself.
/// A terminal method comes with the whole command the client runs.
pub async fn sign_in_methods(
    state: &State,
    agent: &str,
) -> Result<Vec<forge_api::model::SignInMethod>, HandlerError> {
    let (command, cwd) = sign_in_command(state, agent).await?;
    let base = command.clone();
    let methods = forge_acp::AcpClient::sign_in_methods(
        command,
        &cwd,
        std::sync::Arc::new(forge_sandbox::Unsandboxed::for_sign_in()),
    )
    .await
    .map_err(|e| sign_in_failed(agent, e))?;
    Ok(methods
        .into_iter()
        .map(|m| forge_api::model::SignInMethod {
            id: m.id,
            name: m.name,
            description: m.description,
            terminal: m.terminal.map(|t| forge_api::model::TerminalSignIn {
                argv: base.argv.iter().cloned().chain(t.args).collect(),
                env: base.env.iter().cloned().chain(t.env).collect(),
            }),
        })
        .collect())
}

/// `agent.signIn`: run one of the agent's own sign-in methods, outside the sandbox with the
/// user's real home, so it keeps the login where it would outside Forge.
pub async fn sign_in_with(state: &State, agent: &str, method: &str) -> Result<(), HandlerError> {
    let (command, cwd) = sign_in_command(state, agent).await?;
    // What the agent prints, for `forge agent sign-in` to show (the URL to open, say).
    let log = sign_in_log(&state.paths.data_dir, agent);
    let _ = tokio::fs::remove_file(&log).await;
    let signed = tokio::time::timeout(
        SIGN_IN_TIMEOUT,
        forge_acp::AcpClient::authenticate(
            command,
            &cwd,
            std::sync::Arc::new(forge_sandbox::Unsandboxed::for_sign_in()),
            method,
            Some(log),
        ),
    )
    .await
    .map_err(|_| {
        ApiError::new(
            forge_api::ErrorKind::AgentUnavailable,
            format!("{agent} sign-in didn't finish within 10 minutes"),
        )
    })?;
    signed.map_err(|e| sign_in_failed(agent, e))
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

    /// A keyring holding only `stored`.
    struct Store(&'static [&'static str]);

    impl SecretStore for Store {
        fn get(&self, name: &str) -> Result<secrets::Secret, SecretError> {
            if self.0.contains(&name) {
                Ok(secrets::Secret::new(format!("key-{name}")))
            } else {
                Err(SecretError::Missing {
                    name: name.to_owned(),
                    fix: format!("store {name}"),
                })
            }
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn claude_login(home: &Path) {
        let later = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
            + 3_600_000;
        let path = home.join(".claude/.credentials.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            path,
            format!(r#"{{"claudeAiOauth":{{"accessToken":"oauth","expiresAt":{later}}}}}"#),
        )
        .unwrap();
    }

    // On macOS the login keychain is read first, and tests never read the real one.
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_stored_key_wins_and_the_login_stands_in_only_for_a_missing_one() {
        let m = adapters::manifest("claude-code").unwrap();
        let profile = builtin(m);
        let home = tempfile::tempdir().unwrap();
        // Neither: both ways to sign in are offered.
        let fixes = credentials(&profile, Some(m), &Store(&[]), home.path())
            .err()
            .unwrap()
            .join("; ");
        assert!(
            fixes.contains("store ANTHROPIC_API_KEY") && fixes.contains("`claude`"),
            "{fixes}"
        );
        claude_login(home.path());
        let with_login = credentials(&profile, Some(m), &Store(&[]), home.path())
            .ok()
            .unwrap();
        assert_eq!(with_login.routes.len(), 1);
        assert_eq!(with_login.routes[0].secret.name, "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(
            with_login.routes[0].value.current().unwrap().expose(),
            "oauth"
        );
        assert!(with_login.login.is_some());
        let with_key = credentials(
            &profile,
            Some(m),
            &Store(&["ANTHROPIC_API_KEY"]),
            home.path(),
        )
        .ok()
        .unwrap();
        assert_eq!(with_key.routes.len(), 1);
        assert_eq!(with_key.routes[0].secret.name, "ANTHROPIC_API_KEY");
        assert!(with_key.login.is_none());
        // A profile naming a secret outside the registry's gets no login in its place.
        let other = AgentProfileConfig {
            env_secrets: vec!["OPENAI_API_KEY".into()],
            ..profile.clone()
        };
        let fixes = credentials(&other, Some(m), &Store(&[]), home.path())
            .err()
            .unwrap();
        assert_eq!(fixes, ["store OPENAI_API_KEY"]);
        // Nor an agent outside the registry.
        assert!(credentials(&profile, None, &Store(&[]), home.path()).is_err());
        // One that keeps its own login needs no secret from the keyring.
        let own = adapters::manifest("antigravity").unwrap();
        let signed = credentials(&builtin(own), Some(own), &Store(&[]), home.path())
            .ok()
            .unwrap();
        assert!(signed.routes.is_empty() && signed.login.is_none());
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
