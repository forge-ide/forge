//! `forge.toml`, the project config at the project root.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Component, Path};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::branches::BranchConfig;
use crate::{ConfigError, read_toml};

pub const PROJECT_CONFIG_FILE: &str = "forge.toml";

/// Header written at the top of a generated `forge.toml`.
const GENERATED_HEADER: &str =
    "# Forge project config. Created by `forge project open`; edit freely.\n\n";

/// The `forge.toml` file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub project: ProjectSection,
    /// The repos in this project. v0.1 supports one.
    #[serde(rename = "repo")]
    pub repos: Vec<RepoConfig>,
    #[serde(default, skip_serializing_if = "Budgets::is_empty")]
    pub budgets: Budgets,
    #[serde(default)]
    pub sessions: SessionLimits,
    /// Overrides the user's branch naming for this project.
    #[serde(default, skip_serializing_if = "BranchConfig::is_empty")]
    pub branches: BranchConfig,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    pub name: String,
    /// Agent profile ID used when a session doesn't name one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RepoConfig {
    pub name: String,
    /// Relative to the project root, with forward slashes.
    pub path: String,
    pub base_branch: String,
}

/// Spending caps in USD. Unset means no cap.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    /// Pause a session that spends more than this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_daily_usd: Option<f64>,
}

impl Budgets {
    fn is_empty(&self) -> bool {
        self.session_usd.is_none() && self.project_daily_usd.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionLimits {
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
    #[serde(default = "default_retention_days")]
    pub worktree_retention_days: u32,
}

fn default_max_concurrent() -> u32 {
    5
}

fn default_retention_days() -> u32 {
    7
}

impl Default for SessionLimits {
    fn default() -> Self {
        Self {
            max_concurrent: default_max_concurrent(),
            worktree_retention_days: default_retention_days(),
        }
    }
}

impl ProjectConfig {
    /// A single-repo config for a project root that has no `forge.toml` yet.
    pub fn single_repo(name: &str, base_branch: &str) -> Self {
        Self {
            project: ProjectSection {
                name: name.to_owned(),
                default_agent: None,
            },
            repos: vec![RepoConfig {
                name: name.to_owned(),
                path: ".".to_owned(),
                base_branch: base_branch.to_owned(),
            }],
            budgets: Budgets::default(),
            sessions: SessionLimits::default(),
            branches: BranchConfig::default(),
        }
    }

    /// Load and validate `<root>/forge.toml`. `Ok(None)` if the file doesn't exist.
    pub fn load(root: &Path) -> Result<Option<Self>, ConfigError> {
        let path = root.join(PROJECT_CONFIG_FILE);
        let Some(config) = read_toml::<Self>(&path)? else {
            return Ok(None);
        };
        config
            .validate()
            .map_err(|problems| ConfigError::Invalid { path, problems })?;
        Ok(Some(config))
    }

    /// Write `<root>/forge.toml`, refusing to replace an existing file.
    pub fn write_new(&self, root: &Path) -> Result<(), ConfigError> {
        let path = root.join(PROJECT_CONFIG_FILE);
        let body = toml::to_string(self).map_err(|e| ConfigError::Invalid {
            path: path.clone(),
            problems: vec![e.to_string()],
        })?;
        let io_err = |source| ConfigError::Io {
            path: path.clone(),
            source,
        };
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(io_err)?;
        file.write_all(GENERATED_HEADER.as_bytes())
            .map_err(io_err)?;
        file.write_all(body.as_bytes()).map_err(io_err)?;
        Ok(())
    }

    /// Every problem found, so the user can fix them in one pass.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        if self.project.name.trim().is_empty() {
            problems.push("project.name must not be empty".to_owned());
        }
        if self.repos.is_empty() {
            problems.push("at least one [[repo]] is required".to_owned());
        }
        let mut names = HashSet::new();
        for (i, repo) in self.repos.iter().enumerate() {
            let at = format!("repo[{i}]");
            if repo.name.is_empty() || repo.name.contains(['/', '\\']) {
                problems.push(format!(
                    "{at}.name must be non-empty and contain no slashes"
                ));
            } else if !names.insert(repo.name.as_str()) {
                problems.push(format!("{at}.name {:?} is used twice", repo.name));
            }
            if !is_contained_relative(&repo.path) {
                problems.push(format!(
                    "{at}.path {:?} must be relative to the project root and stay inside it",
                    repo.path
                ));
            }
            if repo.base_branch.trim().is_empty() {
                problems.push(format!("{at}.base_branch must not be empty"));
            }
        }
        for (key, value) in [
            ("budgets.session_usd", self.budgets.session_usd),
            ("budgets.project_daily_usd", self.budgets.project_daily_usd),
        ] {
            if value.is_some_and(|v| !v.is_finite() || v < 0.0) {
                problems.push(format!("{key} must be zero or more"));
            }
        }
        if self.sessions.max_concurrent == 0 {
            problems.push("sessions.max_concurrent must be at least 1".to_owned());
        }
        problems.extend(self.branches.validate("branches"));
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

/// A forward-slash relative path that never climbs above its base.
fn is_contained_relative(path: &str) -> bool {
    if path.is_empty() || path.contains('\\') {
        return false;
    }
    let mut depth = 0i32;
    for component in Path::new(path).components() {
        match component {
            Component::CurDir => {}
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPEC_EXAMPLE: &str = r#"
[project]
name = "forge"
default_agent = "claude"

[[repo]]
name = "forge"
path = "."
base_branch = "main"

[budgets]
session_usd = 10.0
project_daily_usd = 50.0

[sessions]
max_concurrent = 5
worktree_retention_days = 7
"#;

    fn write(dir: &Path, body: &str) {
        std::fs::write(dir.join(PROJECT_CONFIG_FILE), body).unwrap();
    }

    #[test]
    fn loads_the_spec_example() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), SPEC_EXAMPLE);
        let config = ProjectConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(config.project.default_agent.as_deref(), Some("claude"));
        assert_eq!(config.repos[0].base_branch, "main");
        assert_eq!(config.budgets.session_usd, Some(10.0));
    }

    #[test]
    fn missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ProjectConfig::load(dir.path()).unwrap(), None);
    }

    #[test]
    fn unknown_keys_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            &SPEC_EXAMPLE.replace("max_concurrent", "max_concurent"),
        );
        let err = ProjectConfig::load(dir.path()).unwrap_err();
        assert!(matches!(err, ConfigError::Parse { .. }), "{err}");
        assert!(err.to_string().contains("max_concurent"), "{err}");
    }

    #[test]
    fn sections_default() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "[project]\nname = \"x\"\n[[repo]]\nname = \"x\"\npath = \".\"\nbase_branch = \"main\"\n",
        );
        let config = ProjectConfig::load(dir.path()).unwrap().unwrap();
        assert_eq!(config.sessions, SessionLimits::default());
        assert_eq!(config.budgets, Budgets::default());
    }

    #[test]
    fn validation_reports_every_problem() {
        let mut config = ProjectConfig::single_repo("x", "main");
        config.project.name = " ".into();
        config.repos.push(config.repos[0].clone());
        config.repos[1].path = "../elsewhere".into();
        config.sessions.max_concurrent = 0;
        config.budgets.session_usd = Some(-1.0);
        config.branches.types = Some(vec![]);
        let problems = config.validate().unwrap_err();
        assert_eq!(problems.len(), 6, "{problems:#?}");
    }

    #[test]
    fn repo_paths_must_stay_inside() {
        assert!(is_contained_relative("."));
        assert!(is_contained_relative("api"));
        assert!(is_contained_relative("a/../b"));
        assert!(!is_contained_relative("/abs"));
        assert!(!is_contained_relative("a/../../b"));
        assert!(!is_contained_relative(""));
        assert!(!is_contained_relative("a\\b"));
    }

    #[test]
    fn written_config_round_trips_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let config = ProjectConfig::single_repo("demo", "trunk");
        config.write_new(dir.path()).unwrap();
        let text = std::fs::read_to_string(dir.path().join(PROJECT_CONFIG_FILE)).unwrap();
        assert!(text.starts_with("# Forge project config"));
        assert!(!text.contains("[budgets]"));
        assert_eq!(
            ProjectConfig::load(dir.path()).unwrap(),
            Some(config.clone())
        );
        assert!(matches!(
            config.write_new(dir.path()),
            Err(ConfigError::Io { .. })
        ));
    }
}
