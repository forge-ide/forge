//! Native shims: what each registry agent needs before it starts so that its own permission
//! system defers to Forge's, its only commit path is Forge's `git_commit`, and it is told so.
//!
//! A shim never turns on an agent mode that skips Forge's permission requests, and pre-allows
//! nothing but `git_commit` (which Forge's endpoint decides itself). The agent reads its user's
//! own settings and the repo's, as it would outside Forge: the user's from copies Forge makes
//! at each launch (see [`home_config`]), never the real files, which stay hidden with the rest
//! of the real home. Settings Forge must win over go where the agent ranks them above anything
//! the user, a repo, or the agent itself could write. The shim's own files go in the session's
//! shim dir, a fresh, daemon-owned directory the sandbox shows read-only.

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::adapters::Manifest;
use crate::client::{McpEndpoint, SessionSetup};
use crate::logins::{Login, LoginKind, unsigned_jwt};

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

/// What agents keep in their home: recreated at every launch from the user's own config (see
/// [`install_home`]), since the sandbox home is writable and an earlier command could have
/// left settings there that run programs unasked (Codex's `notify`, Gemini's hooks, Claude's
/// MCP servers).
pub const AGENT_HOME_ENTRIES: &[&str] = &[".claude", ".claude.json", ".codex", ".gemini", ".env"];

/// What a shim needs to know about the session.
pub struct ShimContext<'a> {
    /// The session's shim dir: empty, owned by the daemon, shown read-only in the sandbox at
    /// the same path.
    pub dir: &'a Path,
    /// Forge's MCP endpoint, as the agent reaches it.
    pub mcp_url: Option<String>,
    /// Variables as the sandbox sets them for the agent.
    pub var: &'a dyn Fn(&str) -> Option<String>,
    /// What the agent reads from the user's own config.
    pub home: &'a HomeConfig,
    /// The sign-in the proxy carries for the agent, when it isn't an API key.
    pub login: Option<LoginKind>,
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
pub enum ShimError {
    #[error("writing the {agent} shim: {source}")]
    Write {
        agent: String,
        source: std::io::Error,
    },
    #[error("reading the user's {agent} config: {why}")]
    Read { agent: String, why: String },
}

/// Variables Gemini reads that a repo's `.env` could otherwise set (dotenv never replaces one
/// already set): where it reads settings, prompts, and credentials from, and what it runs
/// outside the sandbox's view of things. The model is left to the user's settings.
const GEMINI_PINNED_ENV: &[&str] = &[
    "GEMINI_CLI_HOME",
    "GEMINI_CLI_SYSTEM_SETTINGS_PATH",
    "GEMINI_CLI_SYSTEM_DEFAULTS_PATH",
    "GEMINI_CLI_TRUSTED_FOLDERS_PATH",
    "GEMINI_SYSTEM_MD",
    "GEMINI_WRITE_SYSTEM_MD",
    "GEMINI_SANDBOX",
    "GEMINI_SANDBOX_PROXY_COMMAND",
    "GEMINI_CLI_IDE_SERVER_STDIO_COMMAND",
    "GEMINI_CLI_IDE_SERVER_STDIO_ARGS",
    "GEMINI_CLI_EXTENSION_REGISTRY_URI",
    "GEMINI_CLI_AUTH_OVERRIDE",
    "GEMINI_CLI_CUSTOM_HEADERS",
    "GEMINI_API_KEY_AUTH_MECHANISM",
    "GOOGLE_APPLICATION_CREDENTIALS",
    "GOOGLE_CLOUD_PROJECT",
    "NODE_OPTIONS",
];

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
    let failed = |source| ShimError::Write {
        agent: manifest.id.clone(),
        source,
    };
    match manifest.id.as_str() {
        "claude-code" => claude(&mut shim, cx).map_err(failed)?,
        "codex" => codex(&mut shim, cx),
        "gemini" => gemini(&mut shim, cx).map_err(failed)?,
        _ => {}
    }
    Ok(shim)
}

/// Claude Code: the repo's settings files and the user's (from a read-only copy in the shim
/// dir, so neither the agent nor a command can add rules to it mid-session; the agent-writable
/// user settings in the sandbox home are not read), its bypass mode refused, its web tools off,
/// only `git_commit` pre-allowed, and the `default` mode, in which every tool the settings don't
/// allow asks the client.
fn claude(shim: &mut Shim, cx: &ShimContext<'_>) -> std::io::Result<()> {
    let mut options = json!({
        "settingSources": ["project", "local"],
        "allowDangerouslySkipPermissions": false,
        "allowedTools": [format!("mcp__{MCP_SERVER}__git_commit")],
        "disallowedTools": ["WebFetch", "WebSearch"],
        "promptSuggestions": false,
    });
    if let Some(settings) = &cx.home.claude_settings {
        let path = cx.dir.join("claude-settings.json");
        write_new(&path, settings)?;
        options["settings"] = path.display().to_string().into();
    }
    let meta = json!({
        "claudeCode": { "options": options },
        "systemPrompt": { "append": INSTRUCTIONS },
    });
    shim.setup.meta = meta.as_object().cloned();
    shim.setup.mode = Some("default".into());
    Ok(())
}

/// Codex: the read-only mode (see the manifest's quirks), plain `-c` shells, escalations
/// decided by the user (through Forge) rather than Codex's own reviewer, Forge's endpoint with
/// `git_commit` pre-approved, and its web search and phoning home off. `CODEX_CONFIG` is
/// applied as overrides above every config file, the user's and the repo's.
fn codex(shim: &mut Shim, cx: &ShimContext<'_>) {
    let mut config = json!({
        "allow_login_shell": false,
        "features": { "shell_snapshot": false },
        // `auto_review` would have a model approve escalated commands, which then never
        // reach Forge.
        "approvals_reviewer": "user",
        "web_search": "disabled",
        "check_for_update_on_startup": false,
        "analytics": { "enabled": false },
        "feedback": { "enabled": false },
        "developer_instructions": format!("{INSTRUCTIONS}\n\n{CODEX_INSTRUCTIONS}"),
    });
    if let Some(url) = (cx.var)("OPENAI_BASE_URL") {
        config["openai_base_url"] = url.into();
    }
    if cx.login == Some(LoginKind::Chatgpt) {
        if let Some(url) = (cx.var)(LoginKind::Chatgpt.secret().base_url_var) {
            config["chatgpt_base_url"] = url.clone().into();
            // The built-in provider routes ChatGPT workspaces only to HTTPS origins; the
            // proxy's route is plain HTTP over loopback.
            config["model_provider"] = CODEX_LOGIN_PROVIDER.into();
            config["model_providers"] = json!({
                CODEX_LOGIN_PROVIDER: {
                    "name": "ChatGPT through Forge",
                    "base_url": format!("{url}/codex"),
                    "wire_api": "responses",
                    "requires_openai_auth": true,
                }
            });
        }
        // The auth.json the home holds is the ChatGPT sign-in, with placeholders.
        set_env(shim, "DEFAULT_AUTH_REQUEST", r#"{"methodId":"chat-gpt"}"#);
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

/// Gemini CLI: a policy file passed with `--policy`, which replaces `~/.gemini/policies` (the
/// user's own policies included) and ranks above the workspace's. It asks for every shell
/// command, edit, write, and MCP tool at the top of its tier, above every settings-derived
/// allow, the user's and a repo's, and allows only Forge's `git_commit`. Gemini reads its
/// user settings from the copy in the sandbox home, and a trusted repo's `.gemini/settings.json`
/// and `.env` as it would outside Forge.
fn gemini(shim: &mut Shim, cx: &ShimContext<'_>) -> std::io::Result<()> {
    let policies = cx.dir.join("gemini-policies");
    std::fs::create_dir(&policies)?;
    write_new(&policies.join("forge.toml"), GEMINI_POLICY.as_bytes())?;
    shim.args
        .extend(["--policy".into(), policies.display().to_string()]);
    // Set, empty, so a repo's .env can't set them.
    shim.env.extend(
        GEMINI_PINNED_ENV
            .iter()
            .map(|name| ((*name).to_owned(), String::new())),
    );
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

/// What an agent reads from its user's own config, gathered from the real home before the
/// sandbox starts. Credentials are never part of it: a login reaches the agent only as
/// placeholders (see [`crate::logins`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HomeConfig {
    /// Files written into the sandbox home each launch, relative to it: copies of the user's
    /// settings, and a login's placeholders.
    pub files: Vec<(PathBuf, Vec<u8>)>,
    /// Links in the sandbox home, relative to it, to the user's own instructions, skills,
    /// commands, and plugins, by their real paths, which the sandbox shows read-only (see
    /// [`HomeConfig::read_only`]).
    pub links: Vec<(PathBuf, PathBuf)>,
    /// Claude Code's user settings, which its shim passes from the read-only shim dir.
    pub claude_settings: Option<Vec<u8>>,
}

impl HomeConfig {
    /// The real paths the links point at, which the sandbox must show read-only.
    pub fn read_only(&self) -> Vec<PathBuf> {
        self.links.iter().map(|(_, real)| real.clone()).collect()
    }
}

/// The largest settings file Forge copies.
const MAX_SETTINGS: u64 = 1024 * 1024;

/// Entries of each agent's config dir that are the user's own instructions and extensions,
/// shown read-only by their real path. Never the dir itself, which holds credentials, history,
/// and session transcripts.
fn shared_entries(agent: &str) -> (&'static str, &'static [&'static str]) {
    match agent {
        "claude-code" => (
            ".claude",
            &[
                "CLAUDE.md",
                "agents",
                "commands",
                "hooks",
                "output-styles",
                "plugins",
                "rules",
                "skills",
            ],
        ),
        "codex" => (
            ".codex",
            &[
                "AGENTS.md",
                "AGENTS.override.md",
                "plugins",
                "prompts",
                "rules",
                "skills",
            ],
        ),
        "gemini" => (
            ".gemini",
            &["GEMINI.md", "agents", "commands", "extensions", "skills"],
        ),
        _ => ("", &[]),
    }
}

/// What `manifest`'s agent reads from the user's config under `home` (the real home dir), and
/// for a login, the placeholders it signs in with. Blocking: reads the user's files.
pub fn home_config(
    manifest: Option<&Manifest>,
    home: &Path,
    login: Option<&Login>,
) -> Result<HomeConfig, ShimError> {
    let mut out = HomeConfig::default();
    let Some(manifest) = manifest else {
        return Ok(out);
    };
    let failed = |why: String| ShimError::Read {
        agent: manifest.name.clone(),
        why,
    };
    let (dir, entries) = shared_entries(&manifest.id);
    for entry in entries {
        let rel = Path::new(dir).join(entry);
        // By the resolved path, which is where the sandbox shows it.
        if let Ok(real) = std::fs::canonicalize(home.join(&rel)) {
            out.links.push((rel, real));
        }
    }
    match manifest.id.as_str() {
        "claude-code" => {
            out.claude_settings =
                read_settings(&home.join(".claude/settings.json")).map_err(failed)?;
            // User-scope MCP servers live in ~/.claude.json, beside state Forge leaves behind.
            if let Some(raw) = read_settings(&home.join(".claude.json")).map_err(failed)? {
                let state: serde_json::Value = serde_json::from_slice(&raw)
                    .map_err(|e| failed(format!("~/.claude.json: {e}")))?;
                if let Some(servers) = state.get("mcpServers").filter(|s| s.is_object()) {
                    let kept = json!({ "mcpServers": servers });
                    out.files
                        .push((".claude.json".into(), kept.to_string().into_bytes()));
                }
            }
        }
        "codex" => {
            if let Some(raw) = read_settings(&home.join(".codex/config.toml")).map_err(failed)? {
                out.files.push((".codex/config.toml".into(), raw));
            }
            if let Some(account) = login.and_then(|l| l.account.as_ref()) {
                out.files
                    .push((".codex/auth.json".into(), codex_auth(account).into_bytes()));
            }
        }
        "gemini" => {
            if let Some(raw) = read_settings(&home.join(".gemini/settings.json")).map_err(failed)? {
                out.files.push((".gemini/settings.json".into(), raw));
            }
        }
        _ => {}
    }
    Ok(out)
}

/// Codex's `auth.json` for a ChatGPT sign-in, holding placeholders for its tokens: unsigned
/// tokens with the real ones' claims, which Codex reads the plan and account from, set to
/// expire far off so Codex never tries to refresh them itself.
fn codex_auth(account: &crate::logins::ChatgptAccount) -> String {
    const TEN_YEARS: u64 = 10 * 365 * 24 * 3600;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let far = |claims: &serde_json::Value| {
        let mut claims = claims.clone();
        claims["exp"] = (now.as_secs() + TEN_YEARS).into();
        unsigned_jwt(&claims)
    };
    let last_refresh = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": null,
        "tokens": {
            "id_token": far(&account.id_claims),
            "access_token": far(&account.access_claims),
            "refresh_token": forge_sandbox::secrets::PLACEHOLDER,
            "account_id": account.account_id,
        },
        "last_refresh": last_refresh,
    })
    .to_string()
}

/// A settings file's contents, if it exists; refused if it isn't a regular file under 1 MiB.
fn read_settings(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    let meta = file
        .metadata()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > MAX_SETTINGS {
        return Err(format!(
            "{} is not a regular file under 1 MiB",
            path.display()
        ));
    }
    let mut out = Vec::new();
    file.take(MAX_SETTINGS)
        .read_to_end(&mut out)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Some(out))
}

/// Put `config` into the sandbox home `home`, whose agent entries ([`AGENT_HOME_ENTRIES`]) the
/// caller has just removed: nothing already there is followed or replaced. `seen_as` is the
/// home's path inside the sandbox: where that puts a link at its own target (the Standard
/// tier shows the scratch home at the real home's path), the read-only mount of the target is
/// what the agent sees there, and no link is made.
pub fn install_home(config: &HomeConfig, home: &Path, seen_as: &Path) -> std::io::Result<()> {
    for (rel, contents) in &config.files {
        let path = home.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_new(&path, contents)?;
    }
    for (rel, real) in &config.links {
        if seen_as.join(rel) == *real {
            continue;
        }
        let path = home.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::os::unix::fs::symlink(real, &path)?;
    }
    Ok(())
}

/// The model provider Codex reaches its ChatGPT sign-in's backend through.
const CODEX_LOGIN_PROVIDER: &str = "forge-chatgpt";

/// Set `name`, replacing what the manifest set.
fn set_env(shim: &mut Shim, name: &str, value: &str) {
    shim.env.retain(|(k, _)| k != name);
    shim.env.push((name.to_owned(), value.to_owned()));
}

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

    static NO_HOME: HomeConfig = HomeConfig {
        files: Vec::new(),
        links: Vec::new(),
        claude_settings: None,
    };

    fn context<'a>(dir: &'a Path, var: &'a dyn Fn(&str) -> Option<String>) -> ShimContext<'a> {
        ShimContext {
            dir,
            mcp_url: Some("http://127.0.0.1:9/_forge/tok/mcp".into()),
            var,
            home: &NO_HOME,
            login: None,
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
    fn claude_reads_the_repos_settings_and_a_fixed_copy_of_the_users_but_no_bypass() {
        let dir = tempfile::tempdir().unwrap();
        let shim = prepare(manifest("claude-code"), &context(dir.path(), &no_vars)).unwrap();
        let meta = Value::Object(shim.setup.meta.clone().unwrap());
        let options = &meta["claudeCode"]["options"];
        // Not "user": the sandbox home's settings file is the agent's to write.
        assert_eq!(options["settingSources"], json!(["project", "local"]));
        assert_eq!(options["allowDangerouslySkipPermissions"], false);
        assert_eq!(allowed_tools(&meta), ["mcp__forge__git_commit"]);
        assert_eq!(options["disallowedTools"], json!(["WebFetch", "WebSearch"]));
        assert!(options.get("strictMcpConfig").is_none());
        assert!(
            options.get("settings").is_none(),
            "the user has no settings"
        );
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
        assert!(files(dir.path()).is_empty());
        // The user's settings go in from the read-only shim dir.
        let home = HomeConfig {
            claude_settings: Some(br#"{"model":"opus"}"#.to_vec()),
            ..HomeConfig::default()
        };
        let dir = tempfile::tempdir().unwrap();
        let cx = ShimContext {
            home: &home,
            ..context(dir.path(), &no_vars)
        };
        let shim = prepare(manifest("claude-code"), &cx).unwrap();
        let meta = Value::Object(shim.setup.meta.unwrap());
        let path = dir.path().join("claude-settings.json");
        assert_eq!(
            meta["claudeCode"]["options"]["settings"],
            path.display().to_string()
        );
        assert_eq!(std::fs::read(&path).unwrap(), br#"{"model":"opus"}"#);
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
        assert_eq!(config["approvals_reviewer"], "user");
        // The user's own notify program, if any, runs as it would outside Forge.
        assert!(config.get("notify").is_none());
        assert!(config.get("chatgpt_base_url").is_none());
        assert_eq!(env["DEFAULT_AUTH_REQUEST"], r#"{"methodId":"api-key"}"#);
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
    fn codex_with_a_chatgpt_login_uses_its_route_and_signs_in_with_chatgpt() {
        let dir = tempfile::tempdir().unwrap();
        let var = |name: &str| {
            (name == "CODEX_CHATGPT_BASE_URL")
                .then(|| "http://127.0.0.1:9/_forge/tok/chatgpt/backend-api".into())
        };
        let cx = ShimContext {
            login: Some(LoginKind::Chatgpt),
            ..context(dir.path(), &var)
        };
        let shim = prepare(manifest("codex"), &cx).unwrap();
        let config: Value = serde_json::from_str(&shim_env(&shim, "CODEX_CONFIG")).unwrap();
        assert_eq!(
            config["chatgpt_base_url"],
            "http://127.0.0.1:9/_forge/tok/chatgpt/backend-api"
        );
        assert!(config.get("openai_base_url").is_none());
        // Through a provider of Forge's: the built-in one routes ChatGPT workspaces only to
        // HTTPS origins.
        assert_eq!(config["model_provider"], CODEX_LOGIN_PROVIDER);
        let provider = &config["model_providers"][CODEX_LOGIN_PROVIDER];
        assert_eq!(
            provider["base_url"],
            "http://127.0.0.1:9/_forge/tok/chatgpt/backend-api/codex"
        );
        assert_eq!(provider["requires_openai_auth"], true);
        assert_eq!(provider["wire_api"], "responses");
        assert_eq!(
            shim_env(&shim, "DEFAULT_AUTH_REQUEST"),
            r#"{"methodId":"chat-gpt"}"#
        );
        assert_eq!(
            shim.env
                .iter()
                .filter(|(k, _)| k == "DEFAULT_AUTH_REQUEST")
                .count(),
            1
        );
    }

    fn shim_env(shim: &Shim, name: &str) -> String {
        shim.env
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .unwrap()
    }

    #[test]
    fn gemini_gets_a_policy_above_every_settings_allow() {
        let dir = tempfile::tempdir().unwrap();
        let shim = prepare(manifest("gemini"), &context(dir.path(), &no_vars)).unwrap();
        assert_eq!(files(dir.path()), ["gemini-policies/forge.toml"]);
        let env: std::collections::HashMap<_, _> = shim.env.iter().cloned().collect();
        assert_eq!(env["GEMINI_CLI_TRUST_WORKSPACE"], "true");
        // Pinned empty, so a repo's .env can't point them anywhere.
        for name in GEMINI_PINNED_ENV {
            assert_eq!(env[*name], "", "{name}");
        }
        assert_eq!(
            shim.args,
            [
                "--policy",
                &dir.path().join("gemini-policies").display().to_string()
            ]
        );
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
        // Every rule sits at the top of the user tier, above a workspace policy's (tier 3) and
        // settings-derived allows (4.1-4.95).
        assert!(
            rules
                .iter()
                .all(|r| r["priority"].as_integer().unwrap() >= 998)
        );
        assert_eq!(shim.setup.command_source, CommandSource::Title);
        assert_eq!(shim.setup.mode.as_deref(), Some("default"));
        assert!(first_prompt_preamble(manifest("gemini")).is_some());
        assert!(first_prompt_preamble(manifest("claude-code")).is_none());
        // A second launch into the same dir would find the files there: refused, not followed.
        assert!(prepare(manifest("gemini"), &context(dir.path(), &no_vars)).is_err());
    }

    /// A real home with every agent's config, credentials included.
    fn user_home() -> tempfile::TempDir {
        let home = tempfile::tempdir().unwrap();
        let put = |rel: &str, contents: &str| {
            let path = home.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        };
        put(
            ".claude/settings.json",
            r#"{"model":"opus","permissions":{"allow":["Bash(ls)"]}}"#,
        );
        put(
            ".claude/.credentials.json",
            r#"{"claudeAiOauth":{"accessToken":"real-token"}}"#,
        );
        put(".claude/history.jsonl", "secret prompts");
        put(".claude/skills/review/SKILL.md", "a skill");
        put(".claude/CLAUDE.md", "my instructions");
        put(
            ".claude.json",
            r#"{"mcpServers":{"docs":{"command":"docs-mcp"}},"oauthAccount":{"emailAddress":"u@example.com"},"projects":{"/x":{}}}"#,
        );
        put(".codex/config.toml", "model = \"gpt-5\"\n");
        put(
            ".codex/auth.json",
            r#"{"auth_mode":"chatgpt","tokens":{"access_token":"real-token"}}"#,
        );
        put(".codex/AGENTS.md", "codex notes");
        put(
            ".gemini/settings.json",
            r#"{"model":{"name":"gemini-pro"}}"#,
        );
        put(
            ".gemini/oauth_creds.json",
            r#"{"access_token":"real-token"}"#,
        );
        home
    }

    #[test]
    fn agents_get_the_users_settings_and_extensions_but_never_credentials() {
        let home = user_home();
        let root = std::fs::canonicalize(home.path()).unwrap();
        let claude = home_config(manifest("claude-code"), home.path(), None).unwrap();
        assert_eq!(
            claude.claude_settings.as_deref(),
            Some(br#"{"model":"opus","permissions":{"allow":["Bash(ls)"]}}"#.as_slice())
        );
        assert_eq!(
            claude.links,
            [
                (
                    PathBuf::from(".claude/CLAUDE.md"),
                    root.join(".claude/CLAUDE.md")
                ),
                (PathBuf::from(".claude/skills"), root.join(".claude/skills")),
            ]
        );
        assert_eq!(
            claude.read_only(),
            [root.join(".claude/CLAUDE.md"), root.join(".claude/skills")]
        );
        // Only the MCP servers from ~/.claude.json, none of its account or project state.
        let (path, kept) = &claude.files[0];
        assert_eq!(path, Path::new(".claude.json"));
        let kept: Value = serde_json::from_slice(kept).unwrap();
        assert_eq!(
            kept,
            json!({"mcpServers": {"docs": {"command": "docs-mcp"}}})
        );
        assert_eq!(claude.files.len(), 1);

        let codex = home_config(manifest("codex"), home.path(), None).unwrap();
        assert_eq!(
            codex.files,
            [(
                PathBuf::from(".codex/config.toml"),
                b"model = \"gpt-5\"\n".to_vec()
            )]
        );
        assert_eq!(
            codex.links,
            [(
                PathBuf::from(".codex/AGENTS.md"),
                root.join(".codex/AGENTS.md")
            )]
        );

        let gemini = home_config(manifest("gemini"), home.path(), None).unwrap();
        assert_eq!(gemini.files[0].0, Path::new(".gemini/settings.json"));
        assert!(gemini.links.is_empty());

        for config in [&claude, &codex, &gemini] {
            let all = format!("{config:?}");
            assert!(
                !all.contains("real-token") && !all.contains("secret prompts"),
                "{all}"
            );
            assert!(
                !config
                    .read_only()
                    .iter()
                    .any(|p| p == &root || p.ends_with(".claude") || p.ends_with(".codex"))
            );
        }
        assert_eq!(
            home_config(None, home.path(), None).unwrap(),
            HomeConfig::default()
        );
    }

    #[test]
    fn a_settings_file_that_isnt_a_regular_file_is_refused() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(home.path().join(".claude/settings.json")).unwrap();
        assert!(matches!(
            home_config(manifest("claude-code"), home.path(), None),
            Err(ShimError::Read { .. })
        ));
    }

    #[test]
    fn a_chatgpt_login_reaches_codex_only_as_placeholders() {
        let home = user_home();
        let claims = |c: Value| {
            use base64::Engine as _;
            let body = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(c.to_string());
            format!("eyJhbGciOiJSUzI1NiJ9.{body}.real-signature")
        };
        let exp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 600;
        let access = claims(
            json!({"exp": exp, "https://api.openai.com/auth": {"chatgpt_plan_type": "pro"}}),
        );
        let id = claims(json!({"exp": exp, "email": "u@example.com"}));
        let auth = json!({
            "auth_mode": "chatgpt",
            "tokens": {"access_token": access, "id_token": id, "refresh_token": "real-refresh", "account_id": "acct-1"},
        });
        std::fs::write(home.path().join(".codex/auth.json"), auth.to_string()).unwrap();
        let login = crate::logins::find(LoginKind::Chatgpt, home.path())
            .unwrap()
            .unwrap();
        let config = home_config(manifest("codex"), home.path(), Some(&login)).unwrap();
        let (path, written) = config.files.last().unwrap();
        assert_eq!(path, Path::new(".codex/auth.json"));
        let text = String::from_utf8(written.clone()).unwrap();
        assert!(
            !text.contains("real-signature") && !text.contains("real-refresh"),
            "{text}"
        );
        let written: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(written["auth_mode"], "chatgpt");
        assert_eq!(written["tokens"]["account_id"], "acct-1");
        assert_eq!(
            written["tokens"]["refresh_token"],
            forge_sandbox::secrets::PLACEHOLDER
        );
        // Codex reads the claims; they expire far off, so it never refreshes by itself.
        let payload = |jwt: &str| {
            use base64::Engine as _;
            let part = jwt.split('.').nth(1).unwrap();
            let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(part)
                .unwrap();
            serde_json::from_slice::<Value>(&bytes).unwrap()
        };
        let id_claims = payload(written["tokens"]["id_token"].as_str().unwrap());
        assert_eq!(id_claims["email"], "u@example.com");
        assert!(id_claims["exp"].as_u64().unwrap() > exp + 365 * 24 * 3600);
        let access_claims = payload(written["tokens"]["access_token"].as_str().unwrap());
        assert_eq!(
            access_claims["https://api.openai.com/auth"]["chatgpt_plan_type"],
            "pro"
        );
    }

    #[test]
    fn the_home_is_installed_without_following_anything_there() {
        let home = user_home();
        let config = home_config(manifest("claude-code"), home.path(), None).unwrap();
        let sandbox_home = tempfile::tempdir().unwrap();
        install_home(&config, sandbox_home.path(), sandbox_home.path()).unwrap();
        let skills = sandbox_home.path().join(".claude/skills");
        assert!(std::fs::symlink_metadata(&skills).unwrap().is_symlink());
        assert!(skills.join("review/SKILL.md").is_file());
        assert!(sandbox_home.path().join(".claude.json").is_file());
        assert!(
            !sandbox_home
                .path()
                .join(".claude/.credentials.json")
                .exists()
        );
        // A second install over what's there is refused, not followed.
        assert!(install_home(&config, sandbox_home.path(), sandbox_home.path()).is_err());
        // Shown at the real home's path, the targets' own mounts stand where links would.
        let standard = tempfile::tempdir().unwrap();
        let real_home = std::fs::canonicalize(home.path()).unwrap();
        install_home(&config, standard.path(), &real_home).unwrap();
        assert!(!standard.path().join(".claude/skills").exists());
        assert!(standard.path().join(".claude.json").is_file());
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
                mcp_url: None,
                ..context(dir.path(), &no_vars)
            },
        )
        .unwrap();
        assert!(none.setup.mcp_servers.is_empty());
    }
}
