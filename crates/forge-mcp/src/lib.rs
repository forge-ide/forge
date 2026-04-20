//! MCP (Model Context Protocol) configuration parser.
//!
//! This crate owns the universal `.mcp.json` schema — the `mcpServers` object
//! keyed by name with `command` / `args` / `env` for stdio transports and
//! `url` / `headers` for HTTP transports. Transports themselves (F-128, F-129)
//! and the lifecycle manager (F-130) live in follow-up crates; this one just
//! produces typed [`McpServerSpec`] values from config files.

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

/// Parsed declaration of a single MCP server, as it appears under
/// `mcpServers.<name>` in a `.mcp.json` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerSpec {
    pub kind: ServerKind,
}

/// Transport-specific shape of an MCP server declaration.
///
/// Discriminated by either an explicit `"type"` field (universal proposal) or,
/// when `type` is absent, by whether `command` (stdio) or `url` (http) is
/// present — matching the real-world configs shipped by Claude Desktop,
/// Cursor, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerKind {
    Stdio {
        command: String,
        args: Vec<String>,
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        headers: BTreeMap<String, String>,
    },
}

/// Flat on-disk representation. We keep every field optional so that the
/// `#[serde(deny_unknown_fields)]` guard runs first and rejects typos, then
/// [`McpServerSpec::try_from`] decides which `ServerKind` variant to emit.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawServer {
    #[serde(rename = "type")]
    kind: Option<String>,
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    url: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

impl TryFrom<RawServer> for McpServerSpec {
    type Error = anyhow::Error;

    fn try_from(raw: RawServer) -> Result<Self> {
        let resolved = resolve_transport(raw.kind.as_deref(), &raw)?;

        let kind = match resolved {
            Transport::Stdio => {
                if raw.url.is_some() || !raw.headers.is_empty() {
                    return Err(anyhow!("stdio MCP server must not set `url` or `headers`"));
                }
                let command = raw
                    .command
                    .ok_or_else(|| anyhow!("stdio MCP server missing `command`"))?;
                ServerKind::Stdio {
                    command,
                    args: raw.args,
                    env: raw.env,
                }
            }
            Transport::Http => {
                if raw.command.is_some() || !raw.args.is_empty() || !raw.env.is_empty() {
                    return Err(anyhow!(
                        "http MCP server must not set `command`, `args`, or `env`"
                    ));
                }
                let url = raw
                    .url
                    .ok_or_else(|| anyhow!("http MCP server missing `url`"))?;
                ServerKind::Http {
                    url,
                    headers: raw.headers,
                }
            }
        };

        Ok(McpServerSpec { kind })
    }
}

enum Transport {
    Stdio,
    Http,
}

fn resolve_transport(declared: Option<&str>, raw: &RawServer) -> Result<Transport> {
    match declared {
        Some("stdio") => Ok(Transport::Stdio),
        Some("http") => Ok(Transport::Http),
        Some(other) => Err(anyhow!(
            "unknown MCP server type {other:?}; expected \"stdio\" or \"http\""
        )),
        None => match (raw.command.is_some(), raw.url.is_some()) {
            (true, false) => Ok(Transport::Stdio),
            (false, true) => Ok(Transport::Http),
            (true, true) => Err(anyhow!(
                "MCP server declares both `command` and `url`; set `type` explicitly"
            )),
            (false, false) => Err(anyhow!(
                "MCP server missing transport fields: need `command` (stdio) or `url` (http)"
            )),
        },
    }
}

pub mod config {
    //! File-system loaders for workspace- and user-scoped `.mcp.json`.

    use super::*;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct McpConfigFile {
        #[serde(rename = "mcpServers", default)]
        servers: BTreeMap<String, RawServer>,
    }

    /// Load `<workspace_root>/.mcp.json`. Missing file yields an empty map; a
    /// present-but-malformed file yields an error with the path in context.
    pub fn load_workspace(workspace_root: &Path) -> Result<BTreeMap<String, McpServerSpec>> {
        read_config_file(&workspace_root.join(".mcp.json"))
    }

    /// Load the user-scope `~/.mcp.json` per the architecture doc's universal
    /// home-directory convention (matches Claude Desktop and the MCP universal
    /// proposal). Missing file yields an empty map.
    pub fn load_user() -> Result<BTreeMap<String, McpServerSpec>> {
        let home =
            dirs::home_dir().ok_or_else(|| anyhow!("could not resolve user home directory"))?;
        read_config_file(&home.join(".mcp.json"))
    }

    /// Test seam: same as [`load_user`] but with an explicit home directory
    /// (so tests can point at a tempdir).
    pub fn load_user_from(home_dir: &Path) -> Result<BTreeMap<String, McpServerSpec>> {
        read_config_file(&home_dir.join(".mcp.json"))
    }

    /// Merge user- and workspace-scope servers, with workspace entries
    /// overriding user entries on name collisions. Callers that only need one
    /// scope can invoke [`load_workspace`] or [`load_user`] directly.
    pub fn load_merged(
        workspace_root: &Path,
        user_config_dir: &Path,
    ) -> Result<BTreeMap<String, McpServerSpec>> {
        let mut merged = load_user_from(user_config_dir)?;
        for (name, spec) in load_workspace(workspace_root)? {
            merged.insert(name, spec);
        }
        Ok(merged)
    }

    fn read_config_file(path: &PathBuf) -> Result<BTreeMap<String, McpServerSpec>> {
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let raw =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let file: McpConfigFile =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;

        let mut out = BTreeMap::new();
        for (name, raw_server) in file.servers {
            let spec = McpServerSpec::try_from(raw_server)
                .with_context(|| format!("invalid MCP server {name:?} in {}", path.display()))?;
            out.insert(name, spec);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    #[test]
    fn parses_stdio_server_inferred_from_command() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            r#"{
                "mcpServers": {
                    "github": {
                        "command": "npx",
                        "args": ["-y", "@modelcontextprotocol/server-github"],
                        "env": { "GITHUB_TOKEN": "ghp_xxx" }
                    }
                }
            }"#,
        );

        let servers = config::load_workspace(tmp.path()).unwrap();
        let spec = servers.get("github").expect("github entry");
        match &spec.kind {
            ServerKind::Stdio { command, args, env } => {
                assert_eq!(command, "npx");
                assert_eq!(
                    args,
                    &vec![
                        "-y".to_string(),
                        "@modelcontextprotocol/server-github".into()
                    ]
                );
                assert_eq!(env.get("GITHUB_TOKEN").map(String::as_str), Some("ghp_xxx"));
            }
            other => panic!("expected Stdio, got {other:?}"),
        }
    }

    #[test]
    fn parses_http_server_with_explicit_type() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            r#"{
                "mcpServers": {
                    "remote": {
                        "type": "http",
                        "url": "https://mcp.example.com/api",
                        "headers": { "Authorization": "Bearer token" }
                    }
                }
            }"#,
        );

        let servers = config::load_workspace(tmp.path()).unwrap();
        let spec = servers.get("remote").expect("remote entry");
        match &spec.kind {
            ServerKind::Http { url, headers } => {
                assert_eq!(url, "https://mcp.example.com/api");
                assert_eq!(
                    headers.get("Authorization").map(String::as_str),
                    Some("Bearer token")
                );
            }
            other => panic!("expected Http, got {other:?}"),
        }
    }

    #[test]
    fn rejects_unknown_fields() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            r#"{
                "mcpServers": {
                    "bad": {
                        "command": "foo",
                        "totally_unknown_field": true
                    }
                }
            }"#,
        );

        let err = config::load_workspace(tmp.path()).expect_err("unknown field must reject");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("totally_unknown_field"),
            "error should name offending field: {msg}"
        );
    }

    #[test]
    fn rejects_top_level_unknown_fields() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            r#"{
                "mcpServers": {},
                "extra": 1
            }"#,
        );

        let err =
            config::load_workspace(tmp.path()).expect_err("top-level unknown field must reject");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("extra"),
            "error should name offending field: {msg}"
        );
    }

    #[test]
    fn missing_workspace_file_is_empty_map() {
        let tmp = TempDir::new().unwrap();
        let servers = config::load_workspace(tmp.path()).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn missing_user_file_is_empty_map() {
        let tmp = TempDir::new().unwrap();
        let servers = config::load_user_from(tmp.path()).unwrap();
        assert!(servers.is_empty());
    }

    #[test]
    fn workspace_overrides_user_on_name_collision() {
        let workspace = TempDir::new().unwrap();
        let user_cfg = TempDir::new().unwrap();

        write(
            &user_cfg.path().join(".mcp.json"),
            r#"{
                "mcpServers": {
                    "shared": { "command": "user-binary" },
                    "user-only": { "command": "only-here" }
                }
            }"#,
        );
        write(
            &workspace.path().join(".mcp.json"),
            r#"{
                "mcpServers": {
                    "shared": { "command": "workspace-binary" },
                    "ws-only": { "command": "ws-bin" }
                }
            }"#,
        );

        let merged = config::load_merged(workspace.path(), user_cfg.path()).unwrap();
        assert_eq!(merged.len(), 3);

        let shared = merged.get("shared").expect("shared wins");
        match &shared.kind {
            ServerKind::Stdio { command, .. } => assert_eq!(command, "workspace-binary"),
            other => panic!("expected Stdio, got {other:?}"),
        }
        assert!(merged.contains_key("user-only"));
        assert!(merged.contains_key("ws-only"));
    }

    #[test]
    fn rejects_server_with_neither_command_nor_url() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            r#"{ "mcpServers": { "bad": {} } }"#,
        );

        let err = config::load_workspace(tmp.path()).expect_err("must reject");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("transport"),
            "error should explain missing transport: {msg}"
        );
    }

    #[test]
    fn rejects_unknown_type_value() {
        let tmp = TempDir::new().unwrap();
        write(
            &tmp.path().join(".mcp.json"),
            r#"{
                "mcpServers": {
                    "bad": { "type": "carrier-pigeon", "command": "foo" }
                }
            }"#,
        );

        let err = config::load_workspace(tmp.path()).expect_err("must reject");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("carrier-pigeon"),
            "error should name the bad type: {msg}"
        );
    }
}
