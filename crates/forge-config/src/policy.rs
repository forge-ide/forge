//! `policy.toml`: deny, ask, and allow rules plus protected paths. The same file format serves
//! every layer: org-managed, user (`~/.config/forge/policy.toml`), and project
//! (`<root>/.forge/policy.toml`).

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::agent::ProfileMode;
use crate::{ConfigError, read_toml};

/// The project layer's file, relative to the project root.
pub const PROJECT_POLICY_FILE: &str = ".forge/policy.toml";

/// A policy file. Every key is optional; a missing file has no rules.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyConfig {
    /// Hard denies: no later rule, hook, or mode can allow what these match.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny: Vec<PolicyRule>,
    /// Actions that always need a human, even where an allow rule matches.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ask: Vec<PolicyRule>,
    /// Actions that run without asking, unless a deny or ask rule, guard, or hook says
    /// otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow: Vec<PolicyRule>,
    #[serde(default, skip_serializing_if = "ProtectedConfig::is_empty")]
    pub protected: ProtectedConfig,
}

/// One rule. It matches an action if any of its patterns does, in the modes it lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PolicyRule {
    /// Globs over the command's space-joined argv; `*` matches any run of characters and
    /// `\*` a literal `*`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exec: Vec<String>,
    /// Gitignore-style globs over repo-relative paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edit: Vec<String>,
    /// Shown to the human and the agent when the rule decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Modes the rule applies in; every mode when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<Vec<ProfileMode>>,
}

/// Paths agents may not edit.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProtectedConfig {
    /// Gitignore-style globs over repo-relative paths, added to the built-in list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub paths: Vec<String>,
}

impl ProtectedConfig {
    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }
}

impl PolicyConfig {
    /// Load and validate the file at `path`; a missing file has no rules.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let config: Self = read_toml(path)?.unwrap_or_default();
        let problems = config.validate();
        if !problems.is_empty() {
            return Err(ConfigError::Invalid {
                path: path.to_path_buf(),
                problems,
            });
        }
        Ok(config)
    }

    /// Every problem with the rules' shape. Glob syntax is checked when the policy engine
    /// compiles them.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let lists = [
            ("deny", &self.deny),
            ("ask", &self.ask),
            ("allow", &self.allow),
        ];
        for (list, rules) in lists {
            for (i, rule) in rules.iter().enumerate() {
                let at = format!("{list}[{i}]");
                if rule.exec.is_empty() && rule.edit.is_empty() {
                    problems.push(format!("{at}: needs at least one exec or edit pattern"));
                }
                if rule.modes.as_ref().is_some_and(Vec::is_empty) {
                    problems.push(format!(
                        "{at}: modes is empty, so the rule never applies; remove it to apply \
                         in every mode"
                    ));
                }
                if rule.exec.iter().any(|p| p.trim().is_empty()) {
                    problems.push(format!("{at}: exec patterns must not be empty"));
                }
                problems.extend(path_problems(&at, "edit", &rule.edit));
            }
        }
        problems.extend(path_problems("protected", "paths", &self.protected.paths));
        problems
    }
}

/// Edit patterns are repo-relative: a leading `/` anchors to the repo root, as in gitignore.
/// No empty patterns, bare `/`, or `..`.
fn path_problems(at: &str, key: &str, patterns: &[String]) -> Vec<String> {
    patterns
        .iter()
        .filter_map(|p| {
            if p.trim().is_empty() {
                Some(format!("{at}: {key} patterns must not be empty"))
            } else if p.trim_end_matches('/').is_empty() {
                Some(format!("{at}: {key} pattern {p:?} names no path"))
            } else if p.split('/').any(|seg| seg == "..") {
                Some(format!(
                    "{at}: {key} pattern {p:?} uses '..'; patterns are repo-relative"
                ))
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(text: &str) -> Result<PolicyConfig, ConfigError> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        std::fs::write(&path, text).unwrap();
        PolicyConfig::load(&path)
    }

    #[test]
    fn missing_file_has_no_rules() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            PolicyConfig::load(&dir.path().join("policy.toml")).unwrap(),
            PolicyConfig::default()
        );
    }

    #[test]
    fn loads_the_spec_example() {
        let config = load(
            r#"
[[deny]]
exec = ["git push --force*", "git reset --hard origin*"]
reason = "History-rewriting"

[[ask]]
edit = ["**/migrations/**", ".github/workflows/**"]
reason = "Schema and CI changes need a human"

[[allow]]
exec = ["cargo test*", "cargo clippy*", "pnpm test*", "just *"]
modes = ["accept-edits", "auto"]

[protected]
paths = [".env*", "**/*.pem", "Cargo.lock"]
"#,
        )
        .unwrap();
        assert_eq!(config.deny[0].exec.len(), 2);
        assert_eq!(config.deny[0].reason.as_deref(), Some("History-rewriting"));
        assert_eq!(config.ask[0].edit[1], ".github/workflows/**");
        assert_eq!(
            config.allow[0].modes,
            Some(vec![ProfileMode::AcceptEdits, ProfileMode::Auto])
        );
        assert_eq!(config.protected.paths, [".env*", "**/*.pem", "Cargo.lock"]);
    }

    #[test]
    fn rejects_unknown_keys_and_modes() {
        for text in [
            "[[deny]]\nexec = [\"x\"]\nnetwork = [\"a.com\"]\n",
            "[[allow]]\nexec = [\"x\"]\nmodes = [\"yolo\"]\n",
            "[[permit]]\nexec = [\"x\"]\n",
            "[protected]\nfiles = [\"x\"]\n",
        ] {
            assert!(
                matches!(load(text), Err(ConfigError::Parse { .. })),
                "{text}"
            );
        }
    }

    #[test]
    fn rejects_rules_that_cannot_match() {
        for text in [
            "[[deny]]\nreason = \"no patterns\"\n",
            "[[allow]]\nexec = [\"x\"]\nmodes = []\n",
            "[[ask]]\nexec = [\" \"]\n",
            "[[ask]]\nedit = [\"\"]\n",
            "[[deny]]\nedit = [\"../outside/**\"]\n",
            "[[deny]]\nedit = [\"/\"]\n",
            "[protected]\npaths = [\"src/../../x\"]\n",
        ] {
            assert!(
                matches!(load(text), Err(ConfigError::Invalid { .. })),
                "{text}"
            );
        }
    }
}
