//! `~/.config/forge/config.toml`, the user's own defaults.

use std::path::Path;

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
        let problems = config.branches.validate("branches");
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
