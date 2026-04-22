//! Source-format converters for `forge mcp import` (F-131).
//!
//! Each third-party tool stores its MCP server declarations in a slightly
//! different on-disk layout. A converter module here reads the raw file
//! contents from one such source and returns a `BTreeMap<String,
//! McpServerSpec>` shaped for Forge's universal `.mcp.json` schema.
//!
//! The converters never touch the filesystem directly — callers feed them a
//! string so they're trivially testable with fixtures.
//!
//! # Import sources
//!
//! | Source       | File path                                                         |
//! |--------------|-------------------------------------------------------------------|
//! | VS Code      | `.vscode/mcp.json` (top-level `servers`)                          |
//! | Cursor       | `~/.cursor/mcp.json` (top-level `mcpServers`)                     |
//! | Claude       | `~/Library/Application Support/Claude/claude_desktop_config.json` |
//! | Continue     | `~/.continue/config.json` (one key among many)                    |
//! | Kiro         | `.kiro/mcp.json` (workspace) — mirrors Claude/Cursor              |
//! | Codex        | `.codex/config.toml` (workspace) — TOML `[mcp_servers.<name>]`    |
//!
//! The import path uses a deliberately *lenient* top-level parse: unknown
//! top-level keys are ignored (many sources nest `mcpServers` inside a
//! larger app config). Server entries themselves are strict — unknown
//! fields are dropped silently rather than rejected because source tools
//! routinely add tool-specific metadata (`disabled`, `autoApprove`,
//! `envFile`, `auth`, `supports_parallel_tool_calls`, ...) that have no
//! universal-schema analogue.

use crate::{build_server_kind, McpServerSpec, StrictFields};
use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

/// Identifiers for the six supported import sources. Matches the
/// `ImportSource` enum declared in `docs/architecture/crate-architecture.md`
/// §3.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSource {
    VsCode,
    Cursor,
    ClaudeDesktop,
    Continue,
    Kiro,
    Codex,
}

impl ImportSource {
    /// Human-readable slug used in CLI flags (`--source=<slug>`) and diff
    /// headers.
    pub fn slug(self) -> &'static str {
        match self {
            ImportSource::VsCode => "vscode",
            ImportSource::Cursor => "cursor",
            ImportSource::ClaudeDesktop => "claude",
            ImportSource::Continue => "continue",
            ImportSource::Kiro => "kiro",
            ImportSource::Codex => "codex",
        }
    }

    /// Every known source, in the order they're walked by auto-detect.
    pub fn all() -> &'static [ImportSource] {
        &[
            ImportSource::VsCode,
            ImportSource::Cursor,
            ImportSource::ClaudeDesktop,
            ImportSource::Continue,
            ImportSource::Kiro,
            ImportSource::Codex,
        ]
    }

    /// Parse the `--source=<slug>` CLI flag value.
    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::all().iter().copied().find(|s| s.slug() == slug)
    }

    /// Convert a raw file body into a `BTreeMap<String, McpServerSpec>`.
    pub fn convert(self, raw: &str) -> Result<BTreeMap<String, McpServerSpec>> {
        match self {
            ImportSource::VsCode => vscode::convert(raw),
            ImportSource::Cursor => cursor::convert(raw),
            ImportSource::ClaudeDesktop => claude::convert(raw),
            ImportSource::Continue => continue_::convert(raw),
            ImportSource::Kiro => kiro::convert(raw),
            ImportSource::Codex => codex::convert(raw),
        }
    }

    /// Default on-disk location for this source, if one can be resolved
    /// from the given workspace root and home directory. Returning `None`
    /// means "can't locate without more info" — e.g. missing `$HOME`.
    pub fn default_path(self, workspace_root: &Path, home: &Path) -> Option<PathBuf> {
        match self {
            ImportSource::VsCode => Some(workspace_root.join(".vscode").join("mcp.json")),
            ImportSource::Cursor => Some(home.join(".cursor").join("mcp.json")),
            ImportSource::ClaudeDesktop => {
                // macOS canonical path per the MCP docs. Linux/Windows have
                // their own locations; we pick the macOS one because it's
                // the one the Issue cites verbatim and because Claude
                // Desktop is macOS/Windows-only anyway.
                Some(
                    home.join("Library")
                        .join("Application Support")
                        .join("Claude")
                        .join("claude_desktop_config.json"),
                )
            }
            ImportSource::Continue => Some(home.join(".continue").join("config.json")),
            ImportSource::Kiro => Some(workspace_root.join(".kiro").join("mcp.json")),
            ImportSource::Codex => Some(workspace_root.join(".codex").join("config.toml")),
        }
    }
}

/// Auto-detection report: which sources were found on disk, and the
/// converted server maps for each.
#[derive(Debug, Default)]
pub struct DetectionReport {
    pub found: Vec<Detection>,
}

#[derive(Debug)]
pub struct Detection {
    pub source: ImportSource,
    pub path: PathBuf,
    pub servers: BTreeMap<String, McpServerSpec>,
}

/// Walk every known source location rooted at `workspace_root` / `home`.
/// For each file that exists and parses, record a `Detection`. Parse
/// failures surface as a hard error (we don't want to silently drop a
/// config the user asked us to import).
pub fn detect_all(workspace_root: &Path, home: &Path) -> Result<DetectionReport> {
    let mut report = DetectionReport::default();
    for &source in ImportSource::all() {
        let Some(path) = source.default_path(workspace_root, home) else {
            continue;
        };
        if !path.exists() {
            continue;
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {} source at {}", source.slug(), path.display()))?;
        let servers = source.convert(&raw).with_context(|| {
            format!("converting {} source at {}", source.slug(), path.display())
        })?;
        report.found.push(Detection {
            source,
            path,
            servers,
        });
    }
    Ok(report)
}

/// A server entry as it appears in most JSON-based sources (VS Code,
/// Cursor, Claude Desktop, Kiro). Lenient about unknown fields so that
/// source-specific extensions (`envFile`, `disabled`, `autoApprove`,
/// `auth`, `sandbox`, `type: "sse"`, ...) don't blow up the import.
#[derive(Debug, Default, Deserialize)]
struct JsonServer {
    #[serde(default, rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl JsonServer {
    fn into_spec(self) -> Result<McpServerSpec> {
        let normalized = self.kind.as_deref().map(normalize_kind);
        let kind = build_server_kind(
            normalized,
            self.command,
            self.args,
            self.env,
            self.url,
            self.headers,
            StrictFields::Drop,
        )?;
        Ok(McpServerSpec { kind })
    }
}

/// Normalize source-specific transport strings onto the universal
/// `"stdio"` / `"http"` pair. Continue uses `"sse"` and `"streamable-http"`
/// for remote variants; we fold both onto `"http"` because the universal
/// schema only stores the destination URL.
fn normalize_kind(raw: &str) -> &str {
    match raw {
        "stdio" => "stdio",
        "http" | "sse" | "streamable-http" | "streamable_http" => "http",
        other => other,
    }
}

fn parse_json_map(raw: &str, key: &str) -> Result<BTreeMap<String, McpServerSpec>> {
    let value: serde_json::Value =
        serde_json::from_str(raw).context("source file is not valid JSON")?;
    let map = match value.get(key) {
        Some(serde_json::Value::Object(obj)) => obj.clone(),
        Some(_) => return Err(anyhow!("{key:?} must be a JSON object")),
        None => return Ok(BTreeMap::new()),
    };

    let mut out = BTreeMap::new();
    for (name, entry) in map {
        let raw: JsonServer = serde_json::from_value(entry)
            .with_context(|| format!("invalid server entry {name:?}"))?;
        let spec = raw
            .into_spec()
            .with_context(|| format!("invalid server entry {name:?}"))?;
        out.insert(name, spec);
    }
    Ok(out)
}

pub mod vscode {
    //! VS Code uses `.vscode/mcp.json` with a top-level `servers` object.
    //! The schema also allows a sibling `inputs: []` array which we ignore
    //! (it only matters for VS Code's own secret-prompt UI).
    use super::*;

    pub fn convert(raw: &str) -> Result<BTreeMap<String, McpServerSpec>> {
        parse_json_map(raw, "servers")
    }
}

pub mod cursor {
    //! Cursor's `~/.cursor/mcp.json` is closest to the universal schema:
    //! top-level `mcpServers`, stdio or http entries.
    use super::*;

    pub fn convert(raw: &str) -> Result<BTreeMap<String, McpServerSpec>> {
        parse_json_map(raw, "mcpServers")
    }
}

pub mod claude {
    //! Claude Desktop's `claude_desktop_config.json` nests `mcpServers`
    //! inside a larger config object. We ignore the sibling keys.
    use super::*;

    pub fn convert(raw: &str) -> Result<BTreeMap<String, McpServerSpec>> {
        parse_json_map(raw, "mcpServers")
    }
}

pub mod continue_ {
    //! Continue stores MCP servers inside its larger `~/.continue/config.json`.
    //! Older versions used an object under `mcpServers`; newer ones use an
    //! array of `{ name, type, command, args, env, url, ... }`. We handle
    //! both shapes. Other top-level keys (`models`, `contextProviders`, …)
    //! are ignored.
    use super::*;

    pub fn convert(raw: &str) -> Result<BTreeMap<String, McpServerSpec>> {
        let value: serde_json::Value =
            serde_json::from_str(raw).context("source file is not valid JSON")?;
        let mcp = match value.get("mcpServers") {
            Some(v) => v,
            None => return Ok(BTreeMap::new()),
        };

        let mut out = BTreeMap::new();
        match mcp {
            serde_json::Value::Object(obj) => {
                for (name, entry) in obj {
                    let raw: JsonServer = serde_json::from_value(entry.clone())
                        .with_context(|| format!("invalid server entry {name:?}"))?;
                    let spec = raw
                        .into_spec()
                        .with_context(|| format!("invalid server entry {name:?}"))?;
                    out.insert(name.clone(), spec);
                }
            }
            serde_json::Value::Array(arr) => {
                for (idx, entry) in arr.iter().enumerate() {
                    let obj = entry
                        .as_object()
                        .ok_or_else(|| anyhow!("Continue mcpServers[{idx}] must be an object"))?;
                    let name = obj
                        .get("name")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| {
                            anyhow!("Continue mcpServers[{idx}] missing required `name`")
                        })?
                        .to_string();
                    let raw: JsonServer = serde_json::from_value(entry.clone())
                        .with_context(|| format!("invalid server entry {name:?}"))?;
                    let spec = raw
                        .into_spec()
                        .with_context(|| format!("invalid server entry {name:?}"))?;
                    out.insert(name, spec);
                }
            }
            _ => return Err(anyhow!("Continue `mcpServers` must be an object or array")),
        }
        Ok(out)
    }
}

pub mod kiro {
    //! Kiro's `.kiro/mcp.json` mirrors Claude Desktop's shape: top-level
    //! `mcpServers`. Kiro-specific extensions (`disabled`, `autoApprove`,
    //! `disabledTools`) are silently dropped.
    use super::*;

    pub fn convert(raw: &str) -> Result<BTreeMap<String, McpServerSpec>> {
        parse_json_map(raw, "mcpServers")
    }
}

pub mod codex {
    //! Codex's `~/.codex/config.toml` stores MCP servers as
    //! `[mcp_servers.<name>]` TOML tables. Codex-specific extensions
    //! (`supports_parallel_tool_calls`, `default_tools_approval_mode`,
    //! tool-level `approval_mode`, `startup_timeout_ms`, `env_vars`, ...)
    //! have no universal-schema analogue and are silently dropped.
    use super::*;

    #[derive(Debug, Deserialize)]
    struct CodexConfig {
        #[serde(default)]
        mcp_servers: BTreeMap<String, CodexServer>,
    }

    #[derive(Debug, Deserialize)]
    struct CodexServer {
        #[serde(default)]
        command: Option<String>,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default)]
        url: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    }

    pub fn convert(raw: &str) -> Result<BTreeMap<String, McpServerSpec>> {
        let cfg: CodexConfig = toml::from_str(raw).context("source file is not valid TOML")?;
        let mut out = BTreeMap::new();
        for (name, server) in cfg.mcp_servers {
            let js = JsonServer {
                kind: None,
                command: server.command,
                args: server.args,
                env: server.env,
                url: server.url,
                headers: server.headers,
            };
            let spec = js
                .into_spec()
                .with_context(|| format!("invalid codex MCP server {name:?}"))?;
            out.insert(name, spec);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServerKind;

    fn assert_stdio<'a>(
        servers: &'a BTreeMap<String, McpServerSpec>,
        name: &str,
    ) -> (&'a String, &'a [String], &'a BTreeMap<String, String>) {
        let spec = servers
            .get(name)
            .unwrap_or_else(|| panic!("missing {name}"));
        match &spec.kind {
            ServerKind::Stdio { command, args, env } => (command, args.as_slice(), env),
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    fn assert_http<'a>(
        servers: &'a BTreeMap<String, McpServerSpec>,
        name: &str,
    ) -> (&'a String, &'a BTreeMap<String, String>) {
        let spec = servers
            .get(name)
            .unwrap_or_else(|| panic!("missing {name}"));
        match &spec.kind {
            ServerKind::Http { url, headers } => (url, headers),
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn slugs_round_trip() {
        for &source in ImportSource::all() {
            assert_eq!(ImportSource::from_slug(source.slug()), Some(source));
        }
        assert_eq!(ImportSource::from_slug("bogus"), None);
    }

    #[test]
    fn vscode_parses_servers_key_stdio_and_http() {
        let raw = r#"{
            "inputs": [{"id": "tok", "type": "promptString"}],
            "servers": {
                "playwright": {
                    "command": "npx",
                    "args": ["-y", "@microsoft/mcp-server-playwright"]
                },
                "github": {
                    "type": "http",
                    "url": "https://api.githubcopilot.com/mcp"
                }
            }
        }"#;
        let servers = vscode::convert(raw).unwrap();
        assert_eq!(servers.len(), 2);
        let (cmd, args, _env) = assert_stdio(&servers, "playwright");
        assert_eq!(cmd, "npx");
        assert_eq!(args, ["-y", "@microsoft/mcp-server-playwright"]);
        let (url, _h) = assert_http(&servers, "github");
        assert_eq!(url, "https://api.githubcopilot.com/mcp");
    }

    #[test]
    fn cursor_parses_mcp_servers_key_with_extensions() {
        // `envFile`, `auth`, `${env:...}` interpolation — Cursor-specific
        // extras that must not make the converter fail.
        let raw = r#"{
            "mcpServers": {
                "local": {
                    "type": "stdio",
                    "command": "python",
                    "args": ["mcp-server.py"],
                    "env": { "API_KEY": "${env:API_KEY}" },
                    "envFile": ".env"
                },
                "oauth": {
                    "url": "https://api.example.com/mcp",
                    "headers": { "Authorization": "Bearer tok" },
                    "auth": { "CLIENT_ID": "${env:ID}" }
                }
            }
        }"#;
        let servers = cursor::convert(raw).unwrap();
        assert_eq!(servers.len(), 2);
        let (cmd, args, env) = assert_stdio(&servers, "local");
        assert_eq!(cmd, "python");
        assert_eq!(args, ["mcp-server.py"]);
        assert_eq!(
            env.get("API_KEY").map(String::as_str),
            Some("${env:API_KEY}")
        );
        let (url, headers) = assert_http(&servers, "oauth");
        assert_eq!(url, "https://api.example.com/mcp");
        assert_eq!(
            headers.get("Authorization").map(String::as_str),
            Some("Bearer tok")
        );
    }

    #[test]
    fn claude_desktop_ignores_sibling_keys() {
        let raw = r#"{
            "globalShortcut": "Cmd+Shift+.",
            "mcpServers": {
                "filesystem": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
                }
            }
        }"#;
        let servers = claude::convert(raw).unwrap();
        let (cmd, args, _env) = assert_stdio(&servers, "filesystem");
        assert_eq!(cmd, "npx");
        assert_eq!(
            args,
            ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
        );
    }

    #[test]
    fn continue_object_form_and_sse_type_maps_to_http() {
        let raw = r#"{
            "models": [],
            "mcpServers": {
                "docs": {
                    "command": "docs-server",
                    "args": ["--port", "42"],
                    "env": { "LOG": "info" }
                },
                "remote": {
                    "type": "sse",
                    "url": "https://continue.example/mcp"
                }
            }
        }"#;
        let servers = continue_::convert(raw).unwrap();
        assert_eq!(servers.len(), 2);
        let (cmd, args, env) = assert_stdio(&servers, "docs");
        assert_eq!(cmd, "docs-server");
        assert_eq!(args, ["--port", "42"]);
        assert_eq!(env.get("LOG").map(String::as_str), Some("info"));
        let (url, _h) = assert_http(&servers, "remote");
        assert_eq!(url, "https://continue.example/mcp");
    }

    #[test]
    fn continue_array_form_uses_name_field() {
        let raw = r#"{
            "mcpServers": [
                {
                    "name": "sqlite",
                    "type": "stdio",
                    "command": "npx",
                    "args": ["-y", "mcp-sqlite", "/db.sqlite"]
                }
            ]
        }"#;
        let servers = continue_::convert(raw).unwrap();
        let (cmd, args, _env) = assert_stdio(&servers, "sqlite");
        assert_eq!(cmd, "npx");
        assert_eq!(args, ["-y", "mcp-sqlite", "/db.sqlite"]);
    }

    #[test]
    fn kiro_ignores_auto_approve_and_disabled() {
        // Both are Kiro-specific and must not reach the universal schema.
        let raw = r#"{
            "mcpServers": {
                "search": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-bravesearch"],
                    "env": { "BRAVE_API_KEY": "xxx" },
                    "disabled": false,
                    "autoApprove": ["search"]
                }
            }
        }"#;
        let servers = kiro::convert(raw).unwrap();
        let (cmd, args, env) = assert_stdio(&servers, "search");
        assert_eq!(cmd, "npx");
        assert_eq!(args, ["-y", "@modelcontextprotocol/server-bravesearch"]);
        assert_eq!(env.get("BRAVE_API_KEY").map(String::as_str), Some("xxx"));
    }

    #[test]
    fn codex_parses_toml_and_drops_extensions() {
        let raw = r#"
[mcp_servers.docs]
command = "docs-server"
args = ["--flag"]
supports_parallel_tool_calls = true
default_tools_approval_mode = "approve"

[mcp_servers.docs.env]
LOG_LEVEL = "info"

[mcp_servers.docs.tools.search]
approval_mode = "prompt"
"#;
        let servers = codex::convert(raw).unwrap();
        let (cmd, args, env) = assert_stdio(&servers, "docs");
        assert_eq!(cmd, "docs-server");
        assert_eq!(args, ["--flag"]);
        assert_eq!(env.get("LOG_LEVEL").map(String::as_str), Some("info"));
    }

    #[test]
    fn convert_dispatches_on_source() {
        let raw = r#"{ "mcpServers": { "x": { "command": "echo" } } }"#;
        let servers = ImportSource::Cursor.convert(raw).unwrap();
        let (cmd, _a, _e) = assert_stdio(&servers, "x");
        assert_eq!(cmd, "echo");
    }

    #[test]
    fn rejects_server_with_no_transport() {
        let raw = r#"{ "servers": { "bad": { "type": "stdio" } } }"#;
        let err = vscode::convert(raw).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("stdio MCP server missing `command`"),
            "error: {msg}"
        );
    }

    #[test]
    fn rejects_invalid_json() {
        let err = cursor::convert("not json").unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("valid JSON"), "error: {msg}");
    }

    #[test]
    fn missing_servers_key_yields_empty_map() {
        // Claude Desktop config without any mcpServers set still needs to
        // parse cleanly.
        let raw = r#"{ "globalShortcut": "Cmd+." }"#;
        let servers = claude::convert(raw).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn default_paths_are_rooted_correctly() {
        let ws = Path::new("/work/project");
        let home = Path::new("/home/user");
        assert_eq!(
            ImportSource::VsCode.default_path(ws, home).unwrap(),
            PathBuf::from("/work/project/.vscode/mcp.json")
        );
        assert_eq!(
            ImportSource::Cursor.default_path(ws, home).unwrap(),
            PathBuf::from("/home/user/.cursor/mcp.json")
        );
        assert_eq!(
            ImportSource::ClaudeDesktop.default_path(ws, home).unwrap(),
            PathBuf::from(
                "/home/user/Library/Application Support/Claude/claude_desktop_config.json"
            )
        );
        assert_eq!(
            ImportSource::Continue.default_path(ws, home).unwrap(),
            PathBuf::from("/home/user/.continue/config.json")
        );
        assert_eq!(
            ImportSource::Kiro.default_path(ws, home).unwrap(),
            PathBuf::from("/work/project/.kiro/mcp.json")
        );
        assert_eq!(
            ImportSource::Codex.default_path(ws, home).unwrap(),
            PathBuf::from("/work/project/.codex/config.toml")
        );
    }

    #[test]
    fn detect_all_walks_all_sources_that_exist() {
        let ws = tempfile::TempDir::new().unwrap();
        let home = tempfile::TempDir::new().unwrap();
        // VS Code present
        let vsdir = ws.path().join(".vscode");
        std::fs::create_dir_all(&vsdir).unwrap();
        std::fs::write(
            vsdir.join("mcp.json"),
            r#"{ "servers": { "vs": { "command": "vs-cmd" } } }"#,
        )
        .unwrap();
        // Cursor present
        let cdir = home.path().join(".cursor");
        std::fs::create_dir_all(&cdir).unwrap();
        std::fs::write(
            cdir.join("mcp.json"),
            r#"{ "mcpServers": { "cursor": { "command": "cur-cmd" } } }"#,
        )
        .unwrap();
        // Codex present
        let codex_dir = ws.path().join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            r#"
[mcp_servers.cx]
command = "cx-cmd"
"#,
        )
        .unwrap();

        let report = detect_all(ws.path(), home.path()).unwrap();
        let sources: Vec<_> = report.found.iter().map(|d| d.source).collect();
        assert_eq!(
            sources,
            vec![
                ImportSource::VsCode,
                ImportSource::Cursor,
                ImportSource::Codex
            ]
        );
        assert!(report.found[0].servers.contains_key("vs"));
        assert!(report.found[1].servers.contains_key("cursor"));
        assert!(report.found[2].servers.contains_key("cx"));
    }
}
