//! `.forge/agents/<id>.toml`, an agent profile: which agent to run and how.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ConfigError, read_toml};

/// Directory under the project root that holds agent profiles.
pub const AGENTS_DIR: &str = ".forge/agents";

/// An agent profile file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AgentProfileConfig {
    /// Adapter registry key, e.g. `claude-code`.
    pub agent: String,
    /// Command that starts the agent speaking ACP on stdio; overrides the registry default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Vec<String>>,
    /// Model to request, if the agent supports choosing one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Default mode for sessions with this profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<ProfileMode>,
    /// Default sandbox tier for sessions with this profile. Default `standard`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<ProfileSandbox>,
    /// Allow sessions with this profile to run without a sandbox. Off unless set here.
    #[serde(default)]
    pub allow_unsandboxed: bool,
    /// Keyring entries to inject through the egress proxy, never into the environment.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_secrets: Vec<String>,
    #[serde(default)]
    pub network: NetworkConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileMode {
    Ask,
    AcceptEdits,
    Auto,
    FullAccess,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ProfileSandbox {
    Light,
    #[default]
    Standard,
    Container,
    Unsandboxed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NetworkConfig {
    /// Hosts the agent may reach through the egress proxy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<String>,
}

/// A profile ID that can't name a file under `.forge/agents/`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("agent profile id {0:?} must be letters, digits, '-', or '_'")]
pub struct InvalidProfileId(pub String);

impl AgentProfileConfig {
    /// The file for profile `id` under `root`. Rejects IDs that could escape the directory.
    pub fn path(root: &Path, id: &str) -> Result<PathBuf, InvalidProfileId> {
        let valid = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err(InvalidProfileId(id.to_owned()));
        }
        Ok(root.join(AGENTS_DIR).join(format!("{id}.toml")))
    }

    /// Load and validate the profile at `path`. `Ok(None)` if it doesn't exist.
    pub fn load(path: &Path) -> Result<Option<Self>, ConfigError> {
        let Some(config) = read_toml::<Self>(path)? else {
            return Ok(None);
        };
        let mut problems = Vec::new();
        if config.agent.trim().is_empty() {
            problems.push("agent must not be empty".to_owned());
        }
        if config.command.as_ref().is_some_and(|c| c.is_empty()) {
            problems.push("command must name a program".to_owned());
        }
        if !problems.is_empty() {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                problems,
            });
        }
        Ok(Some(config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_EXAMPLE: &str = r#"
agent = "claude-code"
command = ["claude-code-acp"]
model = "claude-opus-5"
mode = "accept-edits"
sandbox = "standard"
allow_unsandboxed = false
env_secrets = ["ANTHROPIC_API_KEY"]

[network]
allow = ["api.anthropic.com", "crates.io"]
"#;

    #[test]
    fn loads_the_spec_example() {
        let dir = tempfile::tempdir().unwrap();
        let path = AgentProfileConfig::path(dir.path(), "claude").unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, SPEC_EXAMPLE).unwrap();
        let profile = AgentProfileConfig::load(&path).unwrap().unwrap();
        assert_eq!(profile.agent, "claude-code");
        assert_eq!(profile.mode, Some(ProfileMode::AcceptEdits));
        assert_eq!(profile.sandbox, Some(ProfileSandbox::Standard));
        assert!(!profile.allow_unsandboxed);
        assert_eq!(profile.network.allow.len(), 2);
    }

    #[test]
    fn unsandboxed_is_off_by_default_and_unknown_keys_fail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.toml");
        std::fs::write(&path, "agent = \"x\"\n").unwrap();
        let profile = AgentProfileConfig::load(&path).unwrap().unwrap();
        assert!(!profile.allow_unsandboxed);
        assert_eq!(profile.sandbox, None);

        std::fs::write(&path, "agent = \"x\"\nallow_unsandbox = true\n").unwrap();
        assert!(matches!(
            AgentProfileConfig::load(&path),
            Err(ConfigError::Parse { .. })
        ));
        std::fs::write(&path, "agent = \"x\"\ncommand = []\n").unwrap();
        assert!(matches!(
            AgentProfileConfig::load(&path),
            Err(ConfigError::Invalid { .. })
        ));
        assert_eq!(
            AgentProfileConfig::load(&dir.path().join("missing.toml")).unwrap(),
            None
        );
    }

    #[test]
    fn ids_cannot_escape_the_agents_dir() {
        let root = Path::new("/p");
        assert_eq!(
            AgentProfileConfig::path(root, "claude_2-x").unwrap(),
            Path::new("/p/.forge/agents/claude_2-x.toml")
        );
        for bad in ["", "../x", "a/b", "a.b", "a b"] {
            assert!(AgentProfileConfig::path(root, bad).is_err(), "{bad:?}");
        }
    }
}
