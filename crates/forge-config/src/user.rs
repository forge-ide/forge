//! `~/.config/forge/config.toml`, the user's own defaults.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::branches::BranchConfig;
use crate::{ConfigError, read_toml};

/// The user config file. Every key is optional; a missing file means all defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    /// Agent profile ID used when neither the session nor the project names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
    #[serde(default)]
    pub ui: UiConfig,
    /// Branch naming for new sessions; `forge.toml` can override each key.
    #[serde(default, skip_serializing_if = "BranchConfig::is_empty")]
    pub branches: BranchConfig,
    #[serde(default, skip_serializing_if = "SandboxConfig::is_empty")]
    pub sandbox: SandboxConfig,
}

/// What sandboxed sessions may see beyond the defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SandboxConfig {
    /// Absolute paths visible read-only inside every sandbox, such as an agent's install dir
    /// or an extra toolchain. Lives in the user config, never in a repo's files, so a cloned
    /// repo can't widen its own sandbox. May not be `/`, the home dir itself, or contain
    /// Forge's own data or runtime dirs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_only: Vec<PathBuf>,
}

impl SandboxConfig {
    pub fn is_empty(&self) -> bool {
        self.read_only.is_empty()
    }

    fn validate(&self) -> Vec<String> {
        self.read_only
            .iter()
            .filter_map(|p| {
                if !p.is_absolute() {
                    Some(format!(
                        "sandbox.read_only: {} is not an absolute path",
                        p.display()
                    ))
                } else if p.parent().is_none() {
                    Some("sandbox.read_only: / would expose the whole machine".to_owned())
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UiConfig {
    #[serde(default)]
    pub theme: Theme,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    /// Follow the OS setting.
    #[default]
    System,
    Light,
    Dark,
}

impl UserConfig {
    /// Load and validate the file at `path`; a missing file gives the defaults.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let config: Self = read_toml(path)?.unwrap_or_default();
        let mut problems = config.branches.validate("branches");
        problems.extend(config.sandbox.validate());
        if !problems.is_empty() {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                problems,
            });
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            UserConfig::load(&dir.path().join("config.toml")).unwrap(),
            UserConfig::default()
        );
    }

    #[test]
    fn loads_and_rejects_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "default_agent = \"codex\"\n[ui]\ntheme = \"dark\"\n").unwrap();
        let config = UserConfig::load(&path).unwrap();
        assert_eq!(config.default_agent.as_deref(), Some("codex"));
        assert_eq!(config.ui.theme, Theme::Dark);

        std::fs::write(&path, "[ui]\ntheme = \"dark\"\nfont = \"x\"\n").unwrap();
        assert!(matches!(
            UserConfig::load(&path),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn sandbox_read_only_paths_must_be_absolute_and_narrow() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[sandbox]\nread_only = [\"/opt/agents\"]\n").unwrap();
        let config = UserConfig::load(&path).unwrap();
        assert_eq!(config.sandbox.read_only, [PathBuf::from("/opt/agents")]);

        for bad in ["\"relative/dir\"", "\"/\""] {
            std::fs::write(&path, format!("[sandbox]\nread_only = [{bad}]\n")).unwrap();
            assert!(
                matches!(UserConfig::load(&path), Err(ConfigError::Invalid { .. })),
                "{bad}"
            );
        }
        std::fs::write(&path, "[sandbox]\nread_write = []\n").unwrap();
        assert!(matches!(
            UserConfig::load(&path),
            Err(ConfigError::Parse { .. })
        ));
    }

    #[test]
    fn validates_branch_naming() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "[branches]\nformat = \"me/{type}/{slug}\"\n").unwrap();
        let config = UserConfig::load(&path).unwrap();
        assert_eq!(config.branches.format.as_deref(), Some("me/{type}/{slug}"));

        std::fs::write(&path, "[branches]\nformat = \"{type}\"\n").unwrap();
        assert!(matches!(
            UserConfig::load(&path),
            Err(ConfigError::Invalid { .. })
        ));
    }
}
