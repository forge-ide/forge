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
    /// The agent's working directory, a worktree.
    pub worktree: &'a Path,
    /// Each of the agent's [`repo_config`] paths as the session branch's last commit holds
    /// it: what the user accepted, as opposed to what the agent may since have written.
    pub committed: &'a [(String, Option<String>)],
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
    /// The worktree's copy of config the agent acts on without asking differs from the
    /// branch's last commit: something in the session wrote it.
    #[error(
        "{path} in the worktree differs from the branch's last commit, and {agent} would act on \
         it without asking Forge; restore it (git checkout -- {path}) or commit it yourself, \
         then start {agent} again"
    )]
    RepoConfigChanged { agent: String, path: String },
    /// A config file names an MCP server after Forge's, whose tools Forge lets through
    /// without asking.
    #[error(
        "{path} defines an MCP server named `{MCP_SERVER}`, the name of Forge's own; rename it \
         there, then start {agent} again"
    )]
    ServerNameTaken { agent: String, path: String },
    /// The agent keeps its own login, and it isn't there.
    #[error("{agent} isn't signed in; sign in with `forge agent sign-in {id}`")]
    NotSignedIn { agent: String, id: String },
}

/// Whether a TOML config defines `[mcp_servers.<MCP_SERVER>]`. One that doesn't parse is
/// checked by its agent, which refuses it.
fn toml_names_forge(raw: &[u8]) -> bool {
    std::str::from_utf8(raw)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .is_some_and(|t| {
            t.get("mcp_servers")
                .and_then(|s| s.as_table())
                .is_some_and(|s| s.contains_key(MCP_SERVER))
        })
}

/// Whether a Gemini JSON config's `mcpServers` has a server named [`MCP_SERVER`], read as
/// Gemini reads it: comments stripped, then parsed as JSON. `None` if it doesn't parse, which
/// isn't proof Gemini can't read it (JavaScript takes a lone surrogate escape serde refuses),
/// so the caller refuses the launch.
fn json_names_forge(raw: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(raw).ok()?;
    let v: serde_json::Value = serde_json::from_str(&strip_json_comments(text)).ok()?;
    Some(
        v.get("mcpServers")
            .is_some_and(|s| s.get(MCP_SERVER).is_some()),
    )
}

/// `text` with its `//` and `/* */` comments blanked out, as Gemini's `strip-json-comments`
/// does: markers inside strings are text, and an unclosed comment runs to the end.
fn strip_json_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            match c {
                '\\' => out.extend(chars.next()),
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                while chars.next_if(|&c| c != '\n').is_some() {}
                out.push(' ');
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// The repo files each agent reads its project config from, which a shim takes from the
/// branch's last commit (Claude) or checks against it (Codex and Gemini, whose CLIs read them
/// from the worktree at startup, and can't be pointed elsewhere).
pub fn repo_config(agent: &str) -> &'static [&'static str] {
    match agent {
        "claude-code" => &[".claude/settings.json"],
        "codex" => &[".codex/config.toml", ".codex/hooks.json"],
        // A trusted worktree's .env is loaded into Gemini's environment, which every shell
        // command inherits.
        "gemini" => &[".gemini/settings.json", ".gemini/.env", ".env"],
        // Its workspace hooks, which it runs only for a trusted workspace; Forge's copy of its
        // trust list trusts none, and these are checked too.
        "antigravity" => &[
            ".agents/hooks.json",
            ".agent/hooks.json",
            "_agents/hooks.json",
            "_agent/hooks.json",
        ],
        _ => &[],
    }
}

/// Whether the worktree's copy of `path` is what the branch committed: both missing, or a
/// regular file with the same contents.
fn unchanged(worktree: &Path, path: &str, committed: Option<&str>) -> bool {
    let full = worktree.join(path);
    match (std::fs::symlink_metadata(&full), committed) {
        (Err(e), None) => e.kind() == std::io::ErrorKind::NotFound,
        (Ok(meta), Some(committed)) if meta.is_file() => {
            std::fs::read(&full).is_ok_and(|bytes| bytes == committed.as_bytes())
        }
        _ => false,
    }
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
    shim.setup.mcp_meta = manifest.permissions.mcp_meta;
    shim.env.extend(
        manifest
            .launch
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone())),
    );
    if manifest.id != "claude-code" {
        for (path, committed) in cx.committed {
            if !unchanged(cx.worktree, path, committed.as_deref()) {
                return Err(ShimError::RepoConfigChanged {
                    agent: manifest.name.clone(),
                    path: path.clone(),
                });
            }
        }
    }
    if let Some((path, _)) = cx.committed.iter().find(|(path, raw)| {
        let Some(raw) = raw.as_deref() else {
            return false;
        };
        match (manifest.id.as_str(), path.as_str()) {
            ("codex", ".codex/config.toml") => toml_names_forge(raw.as_bytes()),
            // One it can't check is refused below.
            ("gemini", ".gemini/settings.json") => {
                json_names_forge(raw.as_bytes()).unwrap_or(false)
            }
            _ => false,
        }
    }) {
        return Err(ShimError::ServerNameTaken {
            agent: manifest.name.clone(),
            path: path.clone(),
        });
    }
    if manifest.id == "gemini"
        && let Some((path, _)) = cx.committed.iter().find(|(path, raw)| {
            path == ".gemini/settings.json"
                && raw
                    .as_deref()
                    .is_some_and(|raw| json_names_forge(raw.as_bytes()).is_none())
        })
    {
        return Err(ShimError::Read {
            agent: manifest.name.clone(),
            why: format!("{path} in the repo isn't JSON Forge can check"),
        });
    }
    let failed = |source| ShimError::Write {
        agent: manifest.id.clone(),
        source,
    };
    match manifest.id.as_str() {
        "claude-code" => claude(&mut shim, cx, manifest)?,
        "codex" => codex(&mut shim, cx),
        "gemini" => gemini(&mut shim, cx).map_err(failed)?,
        "antigravity" => antigravity(&mut shim, cx),
        _ => {}
    }
    Ok(shim)
}

/// Claude Code: one settings file, the user's settings merged with the repo's committed
/// `.claude/settings.json`, in the read-only shim dir, and no settings files of its own (it
/// watches them, so one an approved command wrote in the worktree or the sandbox home would
/// take effect at once); its bypass mode refused, its web tools off, only `git_commit`
/// pre-allowed, and the `default` mode, in which every tool the settings don't allow asks the
/// client.
fn claude(shim: &mut Shim, cx: &ShimContext<'_>, manifest: &Manifest) -> Result<(), ShimError> {
    let unreadable = |why: String| ShimError::Read {
        agent: manifest.name.clone(),
        why,
    };
    let user = match &cx.home.claude_settings {
        Some(raw) => Some(
            serde_json::from_slice::<serde_json::Value>(raw)
                .map_err(|e| unreadable(format!("~/.claude/settings.json: {e}")))?,
        ),
        None => None,
    };
    let project = match cx.committed.iter().find_map(|(_, c)| c.as_deref()) {
        Some(raw) => Some(
            serde_json::from_str::<serde_json::Value>(raw)
                .map_err(|e| unreadable(format!("the repo's .claude/settings.json: {e}")))?,
        ),
        None => None,
    };
    let mut options = json!({
        "settingSources": [],
        "allowDangerouslySkipPermissions": false,
        "allowedTools": [format!("mcp__{MCP_SERVER}__git_commit")],
        "disallowedTools": ["WebFetch", "WebSearch"],
        "promptSuggestions": false,
    });
    let merged = match (user, project) {
        (Some(mut user), Some(project)) => {
            merge_settings(&mut user, project);
            Some(user)
        }
        (one, other) => one.or(other),
    };
    if let Some(settings) = merged {
        let path = cx.dir.join("claude-settings.json");
        write_new(&path, settings.to_string().as_bytes()).map_err(|source| ShimError::Write {
            agent: manifest.id.clone(),
            source,
        })?;
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

/// Merge `over` into `base` as Claude Code layers its settings: objects key by key, arrays
/// concatenated without repeats, and anything else replaced by `over`'s.
fn merge_settings(base: &mut serde_json::Value, over: serde_json::Value) {
    use serde_json::Value;
    match (base, over) {
        (Value::Object(base), Value::Object(over)) => {
            for (key, value) in over {
                match base.get_mut(&key) {
                    Some(existing) => merge_settings(existing, value),
                    None => {
                        base.insert(key, value);
                    }
                }
            }
        }
        (Value::Array(base), Value::Array(over)) => {
            for value in over {
                if !base.contains(&value) {
                    base.push(value);
                }
            }
        }
        (base, over) => *base = over,
    }
}

/// Codex: the read-only mode (see the manifest's quirks), plain `-c` shells, escalations
/// decided by the user (through Forge) rather than Codex's own reviewer, Forge's endpoint with
/// `git_commit` pre-approved, and its web search and phoning home off. `CODEX_CONFIG` is
/// applied as overrides above every config file, the user's and the repo's.
fn codex(shim: &mut Shim, cx: &ShimContext<'_>) {
    let mut config = json!({
        "allow_login_shell": false,
        "features": { "shell_snapshot": false },
        // Its commands get the environment its env policy builds, which a config could make
        // leave out the sandbox's startup-file pins; `set` always applies.
        "shell_environment_policy": {
            "set": forge_sandbox::STARTUP_FILES_OFF
                .iter()
                .map(|(k, v)| ((*k).to_owned(), serde_json::Value::from(*v)))
                .collect::<serde_json::Map<_, _>>(),
        },
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

/// Antigravity: its config in the session's own copy ([`stage`]), and the `default` mode,
/// which asks the client about every command and edit (its `auto_edit` and `yolo` modes
/// don't).
fn antigravity(shim: &mut Shim, cx: &ShimContext<'_>) {
    shim.env.push((
        "GEMINI_HOME".into(),
        staged_home(cx.dir).display().to_string(),
    ));
    // Set, it would trust every workspace.
    shim.env
        .push(("AGY_ACP_DISABLE_WORKSPACE_TRUST".into(), "0".into()));
    shim.setup.mode = Some("default".into());
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
    /// Values in the copies that look like credentials (an MCP server's token in its `env`, a
    /// key in Claude's `env`): the agent reads them as it would outside Forge, but the log
    /// redactor hides them, and a command carrying one asks.
    pub secrets: Vec<String>,
    /// The login an agent keeps and refreshes itself, in the user's real config, copied into
    /// the session at each launch ([`stage`]). Nothing goes back: its refresh token doesn't
    /// rotate, and a session's copy could name any file or account.
    pub own_login: Option<PathBuf>,
}

/// Where a session's copy of an agent's own config is: the shim dir's `gemini/` (Antigravity's
/// `GEMINI_HOME`), read-only but for its login dir and its artifacts dir.
fn staged_home(shim_dir: &Path) -> PathBuf {
    shim_dir.join("gemini")
}

/// What the sandbox must show for a session's own copy of an agent's config.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Staged {
    /// Dirs inside the read-only shim dir that the session writes.
    pub read_write: Vec<PathBuf>,
    /// Files inside those that stay read-only (its trust list, its auth settings).
    pub read_only: Vec<PathBuf>,
}

/// Make the session's copy of the agent's own config in `shim_dir`, before the sandbox starts.
/// For Antigravity: an empty `config/` (no hooks, MCP servers, or skills of the user's), a
/// login dir holding copies of the user's token and auth settings, and a trust list that marks
/// the worktree untrusted, so it never runs a repo's hooks. The trust list and settings stay
/// read-only; the session writes only the token copy, its conversations, and its artifacts, none
/// of which reaches the user's real config.
pub fn stage(home: &HomeConfig, shim_dir: &Path, worktree: &Path) -> std::io::Result<Staged> {
    let Some(token) = &home.own_login else {
        return Ok(Staged::default());
    };
    let root = staged_home(shim_dir);
    let login = root.join("antigravity-acp");
    let artifacts = root.join("artifacts");
    for dir in [root.join("config"), artifacts.clone(), login.clone()] {
        std::fs::create_dir_all(&dir)?;
    }
    let real = token.parent().expect("a file has a parent");
    write_new(&login.join("acp_token.json"), &std::fs::read(token)?)?;
    let settings = match std::fs::read(real.join("settings.json")) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            br#"{"auth":{"type":"oauth-personal"}}"#.to_vec()
        }
        Err(e) => return Err(e),
    };
    write_new(&login.join("settings.json"), &settings)?;
    let worktree = std::fs::canonicalize(worktree).unwrap_or_else(|_| worktree.to_path_buf());
    let trust = json!({ "trusted": [], "untrusted": [worktree] });
    write_new(
        &login.join("trusted_workspaces.json"),
        trust.to_string().as_bytes(),
    )?;
    Ok(Staged {
        read_write: vec![login.clone(), artifacts],
        read_only: vec![
            login.join("trusted_workspaces.json"),
            login.join("settings.json"),
        ],
    })
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
    // An entry the user linked elsewhere inside an agent's config dir (to its sign-in, say) is
    // left out: only the entry itself, or something outside those dirs, is shown.
    let agent_dirs: Vec<PathBuf> = AGENT_HOME_ENTRIES
        .iter()
        .filter_map(|e| std::fs::canonicalize(home.join(e)).ok())
        .collect();
    let canonical_home = std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf());
    for entry in entries {
        let rel = Path::new(dir).join(entry);
        // By the resolved path, which is where the sandbox shows it.
        if let Ok(real) = std::fs::canonicalize(home.join(&rel)) {
            let itself = real == canonical_home.join(&rel);
            if itself || !agent_dirs.iter().any(|d| real.starts_with(d)) {
                out.links.push((rel, real));
            }
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
                    if servers.get(MCP_SERVER).is_some() {
                        return Err(ShimError::ServerNameTaken {
                            agent: manifest.name.clone(),
                            path: "~/.claude.json".into(),
                        });
                    }
                    let kept = json!({ "mcpServers": servers });
                    out.files
                        .push((".claude.json".into(), kept.to_string().into_bytes()));
                }
            }
        }
        "codex" => {
            if let Some(raw) = read_settings(&home.join(".codex/config.toml")).map_err(failed)? {
                if toml_names_forge(&raw) {
                    return Err(ShimError::ServerNameTaken {
                        agent: manifest.name.clone(),
                        path: "~/.codex/config.toml".into(),
                    });
                }
                out.files.push((".codex/config.toml".into(), raw));
            }
            if let Some(account) = login.and_then(|l| l.account.as_ref()) {
                out.files
                    .push((".codex/auth.json".into(), codex_auth(account).into_bytes()));
            }
        }
        "antigravity" => {
            let token = manifest
                .auth
                .own_login
                .as_ref()
                .map(|login| home.join(login))
                .filter(|t| t.is_file())
                .ok_or_else(|| ShimError::NotSignedIn {
                    agent: manifest.name.clone(),
                    id: manifest.id.clone(),
                })?;
            out.own_login = Some(token.clone());
            // Its token's secrets, for the redactor; Google rarely rotates a refresh token.
            if let Some(raw) = read_settings(&token).map_err(failed)?
                && let Ok(token) = serde_json::from_slice::<serde_json::Value>(&raw)
            {
                for key in ["refresh_token", "client_secret"] {
                    if let Some(value) = token.get(key).and_then(|v| v.as_str()) {
                        out.secrets.push(value.to_owned());
                    }
                }
            }
        }
        "gemini" => {
            let check = |raw: &[u8], path: String| match json_names_forge(raw) {
                Some(false) => Ok(()),
                Some(true) => Err(ShimError::ServerNameTaken {
                    agent: manifest.name.clone(),
                    path,
                }),
                None => Err(failed(format!("{path} isn't JSON Forge can check"))),
            };
            // Extensions aren't checked: Gemini registers the server ACP offers first and
            // skips an extension's server of a name it already has.
            if let Some(raw) = read_settings(&home.join(".gemini/settings.json")).map_err(failed)? {
                check(&raw, "~/.gemini/settings.json".into())?;
                out.files.push((".gemini/settings.json".into(), raw));
            }
        }
        _ => {}
    }
    let mut found = Vec::new();
    if let Some(raw) = &out.claude_settings {
        found.extend(serde_json::from_slice::<serde_json::Value>(raw).ok());
    }
    for (path, raw) in &out.files {
        // A login's placeholders hold nothing secret.
        if path.ends_with("auth.json") {
            continue;
        }
        let parsed = if path.extension().is_some_and(|e| e == "toml") {
            std::str::from_utf8(raw)
                .ok()
                .and_then(|text| toml::from_str::<toml::Value>(text).ok())
                .and_then(|v| serde_json::to_value(v).ok())
        } else {
            serde_json::from_slice(raw).ok()
        };
        found.extend(parsed);
    }
    for value in &found {
        credential_values(value, false, &mut out.secrets);
    }
    out.secrets.sort();
    out.secrets.dedup();
    Ok(out)
}

/// Keys whose values are credentials, or hold them.
fn holds_credentials(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "env" | "headers" | "http_headers" | "env_http_headers"
    ) || [
        "token",
        "key",
        "secret",
        "password",
        "authorization",
        "bearer",
    ]
    .iter()
    .any(|word| key.contains(word))
}

/// The strings in `value` under a key that [`holds_credentials`], except ones that are plainly
/// paths, variable references, or URLs carrying nothing secret; and anywhere, a URL with
/// credentials or a query in it (an MCP server's `url` with `?api_key=`, a webhook).
fn credential_values(value: &serde_json::Value, inside: bool, out: &mut Vec<String>) {
    use serde_json::Value;
    match value {
        Value::Object(map) => {
            for (key, value) in map {
                credential_values(value, inside || holds_credentials(key), out);
            }
        }
        Value::String(s) if s.starts_with("http") => {
            // Userinfo or a query can carry a key, and so can a long path in a credential's
            // place (a webhook's).
            let rest = s.split_once("://").map_or("", |(_, rest)| rest);
            let authority = rest.split('/').next().unwrap_or_default();
            if authority.contains('@') || rest.contains('?') || (inside && rest.contains('/')) {
                out.push(s.clone());
            }
        }
        Value::Array(items) => {
            for item in items {
                credential_values(item, inside, out);
            }
        }
        Value::String(s) if inside => {
            let plain = ["http", "/", "~", "$"].iter().any(|p| s.starts_with(p));
            if !plain && !s.trim().is_empty() {
                out.push(s.clone());
            }
        }
        _ => {}
    }
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
    // Checked before opening too, so a FIFO there can't block the open.
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() => {
            return Err(format!("{} is not a regular file", path.display()));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => {}
    }
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

/// Remove the agents' entries ([`AGENT_HOME_ENTRIES`]) from the sandbox home `home`, at each
/// launch and when the session ends. Nothing may run in the session meanwhile; removing a
/// directory never follows a symlink inside it.
pub fn clear_home(home: &Path) -> std::io::Result<()> {
    for entry in AGENT_HOME_ENTRIES {
        let path = home.join(entry);
        let removed = match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(&path),
            Ok(_) => std::fs::remove_file(&path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        };
        removed.map_err(|e| {
            std::io::Error::new(e.kind(), format!("clearing {}: {e}", path.display()))
        })?;
    }
    Ok(())
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
/// Only its owner can read it: it may hold the user's own settings.
fn write_new(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
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
        secrets: Vec::new(),
        own_login: None,
    };

    fn context<'a>(dir: &'a Path, var: &'a dyn Fn(&str) -> Option<String>) -> ShimContext<'a> {
        ShimContext {
            dir,
            mcp_url: Some("http://127.0.0.1:9/_forge/tok/mcp".into()),
            var,
            home: &NO_HOME,
            login: None,
            // An empty worktree, matching a branch with no agent config: tests that need
            // files pass their own.
            worktree: Path::new("/nonexistent-forge-worktree"),
            committed: &[],
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
        // None at all: Claude watches its settings files, and the agent can write the sandbox
        // home's and the worktree's.
        assert_eq!(options["settingSources"], json!([]));
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
        // The user's settings and the repo's committed ones go in from the read-only shim
        // dir, merged as Claude layers them; what the worktree holds now doesn't count.
        let home = HomeConfig {
            claude_settings: Some(
                br#"{"model":"opus","permissions":{"allow":["Bash(ls)"]},"env":{"A":"1"}}"#
                    .to_vec(),
            ),
            ..HomeConfig::default()
        };
        let committed = [(
            ".claude/settings.json".to_owned(),
            Some(
                r#"{"model":"sonnet","permissions":{"allow":["Bash(ls)","Bash(make)"]}}"#
                    .to_owned(),
            ),
        )];
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(worktree.join(".claude")).unwrap();
        std::fs::write(
            worktree.join(".claude/settings.json"),
            r#"{"permissions":{"allow":["Bash"]}}"#,
        )
        .unwrap();
        let shim_dir = dir.path().join("shim");
        std::fs::create_dir(&shim_dir).unwrap();
        let cx = ShimContext {
            home: &home,
            worktree: &worktree,
            committed: &committed,
            ..context(&shim_dir, &no_vars)
        };
        let shim = prepare(manifest("claude-code"), &cx).unwrap();
        let meta = Value::Object(shim.setup.meta.unwrap());
        let path = shim_dir.join("claude-settings.json");
        assert_eq!(
            meta["claudeCode"]["options"]["settings"],
            path.display().to_string()
        );
        let merged: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            merged,
            json!({"model":"sonnet","permissions":{"allow":["Bash(ls)","Bash(make)"]},"env":{"A":"1"}})
        );
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
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
        // Its commands keep the sandbox's startup-file pins whatever a config's env policy.
        let set = &config["shell_environment_policy"]["set"];
        assert_eq!(set["ZDOTDIR"], "/dev/null");
        assert_eq!(set["BASH_ENV"], "");
        assert_eq!(set["ENV"], "");
        assert_eq!(set["SHLVL"], "1");
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
    fn codex_and_gemini_wont_start_on_repo_config_changed_since_the_last_commit() {
        for (agent, path) in [
            ("codex", ".codex/config.toml"),
            ("codex", ".codex/hooks.json"),
            ("gemini", ".gemini/settings.json"),
            ("gemini", ".gemini/.env"),
            ("gemini", ".env"),
        ] {
            assert!(repo_config(agent).contains(&path), "{agent} {path}");
            let dir = tempfile::tempdir().unwrap();
            let worktree = dir.path().join("wt");
            std::fs::create_dir_all(worktree.join(path).parent().unwrap()).unwrap();
            let shim_dir = dir.path().join("shim");
            let attempt = |committed: Option<&str>| {
                let _ = std::fs::remove_dir_all(&shim_dir);
                std::fs::create_dir(&shim_dir).unwrap();
                let committed = [(path.to_owned(), committed.map(str::to_owned))];
                let cx = ShimContext {
                    worktree: &worktree,
                    committed: &committed,
                    ..context(&shim_dir, &no_vars)
                };
                match prepare(manifest(agent), &cx) {
                    Err(ShimError::RepoConfigChanged { path: found, .. }) => Some(found),
                    Ok(_) => None,
                    Err(e) => panic!("{agent} {path}: {e}"),
                }
            };
            // As committed, or absent from both: fine.
            assert_eq!(attempt(None), None, "{agent} {path}");
            // Contents each agent can read.
            let committed = if path.ends_with(".json") {
                r#"{"committed": true}"#
            } else {
                "committed"
            };
            std::fs::write(worktree.join(path), committed).unwrap();
            assert_eq!(attempt(Some(committed)), None, "{agent} {path}");
            // Written in the session, added, or removed: refused.
            std::fs::write(worktree.join(path), "rewritten").unwrap();
            assert_eq!(attempt(Some(committed)).as_deref(), Some(path));
            assert_eq!(attempt(None).as_deref(), Some(path));
            std::fs::remove_file(worktree.join(path)).unwrap();
            assert_eq!(attempt(Some(committed)).as_deref(), Some(path));
            // A symlink is never the committed file.
            std::os::unix::fs::symlink("/nonexistent", worktree.join(path)).unwrap();
            assert_eq!(attempt(None).as_deref(), Some(path));
        }
    }

    #[test]
    fn antigravity_works_on_a_copy_of_its_login_and_trusts_no_worktree() {
        let home = tempfile::tempdir().unwrap();
        let config = |home: &Path| home_config(manifest("antigravity"), home, None);
        // Not signed in: refused, naming how to sign in.
        let err = config(home.path()).unwrap_err().to_string();
        assert!(err.contains("forge agent sign-in antigravity"), "{err}");
        let real = home.path().join(".gemini/antigravity-acp");
        std::fs::create_dir_all(&real).unwrap();
        let token = real.join("acp_token.json");
        std::fs::write(
            &token,
            r#"{"client_id":"cid","client_secret":"GOCSPX-secretvalue","refresh_token":"1//refreshvalue","token_uri":"https://oauth2.googleapis.com/token"}"#,
        )
        .unwrap();
        // What the user's other Antigravity clients trust, which a session never sees.
        std::fs::write(
            real.join("trusted_workspaces.json"),
            r#"{"trusted":["/"],"untrusted":[]}"#,
        )
        .unwrap();
        let got = config(home.path()).unwrap();
        assert_eq!(got.own_login.as_deref(), Some(token.as_path()));
        assert!(got.links.is_empty() && got.files.is_empty());
        for value in ["GOCSPX-secretvalue", "1//refreshvalue"] {
            assert!(got.secrets.iter().any(|s| s == value), "{value}");
        }

        let shim_dir = home.path().join("shim");
        std::fs::create_dir(&shim_dir).unwrap();
        let worktree = home.path().join("repo/.worktrees/feat/x");
        std::fs::create_dir_all(&worktree).unwrap();
        let staged = stage(&got, &shim_dir, &worktree).unwrap();
        let login = shim_dir.join("gemini/antigravity-acp");
        assert_eq!(
            staged.read_write,
            [login.clone(), shim_dir.join("gemini/artifacts")]
        );
        assert_eq!(
            staged.read_only,
            [
                login.join("trusted_workspaces.json"),
                login.join("settings.json")
            ]
        );
        // No hooks, MCP servers, or skills of the user's; the worktree untrusted.
        assert_eq!(
            std::fs::read_dir(shim_dir.join("gemini/config"))
                .unwrap()
                .count(),
            0
        );
        let trust: Value =
            serde_json::from_slice(&std::fs::read(login.join("trusted_workspaces.json")).unwrap())
                .unwrap();
        assert_eq!(trust["trusted"], json!([]));
        assert_eq!(
            trust["untrusted"],
            json!([std::fs::canonicalize(&worktree).unwrap()])
        );
        assert_eq!(
            std::fs::read(login.join("acp_token.json")).unwrap(),
            std::fs::read(&token).unwrap()
        );

        let shim = prepare(
            manifest("antigravity"),
            &ShimContext {
                home: &got,
                worktree: &worktree,
                ..context(&shim_dir, &no_vars)
            },
        )
        .unwrap();
        assert_eq!(shim.setup.mode.as_deref(), Some("default"));
        let env: std::collections::HashMap<_, _> = shim.env.iter().cloned().collect();
        assert_eq!(
            env["GEMINI_HOME"],
            shim_dir.join("gemini").display().to_string()
        );
        assert_eq!(env["AGY_ACP_DISABLE_WORKSPACE_TRUST"], "0");
    }

    /// Forge lets its own server's tools through without asking (they ask Forge when they
    /// run), so a server of the same name from a config file would get the same pass.
    #[test]
    fn a_config_naming_its_own_server_after_forges_is_refused() {
        let taken = |agent: &str, rel: &str, contents: &str| {
            let home = tempfile::tempdir().unwrap();
            let path = home.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
            match home_config(manifest(agent), home.path(), None) {
                Err(ShimError::ServerNameTaken { path, .. }) => Some(path),
                Ok(_) => None,
                Err(e) => panic!("{agent} {rel}: {e}"),
            }
        };
        let codex = "[mcp_servers.forge]\ncommand = \"impostor\"\n";
        let claude = r#"{"mcpServers":{"forge":{"command":"impostor"}}}"#;
        assert_eq!(
            taken("codex", ".codex/config.toml", codex).as_deref(),
            Some("~/.codex/config.toml")
        );
        assert_eq!(
            taken("claude-code", ".claude.json", claude).as_deref(),
            Some("~/.claude.json")
        );
        let gemini = r#"{"mcpServers":{"forge":{"command":"impostor"}}}"#;
        assert_eq!(
            taken("gemini", ".gemini/settings.json", gemini).as_deref(),
            Some("~/.gemini/settings.json")
        );
        // Gemini's settings may hold comments, which it strips before parsing, wherever they
        // are; and a key may be spelled with escapes.
        for commented in [
            "{\n  // mine\n  \"mcpServers\": { \"forge\" : {} }\n}",
            "{\"mcpServers\": { \"forge\" /* x */ : {} }}",
            "{\"mcpServers\": { \"forge\" // x\n : {} }}",
            "{\"mcpServers\": { \"for\\u0067e\": {} }}",
            "{/* \"mcpServers\": {} */ \"mcpServers\": { \"forge\": {} }}",
        ] {
            assert_eq!(
                taken("gemini", ".gemini/settings.json", commented).as_deref(),
                Some("~/.gemini/settings.json"),
                "{commented}"
            );
        }
        // Comment markers inside strings are text.
        assert_eq!(
            taken(
                "gemini",
                ".gemini/settings.json",
                "{\"url\": \"http://x/*\", \"mcpServers\": {\"docs\": {}}, \"y\": \"*/\"}"
            ),
            None
        );
        // Other servers are fine.
        assert_eq!(
            taken(
                "gemini",
                ".gemini/settings.json",
                r#"{"mcpServers":{"docs":{}}}"#
            ),
            None
        );
        let others = "[mcp_servers.docs]\ncommand = \"docs\"\n";
        assert_eq!(taken("codex", ".codex/config.toml", others), None);
        assert_eq!(
            taken(
                "claude-code",
                ".claude.json",
                r#"{"mcpServers":{"docs":{}}}"#
            ),
            None
        );

        // And in the repo's committed config, which Codex reads too.
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(worktree.join(".codex")).unwrap();
        std::fs::write(worktree.join(".codex/config.toml"), codex).unwrap();
        let shim_dir = dir.path().join("shim");
        std::fs::create_dir(&shim_dir).unwrap();
        let committed = [(".codex/config.toml".to_owned(), Some(codex.to_owned()))];
        let cx = ShimContext {
            worktree: &worktree,
            committed: &committed,
            ..context(&shim_dir, &no_vars)
        };
        assert!(matches!(
            prepare(manifest("codex"), &cx),
            Err(ShimError::ServerNameTaken { path, .. }) if path == ".codex/config.toml"
        ));
        std::fs::create_dir_all(worktree.join(".gemini")).unwrap();
        std::fs::write(worktree.join(".gemini/settings.json"), gemini).unwrap();
        let committed = [(".gemini/settings.json".to_owned(), Some(gemini.to_owned()))];
        let cx = ShimContext {
            worktree: &worktree,
            committed: &committed,
            ..context(&shim_dir, &no_vars)
        };
        assert!(matches!(
            prepare(manifest("gemini"), &cx),
            Err(ShimError::ServerNameTaken { path, .. }) if path == ".gemini/settings.json"
        ));
    }

    /// Settings Gemini would read but Forge can't check refuse the launch, rather than passing:
    /// a file serde can't parse may still be one Gemini reads (JavaScript takes a lone
    /// surrogate escape).
    #[test]
    fn gemini_config_forge_cant_check_refuses_the_launch() {
        let refused = |files: &[(&str, &str)]| {
            let home = tempfile::tempdir().unwrap();
            for (rel, contents) in files {
                let path = home.path().join(rel);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, contents).unwrap();
            }
            matches!(
                home_config(manifest("gemini"), home.path(), None),
                Err(ShimError::Read { .. })
            )
        };
        let surrogate = r#"{"x": "\ud800", "mcpServers": {"forge": {}}}"#;
        assert!(refused(&[(".gemini/settings.json", surrogate)]));
        assert!(!refused(&[(".gemini/settings.json", "{}")]));

        // The repo's committed settings, too.
        let dir = tempfile::tempdir().unwrap();
        let worktree = dir.path().join("wt");
        std::fs::create_dir_all(worktree.join(".gemini")).unwrap();
        std::fs::write(worktree.join(".gemini/settings.json"), surrogate).unwrap();
        let shim_dir = dir.path().join("shim");
        std::fs::create_dir(&shim_dir).unwrap();
        let no_vars = |_: &str| None;
        let committed = [(
            ".gemini/settings.json".to_owned(),
            Some(surrogate.to_owned()),
        )];
        let cx = ShimContext {
            worktree: &worktree,
            committed: &committed,
            ..context(&shim_dir, &no_vars)
        };
        assert!(matches!(
            prepare(manifest("gemini"), &cx),
            Err(ShimError::Read { .. })
        ));
    }

    #[test]
    fn credentials_in_the_users_settings_are_found_for_redaction() {
        let home = tempfile::tempdir().unwrap();
        let put = |rel: &str, contents: &str| {
            let path = home.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        };
        put(
            ".claude/settings.json",
            r#"{"env":{"GITHUB_TOKEN":"ghp_settingsvalue1","EDITOR_PATH":"/usr/bin/vim"},"model":"opus-long-name"}"#,
        );
        put(
            ".claude.json",
            r#"{"mcpServers":{"gh":{"command":"gh-mcp","env":{"PAT":"ghp_mcpvalue12345"},"headers":{"Authorization":"Bearer abcdefgh123"}}}}"#,
        );
        let claude = home_config(manifest("claude-code"), home.path(), None).unwrap();
        assert_eq!(
            claude.secrets,
            [
                "Bearer abcdefgh123",
                "ghp_mcpvalue12345",
                "ghp_settingsvalue1"
            ]
        );
        put(
            ".codex/config.toml",
            "model = \"gpt-5-codex-long\"\n[mcp_servers.x]\ncommand = \"x\"\nbearer_token = \"tok_codexvalue99\"\n[mcp_servers.x.env]\nAPI_KEY = \"sk-codexenv-1234\"\nHOME_URL = \"https://example.com\"\n",
        );
        let codex = home_config(manifest("codex"), home.path(), None).unwrap();
        assert_eq!(codex.secrets, ["sk-codexenv-1234", "tok_codexvalue99"]);
        // URLs that carry a key; a plain one doesn't count.
        put(
            ".gemini/settings.json",
            r#"{"mcpServers":{"a":{"httpUrl":"https://mcp.example/x?api_key=k1234567"},"b":{"url":"https://mcp.example/plain"}},"env":{"HOOK":"https://hooks.example/services/T/B/s3cr3t"},"x":"https://u:p4ssword@host.example"}"#,
        );
        let gemini = home_config(manifest("gemini"), home.path(), None).unwrap();
        assert_eq!(
            gemini.secrets,
            [
                "https://hooks.example/services/T/B/s3cr3t",
                "https://mcp.example/x?api_key=k1234567",
                "https://u:p4ssword@host.example",
            ]
        );
    }

    #[test]
    fn an_entry_linked_to_an_agents_sign_in_is_left_out() {
        let home = user_home();
        std::fs::remove_dir_all(home.path().join(".claude/skills")).unwrap();
        std::os::unix::fs::symlink(
            home.path().join(".claude/.credentials.json"),
            home.path().join(".claude/skills"),
        )
        .unwrap();
        let elsewhere = home.path().join("dotfiles/agents");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, home.path().join(".claude/agents")).unwrap();
        let config = home_config(manifest("claude-code"), home.path(), None).unwrap();
        let rels: Vec<_> = config.links.iter().map(|(rel, _)| rel.clone()).collect();
        assert_eq!(
            rels,
            [
                PathBuf::from(".claude/CLAUDE.md"),
                PathBuf::from(".claude/agents"),
            ]
        );
        assert!(!format!("{config:?}").contains("credentials"));
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
