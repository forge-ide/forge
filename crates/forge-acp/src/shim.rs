//! Native shims: what each registry agent needs before it starts so that its own permission
//! system defers to Forge's, its only commit path is Forge's `git_commit`, and it is told so.
//!
//! A shim only ever tightens the agent. It never turns on an agent mode that skips Forge's
//! permission requests, never pre-allows anything but `git_commit` (which Forge's endpoint
//! decides itself), and writes nothing but files in the session's shim dir: a fresh,
//! daemon-owned directory the sandbox shows read-only. It never touches the user's own agent
//! config. Settings that matter go where the agent ranks them above anything a repo or the
//! agent itself could write (its home in the sandbox is writable).

use std::path::Path;

use serde_json::json;

use crate::adapters::Manifest;
use crate::client::{McpEndpoint, SessionSetup};

/// The name agents know Forge's MCP server by.
pub const MCP_SERVER: &str = "forge";

/// What every agent is told about working in Forge.
pub const INSTRUCTIONS: &str = "You are working in a Forge session: a git worktree on its own \
branch, inside a sandbox. Commit with the git_commit tool from the forge MCP server (a message, \
and optionally the paths to commit); running git commit, push, stash, or branch yourself is \
refused. Forge decides each command and edit by its policy and may ask a human or refuse; when \
it refuses, it says why, so adjust instead of retrying the same thing. The repository's own \
instructions, if any, are in AGENTS.md or CLAUDE.md at its root; read them before you start.";

/// What Codex is told besides: its own sandbox can't start inside Forge's, so a command it
/// runs without escalating fails, and an escalated one is what reaches Forge's gate.
const CODEX_INSTRUCTIONS: &str = "Codex's own sandbox can't run here: Forge runs you inside its \
sandbox instead and decides every command. Request escalated permissions \
(sandbox_permissions = \"require_escalated\") for every command you run, with a one-line \
justification; a command run without them fails before it starts.";

/// What agents keep in their home: recreated empty at every launch, since the sandbox home is
/// writable and an earlier command could have left settings there that run programs unasked
/// (Codex's `notify`, Gemini's hooks, Claude's MCP servers).
pub const AGENT_HOME_ENTRIES: &[&str] = &[".claude", ".claude.json", ".codex", ".gemini"];

/// What a shim needs to know about the session.
pub struct ShimContext<'a> {
    /// The session's shim dir: empty, owned by the daemon, shown read-only in the sandbox at
    /// the same path.
    pub dir: &'a Path,
    /// Forge's MCP endpoint, as the agent reaches it.
    pub mcp_url: Option<String>,
    /// Variables as the sandbox sets them for the agent.
    pub var: &'a dyn Fn(&str) -> Option<String>,
}

/// What to start the agent with.
#[derive(Clone, Debug, Default)]
pub struct Shim {
    pub env: Vec<(String, String)>,
    /// Arguments after the manifest's.
    pub args: Vec<String>,
    pub setup: SessionSetup,
}

#[derive(Debug, thiserror::Error)]
#[error("writing the {agent} shim: {source}")]
pub struct ShimError {
    agent: String,
    source: std::io::Error,
}

/// The shim for `manifest`'s agent; for an agent outside the registry, just Forge's MCP
/// endpoint.
pub fn prepare(manifest: Option<&Manifest>, cx: &ShimContext<'_>) -> Result<Shim, ShimError> {
    let mut shim = Shim::default();
    if let Some(url) = &cx.mcp_url {
        shim.setup.mcp_servers.push(McpEndpoint {
            name: MCP_SERVER.into(),
            url: url.clone(),
        });
    }
    let Some(manifest) = manifest else {
        return Ok(shim);
    };
    shim.setup.command_source = manifest.permissions.command;
    shim.env.extend(
        manifest
            .launch
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    let failed = |source| ShimError {
        agent: manifest.id.clone(),
        source,
    };
    match manifest.id.as_str() {
        "claude-code" => claude(&mut shim),
        "codex" => codex(&mut shim, cx),
        "gemini" => gemini(&mut shim, cx).map_err(failed)?,
        _ => {}
    }
    Ok(shim)
}

/// Claude Code: every settings file off (a repo's `.claude/settings.json` could pre-allow
/// tools, and the agent can write its own), its bypass mode refused, its web tools off, only
/// `git_commit` pre-allowed, and the `default` mode, in which every tool that needs
/// permission asks the client.
fn claude(shim: &mut Shim) {
    let meta = json!({
        "claudeCode": {
            "options": {
                "settingSources": [],
                "allowDangerouslySkipPermissions": false,
                "allowedTools": [format!("mcp__{MCP_SERVER}__git_commit")],
                "disallowedTools": ["WebFetch", "WebSearch"],
                // Only the MCP servers Forge passes; none from ~/.claude.json or the repo.
                "strictMcpConfig": true,
                "promptSuggestions": false,
            }
        },
        "systemPrompt": { "append": INSTRUCTIONS },
    });
    shim.setup.meta = meta.as_object().cloned();
    shim.setup.mode = Some("default".into());
}

/// Codex: the read-only mode (see the manifest's quirks), plain `-c` shells, Forge's endpoint
/// with `git_commit` pre-approved, and its web search and phoning home off. `CODEX_CONFIG` is
/// applied as overrides above every config file.
fn codex(shim: &mut Shim, cx: &ShimContext<'_>) {
    let mut config = json!({
        "allow_login_shell": false,
        "features": { "shell_snapshot": false },
        "web_search": "disabled",
        "check_for_update_on_startup": false,
        "analytics": { "enabled": false },
        "feedback": { "enabled": false },
        "history": { "persistence": "none" },
        // A program Codex would run after every turn, outside Forge's gate.
        "notify": [],
        "developer_instructions": format!("{INSTRUCTIONS}\n\n{CODEX_INSTRUCTIONS}"),
    });
    if let Some(url) = (cx.var)("OPENAI_BASE_URL") {
        config["openai_base_url"] = url.into();
    }
    if let Some(url) = &cx.mcp_url {
        config["mcp_servers"] = json!({
            MCP_SERVER: {
                "url": url,
                "tools": { "git_commit": { "approval_mode": "approve" } },
            }
        });
    }
    shim.env.push(("CODEX_CONFIG".into(), config.to_string()));
    shim.setup.mode = Some("read-only".into());
}

/// Gemini CLI: system settings (which rank above the user's and the workspace's) and a policy
/// file passed with `--policy` (which replaces the agent-writable `~/.gemini/policies` and
/// ranks above the workspace's). The policy asks for every shell command, edit, write, and MCP
/// tool at the top of its tier, above the settings-derived allows, and allows only Forge's
/// `git_commit`.
fn gemini(shim: &mut Shim, cx: &ShimContext<'_>) -> std::io::Result<()> {
    let settings = json!({
        "security": {
            "auth": { "selectedType": "gemini-api-key" },
            "disableAlwaysAllow": true,
        },
        "general": { "enableAutoUpdate": false, "defaultApprovalMode": "default" },
        "privacy": { "usageStatisticsEnabled": false },
        // The workspace is trusted (Gemini starts no MCP server in an untrusted folder, Forge's
        // included), so the repo's .gemini/settings.json is merged in. Everything there that
        // runs a command without asking is switched off here, in the tier merged last: tool
        // discovery and calls, hooks, extensions, and MCP servers other than Forge's.
        "tools": {
            "exclude": ["web_fetch", "google_web_search"],
            "discoveryCommand": "",
            "callCommand": "",
        },
        "hooksConfig": { "enabled": false },
        "admin": { "extensions": { "enabled": false } },
        "mcp": { "allowed": [MCP_SERVER] },
        "advanced": { "ignoreLocalEnv": true },
        "context": { "fileName": ["GEMINI.md", "AGENTS.md"] },
    });
    let settings_path = cx.dir.join("gemini-settings.json");
    write_new(&settings_path, &serde_json::to_vec_pretty(&settings)?)?;
    let policies = cx.dir.join("gemini-policies");
    std::fs::create_dir(&policies)?;
    write_new(&policies.join("forge.toml"), GEMINI_POLICY.as_bytes())?;
    shim.env.push((
        "GEMINI_CLI_SYSTEM_SETTINGS_PATH".into(),
        settings_path.display().to_string(),
    ));
    shim.args
        .extend(["--policy".into(), policies.display().to_string()]);
    shim.setup.mode = Some("default".into());
    // The system prompt can only be replaced, not added to; Forge's instructions go in with
    // the first prompt instead.
    Ok(())
}

const GEMINI_POLICY: &str = r#"# Written by Forge for one session. Forge decides every action; Gemini asks it.

[[rule]]
mcpName = "forge"
toolName = "git_commit"
decision = "allow"
priority = 999

[[rule]]
toolName = "run_shell_command"
argsPattern = ".*"
decision = "ask_user"
priority = 999

[[rule]]
toolName = ["replace", "write_file"]
decision = "ask_user"
priority = 999

[[rule]]
toolName = ["web_fetch", "google_web_search"]
decision = "deny"
priority = 999

[[rule]]
mcpName = "*"
toolName = "*"
decision = "ask_user"
priority = 998
"#;

/// Write a file that must not exist yet, so nothing already there is followed or replaced.
fn write_new(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(contents)
}

/// The text the first prompt starts with, for agents whose instructions can't be added to
/// any other way.
pub fn first_prompt_preamble(manifest: Option<&Manifest>) -> Option<&'static str> {
    manifest.filter(|m| m.id == "gemini").map(|_| INSTRUCTIONS)
}

/// The tools a Claude shim pre-allows.
#[cfg(test)]
fn allowed_tools(meta: &serde_json::Value) -> Vec<String> {
    meta["claudeCode"]["options"]["allowedTools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{CommandSource, manifest};
    use serde_json::Value;

    fn context<'a>(dir: &'a Path, var: &'a dyn Fn(&str) -> Option<String>) -> ShimContext<'a> {
        ShimContext {
            dir,
            mcp_url: Some("http://127.0.0.1:9/_forge/tok/mcp".into()),
            var,
        }
    }

    fn no_vars(_: &str) -> Option<String> {
        None
    }

    /// Every file under `dir`, relative to it.
    fn files(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.strip_prefix(dir).unwrap().display().to_string();
            if path.is_dir() {
                out.extend(files(&path).into_iter().map(|f| format!("{name}/{f}")));
            } else {
                out.push(name);
            }
        }
        out.sort();
        out
    }

    #[test]
    fn claude_turns_off_settings_files_and_bypass_and_allows_only_git_commit() {
        let dir = tempfile::tempdir().unwrap();
        let shim = prepare(manifest("claude-code"), &context(dir.path(), &no_vars)).unwrap();
        let meta = Value::Object(shim.setup.meta.clone().unwrap());
        let options = &meta["claudeCode"]["options"];
        assert_eq!(options["settingSources"], json!([]));
        assert_eq!(options["allowDangerouslySkipPermissions"], false);
        assert_eq!(allowed_tools(&meta), ["mcp__forge__git_commit"]);
        assert_eq!(options["disallowedTools"], json!(["WebFetch", "WebSearch"]));
        assert_eq!(options["strictMcpConfig"], true);
        assert!(options.get("permissionMode").is_none());
        assert!(
            meta["systemPrompt"]["append"]
                .as_str()
                .unwrap()
                .contains("git_commit")
        );
        assert_eq!(shim.setup.mode.as_deref(), Some("default"));
        assert_eq!(shim.setup.command_source, CommandSource::RawInput);
        assert_eq!(shim.setup.mcp_servers[0].name, "forge");
        assert!(shim.args.is_empty());
        assert!(
            files(dir.path()).is_empty(),
            "Claude's shim writes no files"
        );
    }

    #[test]
    fn codex_runs_read_only_without_login_shells_and_approves_only_git_commit() {
        let dir = tempfile::tempdir().unwrap();
        let var = |name: &str| {
            (name == "OPENAI_BASE_URL").then(|| "http://127.0.0.1:9/_forge/tok/openai/v1".into())
        };
        let shim = prepare(manifest("codex"), &context(dir.path(), &var)).unwrap();
        let env: std::collections::HashMap<_, _> = shim.env.iter().cloned().collect();
        assert_eq!(env["INITIAL_AGENT_MODE"], "read-only");
        let config: Value = serde_json::from_str(&env["CODEX_CONFIG"]).unwrap();
        assert_eq!(config["allow_login_shell"], false);
        assert_eq!(config["features"]["shell_snapshot"], false);
        assert_eq!(config["web_search"], "disabled");
        assert_eq!(config["notify"], json!([]));
        assert_eq!(
            config["openai_base_url"],
            "http://127.0.0.1:9/_forge/tok/openai/v1"
        );
        let forge = &config["mcp_servers"]["forge"];
        assert_eq!(forge["url"], "http://127.0.0.1:9/_forge/tok/mcp");
        assert_eq!(
            forge["tools"],
            json!({ "git_commit": { "approval_mode": "approve" } })
        );
        assert_eq!(config["mcp_servers"].as_object().unwrap().len(), 1);
        // Nothing that would let Codex skip asking.
        for key in [
            "approval_policy",
            "sandbox_mode",
            "default_tools_approval_mode",
        ] {
            assert!(config.get(key).is_none(), "{key}");
        }
        let told = config["developer_instructions"].as_str().unwrap();
        assert!(
            told.contains("git_commit") && told.contains("require_escalated"),
            "{told}"
        );
        assert_eq!(shim.setup.mode.as_deref(), Some("read-only"));
        assert_eq!(shim.setup.command_source, CommandSource::ToolCallArgv);
        assert!(files(dir.path()).is_empty(), "Codex's shim writes no files");
    }

    #[test]
    fn gemini_puts_its_settings_and_policy_above_the_workspace_and_asks_for_everything() {
        let dir = tempfile::tempdir().unwrap();
        let shim = prepare(manifest("gemini"), &context(dir.path(), &no_vars)).unwrap();
        assert_eq!(
            files(dir.path()),
            ["gemini-policies/forge.toml", "gemini-settings.json"]
        );
        let env: std::collections::HashMap<_, _> = shim.env.iter().cloned().collect();
        let settings_path = dir.path().join("gemini-settings.json");
        assert_eq!(
            env["GEMINI_CLI_SYSTEM_SETTINGS_PATH"],
            settings_path.display().to_string()
        );
        assert_eq!(env["GEMINI_CLI_TRUST_WORKSPACE"], "true");
        assert_eq!(
            shim.args,
            [
                "--policy",
                &dir.path().join("gemini-policies").display().to_string()
            ]
        );
        let settings: Value =
            serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
        assert_eq!(settings["security"]["disableAlwaysAllow"], true);
        assert_eq!(
            settings["security"]["auth"]["selectedType"],
            "gemini-api-key"
        );
        assert!(settings.get("mcpServers").is_none());
        assert!(settings["tools"].get("allowed").is_none());
        // What a trusted repo's settings could otherwise use to run commands unasked.
        assert_eq!(settings["tools"]["discoveryCommand"], "");
        assert_eq!(settings["tools"]["callCommand"], "");
        assert_eq!(settings["hooksConfig"]["enabled"], false);
        assert_eq!(settings["admin"]["extensions"]["enabled"], false);
        assert_eq!(settings["mcp"]["allowed"], json!(["forge"]));
        assert_eq!(settings["advanced"]["ignoreLocalEnv"], true);
        // The policy parses, allows only git_commit, and asks for every shell command.
        let policy: toml::Value = toml::from_str(GEMINI_POLICY).unwrap();
        let rules = policy["rule"].as_array().unwrap();
        let allows: Vec<_> = rules
            .iter()
            .filter(|r| r["decision"].as_str() == Some("allow"))
            .collect();
        assert_eq!(allows.len(), 1);
        assert_eq!(allows[0]["toolName"].as_str(), Some("git_commit"));
        assert_eq!(allows[0]["mcpName"].as_str(), Some("forge"));
        assert!(
            rules
                .iter()
                .any(|r| r["toolName"].as_str() == Some("run_shell_command")
                    && r["argsPattern"].as_str() == Some(".*")
                    && r["decision"].as_str() == Some("ask_user"))
        );
        assert_eq!(shim.setup.command_source, CommandSource::Title);
        assert_eq!(shim.setup.mode.as_deref(), Some("default"));
        assert!(first_prompt_preamble(manifest("gemini")).is_some());
        assert!(first_prompt_preamble(manifest("claude-code")).is_none());
        // A second launch into the same dir would find the files there: refused, not followed.
        assert!(prepare(manifest("gemini"), &context(dir.path(), &no_vars)).is_err());
    }

    #[test]
    fn agents_outside_the_registry_get_only_forges_endpoint() {
        let dir = tempfile::tempdir().unwrap();
        let shim = prepare(None, &context(dir.path(), &no_vars)).unwrap();
        assert!(shim.env.is_empty() && shim.args.is_empty());
        assert!(shim.setup.meta.is_none() && shim.setup.mode.is_none());
        assert_eq!(shim.setup.mcp_servers.len(), 1);
        let none = prepare(
            None,
            &ShimContext {
                dir: dir.path(),
                mcp_url: None,
                var: &no_vars,
            },
        )
        .unwrap();
        assert!(none.setup.mcp_servers.is_empty());
    }
}
