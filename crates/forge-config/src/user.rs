//! `~/.config/forge/config.toml`, the user's own defaults.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
        Ok(read_toml(path)?.unwrap_or_default())
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
}
