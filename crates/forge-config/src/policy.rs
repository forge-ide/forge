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

/// The rule lists in a policy file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleListName {
    Deny,
    Ask,
    Allow,
}

/// Append `rule` to `list` in the policy file at `path`, creating the file if needed and
/// keeping its text (comments included) as it is. The result must still load, or the file is
/// left alone. The file is replaced in one step, so readers never see half of it, and appends
/// in this process run one at a time, so none is lost.
pub fn append_rule(path: &Path, list: RuleListName, rule: &PolicyRule) -> Result<(), ConfigError> {
    static APPENDING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _one_at_a_time = APPENDING.lock().unwrap_or_else(|p| p.into_inner());
    #[derive(Serialize)]
    struct One<'a> {
        #[serde(skip_serializing_if = "Option::is_none")]
        deny: Option<[&'a PolicyRule; 1]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        ask: Option<[&'a PolicyRule; 1]>,
        #[serde(skip_serializing_if = "Option::is_none")]
        allow: Option<[&'a PolicyRule; 1]>,
    }
    let one = One {
        deny: (list == RuleListName::Deny).then_some([rule]),
        ask: (list == RuleListName::Ask).then_some([rule]),
        allow: (list == RuleListName::Allow).then_some([rule]),
    };
    let io = |source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    };
    let snippet = toml::to_string(&one).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        message: e.to_string(),
    })?;
    let mut text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io(e)),
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&snippet);
    let config: PolicyConfig = toml::from_str(&text).map_err(|e| ConfigError::Parse {
        path: path.to_path_buf(),
        message: format!("adding a rule would break the file: {e}"),
    })?;
    let problems = config.validate();
    if !problems.is_empty() {
        return Err(ConfigError::Invalid {
            path: path.to_path_buf(),
            problems,
        });
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned());
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        name.unwrap_or_default(),
        std::process::id()
    ));
    std::fs::write(&tmp, &text).map_err(io)?;
    std::fs::rename(&tmp, path).map_err(io)
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

    fn exact(exec: &str) -> PolicyRule {
        PolicyRule {
            exec: vec![exec.into()],
            reason: Some("remembered".into()),
            ..PolicyRule::default()
        }
    }

    #[test]
    fn appended_rules_keep_the_file_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".forge/policy.toml");
        append_rule(&path, RuleListName::Allow, &exact(r#"echo "a\*b""#)).unwrap();
        std::fs::write(
            &path,
            format!(
                "# team rules\n{}[protected]\npaths = [\"x\"]",
                std::fs::read_to_string(&path).unwrap()
            ),
        )
        .unwrap();
        append_rule(&path, RuleListName::Deny, &exact("rm -rf x")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# team rules\n[[allow]]\n"), "{text}");
        let config = PolicyConfig::load(&path).unwrap();
        assert_eq!(config.allow, [exact(r#"echo "a\*b""#)]);
        assert_eq!(config.deny, [exact("rm -rf x")]);
        assert_eq!(config.protected.paths, ["x"]);
    }

    #[test]
    fn concurrent_appends_all_land() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        std::thread::scope(|scope| {
            for i in 0..8 {
                let path = &path;
                scope.spawn(move || {
                    append_rule(path, RuleListName::Deny, &exact(&format!("rm {i}"))).unwrap()
                });
            }
        });
        assert_eq!(PolicyConfig::load(&path).unwrap().deny.len(), 8);
    }

    #[test]
    fn a_rule_that_would_break_the_file_is_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("policy.toml");
        let inline = "allow = [{ exec = [\"ls\"] }]\n";
        std::fs::write(&path, inline).unwrap();
        assert!(append_rule(&path, RuleListName::Allow, &exact("pwd")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), inline);
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
