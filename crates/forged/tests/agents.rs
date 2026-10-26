//! `agent.list`: the built-in profiles and a project's own, with install and sign-in status
//! found without running any agent.

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use common::*;
use forge_api::methods::{
    AgentList, AgentListParams, AgentRefParams, AgentSignIn, AgentSignInMethods, AgentSignInParams,
};
use forge_api::model::AgentProfile;

fn executable(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fake global npm install of `package` at `version`, whose `bin` would fail the test if
/// anything ran it.
fn npm_install(prefix: &Path, package: &str, bin: &str, version: &str) {
    let root = prefix.join("lib/node_modules").join(package);
    let marker = prefix.join("ran");
    executable(
        &root.join("index.js"),
        &format!("#!/bin/sh\ntouch {}\n", marker.display()),
    );
    std::fs::write(
        root.join("package.json"),
        format!(r#"{{"name": "{package}", "version": "{version}"}}"#),
    )
    .unwrap();
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::os::unix::fs::symlink(root.join("index.js"), prefix.join("bin").join(bin)).unwrap();
}

fn by_id<'a>(list: &'a [AgentProfile], id: &str) -> &'a AgentProfile {
    list.iter()
        .find(|p| p.id == id)
        .unwrap_or_else(|| panic!("no profile {id} in {list:?}"))
}

#[tokio::test]
async fn lists_built_in_and_project_profiles_with_install_and_sign_in_status() {
    let home = temp_home();
    let prefix = home.path().join("npm");
    // Claude's adapter at the supported version, Codex's at another; Gemini not installed.
    npm_install(
        &prefix,
        "@agentclientprotocol/claude-agent-acp",
        "claude-agent-acp",
        "0.95.0",
    );
    npm_install(
        &prefix,
        "@agentclientprotocol/codex-acp",
        "codex-acp",
        "2.1.0",
    );
    executable(&prefix.join("bin/node"), "#!/bin/sh\nexit 1\n");
    // The keyring knows only the Anthropic key.
    let tool = home.path().join("fake-keyring");
    let missing = if cfg!(target_os = "macos") { 44 } else { 1 };
    executable(
        &tool,
        &format!(
            "#!/bin/sh\ncase \"$*\" in\n  *ANTHROPIC_API_KEY*) echo sk-test ;;\n  *) exit {missing} ;;\nesac\n"
        ),
    );
    // Only the fake installs and the system's programs, never the machine's real agents.
    let path = format!("{}:/usr/bin:/bin", prefix.join("bin").display());
    let daemon = Daemon::start_with(home.path(), |cmd| {
        cmd.env("PATH", &path).env("FORGE_KEYRING_TOOL", &tool);
    });
    let mut client = daemon.client().await;

    let builtins = client
        .call::<AgentList>(&AgentListParams::default())
        .await
        .unwrap();
    let ids: Vec<_> = builtins.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["claude", "codex", "gemini"]);

    let claude = by_id(&builtins, "claude");
    assert_eq!(claude.agent, "claude-code");
    assert!(claude.installed && claude.authenticated, "{claude:?}");
    assert_eq!(claude.version.as_deref(), Some("0.95.0"));
    assert_eq!(claude.supported_version.as_deref(), Some("0.95.0"));
    assert_eq!(claude.install_command, None);
    assert_eq!(claude.sign_in, None);

    let codex = by_id(&builtins, "codex");
    assert!(codex.installed && !codex.authenticated, "{codex:?}");
    assert_eq!(codex.version.as_deref(), Some("2.1.0"));
    // Installed at another version: the pinned install command is still offered.
    assert_eq!(
        codex.install_command.as_deref(),
        Some("npm install -g @agentclientprotocol/codex-acp@2.2.2")
    );
    assert!(
        codex.sign_in.as_deref().unwrap().contains("OPENAI_API_KEY"),
        "{codex:?}"
    );

    let gemini = by_id(&builtins, "gemini");
    assert!(!gemini.installed, "{gemini:?}");
    assert_eq!(gemini.version, None);
    assert_eq!(
        gemini.install_command.as_deref(),
        Some("npm install -g @google/gemini-cli@0.65.0")
    );

    // A project adds its own profiles and overrides a built-in one by ID.
    let repo = git_repo_with_commit(home.path(), "repo");
    let agents = repo.join(".forge/agents");
    std::fs::create_dir_all(&agents).unwrap();
    std::fs::write(
        agents.join("gemini.toml"),
        "agent = \"gemini\"\nmodel = \"gemini-pro\"\nenv_secrets = [\"GEMINI_API_KEY\"]\n",
    )
    .unwrap();
    // A repo can adjust a built-in profile, but not swap its program or leave the sandbox.
    std::fs::write(
        agents.join("claude.toml"),
        "agent = \"claude-code\"\ncommand = [\"/bin/sh\", \"-c\", \"evil\"]\n",
    )
    .unwrap();
    std::fs::write(
        agents.join("codex.toml"),
        "agent = \"codex\"\nsandbox = \"unsandboxed\"\nallow_unsandboxed = true\n",
    )
    .unwrap();
    std::fs::write(agents.join("broken.toml"), "agent = \"x\"\nnope = 1\n").unwrap();
    std::fs::write(agents.join("custom.toml"), "agent = \"my-agent\"\n").unwrap();
    std::fs::write(agents.join("not a profile.toml"), "agent = \"x\"\n").unwrap();
    let project = open(&mut client, &repo).await;
    let listed = client
        .call::<AgentList>(&AgentListParams {
            project_id: Some(project.id.clone()),
        })
        .await
        .unwrap();
    let ids: Vec<_> = listed.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["broken", "claude", "codex", "custom", "gemini"]);

    let gemini = by_id(&listed, "gemini");
    assert!(!gemini.installed && !gemini.authenticated, "{gemini:?}");
    assert!(
        gemini.problem.as_deref().unwrap().contains("not installed"),
        "{gemini:?}"
    );
    for id in ["claude", "codex"] {
        let hijacked = by_id(&listed, id);
        let problem = hijacked.problem.as_deref().unwrap_or_default();
        assert!(problem.contains("built-in profile"), "{hijacked:?}");
    }
    assert!(by_id(&listed, "broken").problem.is_some());
    let custom = by_id(&listed, "custom");
    assert!(!custom.installed, "{custom:?}");
    assert!(
        custom.problem.as_deref().unwrap().contains("my-agent"),
        "{custom:?}"
    );

    // Nothing was run to find any of this out.
    assert!(!prefix.join("ran").exists());

    expect_api_error(
        client
            .call::<AgentList>(&AgentListParams {
                project_id: Some(forge_api::ProjectId::new()),
            })
            .await,
        forge_api::ErrorKind::NotFound,
    );
    daemon.stop();
}

#[tokio::test]
async fn an_agents_own_sign_in_counts_when_the_keyring_has_no_key() {
    let home = temp_home();
    let tool = home.path().join("fake-keyring");
    let missing = if cfg!(target_os = "macos") { 44 } else { 1 };
    executable(&tool, &format!("#!/bin/sh\nexit {missing}\n"));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // A current claude.ai sign-in, and a ChatGPT one whose token has expired.
    let claude = home.path().join(".claude/.credentials.json");
    std::fs::create_dir_all(claude.parent().unwrap()).unwrap();
    std::fs::write(
        &claude,
        format!(
            r#"{{"claudeAiOauth":{{"accessToken":"at","refreshToken":"rt","expiresAt":{}}}}}"#,
            (now + 3600) * 1000
        ),
    )
    .unwrap();
    // `{"exp": 1}`, unsigned.
    let expired = "eyJhbGciOiJub25lIn0.eyJleHAiOjF9.x";
    let codex = home.path().join(".codex/auth.json");
    std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
    std::fs::write(
        &codex,
        format!(
            r#"{{"auth_mode":"chatgpt","tokens":{{"access_token":"{expired}","id_token":"{expired}","refresh_token":"rt","account_id":"a"}}}}"#
        ),
    )
    .unwrap();
    let daemon = Daemon::start_with(home.path(), |cmd| {
        cmd.env("FORGE_KEYRING_TOOL", &tool);
    });
    let mut client = daemon.client().await;
    let listed = client
        .call::<AgentList>(&AgentListParams::default())
        .await
        .unwrap();
    // On macOS the fake keyring has no keychain item either, so the file counts.
    let claude = by_id(&listed, "claude");
    assert!(claude.authenticated, "{claude:?}");
    assert_eq!(claude.sign_in, None);
    let codex = by_id(&listed, "codex");
    assert!(!codex.authenticated, "{codex:?}");
    let fix = codex.sign_in.as_deref().unwrap();
    assert!(
        fix.contains("expired") && fix.contains("`codex login`"),
        "{fix}"
    );
    // Gemini has no sign-in Forge carries: an API key, and nothing about a login.
    let gemini = by_id(&listed, "gemini");
    let fix = gemini.sign_in.as_deref().unwrap();
    assert!(
        fix.contains("GEMINI_API_KEY") && !fix.contains("sign in with"),
        "{fix}"
    );
    daemon.stop();
}

/// Signing in runs the registry agent itself, outside the sandbox with the real home, and asks
/// it how it signs in: its own method signs it in where it keeps logins; a terminal method
/// comes back as the whole command for the CLI to run.
#[tokio::test]
async fn a_registry_agent_lists_its_sign_ins_and_signs_in_with_its_own_method() {
    let home = temp_home();
    let prefix = home.path().join("npm");
    npm_install(&prefix, "@google/gemini-cli", "gemini", "0.65.0");
    let script = home.path().join("auth.yaml");
    std::fs::write(
        &script,
        r#"
auth:
  login: .fake/login
  methods:
    - { id: browser, name: "Log in with a browser" }
    - { id: cli, name: "Log in in a terminal", kind: terminal, args: ["--login"] }
steps:
  - end_turn
"#,
    )
    .unwrap();
    // `node` runs the fake agent in its place, whatever program it is given.
    executable(
        &prefix.join("bin/node"),
        &format!(
            "#!/bin/sh\nexec {} {}\n",
            fake_agent().display(),
            script.display()
        ),
    );
    let path = format!("{}:/usr/bin:/bin", prefix.join("bin").display());
    let daemon = Daemon::start_with(home.path(), |cmd| {
        cmd.env("PATH", &path);
    });
    let mut client = daemon.client().await;
    let gemini = || AgentRefParams {
        agent: "gemini".into(),
    };

    let methods = client.call::<AgentSignInMethods>(&gemini()).await.unwrap();
    let ids: Vec<_> = methods.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(ids, ["browser", "cli"]);
    assert_eq!(methods[0].terminal, None);
    let terminal = methods[1].terminal.as_ref().unwrap();
    // The agent's own launch, as the session would start it, with the method's args.
    assert!(terminal.argv[0].ends_with("/node"), "{terminal:?}");
    assert!(
        terminal.argv[1].ends_with("@google/gemini-cli/index.js"),
        "{terminal:?}"
    );
    assert_eq!(terminal.argv.last().map(String::as_str), Some("--login"));

    // Only registry agents sign in, so never a command from a repo's profile file.
    let unknown = client
        .call::<AgentSignInMethods>(&AgentRefParams {
            agent: "custom".into(),
        })
        .await;
    assert!(unknown.is_err());
    // A terminal method isn't the daemon's to run.
    let terminal = client
        .call::<AgentSignIn>(&AgentSignInParams {
            agent: "gemini".into(),
            method_id: "cli".into(),
        })
        .await;
    assert!(terminal.is_err());
    assert!(!home.path().join(".fake/login").exists());

    client
        .call::<AgentSignIn>(&AgentSignInParams {
            agent: "gemini".into(),
            method_id: "browser".into(),
        })
        .await
        .unwrap();
    // In the real home, where the agent keeps it outside Forge.
    assert!(home.path().join(".fake/login").is_file());
}
