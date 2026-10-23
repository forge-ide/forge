//! `[branches]`: how session branch names are built, in the user config and `forge.toml`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BRANCH_FORMAT: &str = "{type}/{slug}";
pub const DEFAULT_BRANCH_TYPES: [&str; 3] = ["feat", "fix", "bug"];

/// Branch naming. Unset keys fall back to the next layer: `forge.toml`, then the user config,
/// then the defaults.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BranchConfig {
    /// Template for new branch names. `{type}` is the branch type and `{slug}` comes from the
    /// session title. Default `{type}/{slug}`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Branch types a title may start with, as in `fix: retry loop`. The first is used when the
    /// title names none. Default `["feat", "fix", "bug"]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
}

/// Branch naming with every layer applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchRules {
    pub format: String,
    pub types: Vec<String>,
}

impl BranchConfig {
    pub fn is_empty(&self) -> bool {
        self.format.is_none() && self.types.is_none()
    }

    /// Problems with this layer, each prefixed with `at` (e.g. `branches`).
    pub fn validate(&self, at: &str) -> Vec<String> {
        let mut problems = Vec::new();
        if let Some(format) = &self.format {
            if !format.contains("{slug}") {
                problems.push(format!("{at}.format must contain {{slug}}"));
            }
            let mut rest = format.as_str();
            while let Some(start) = rest.find('{') {
                let Some(len) = rest[start..].find('}') else {
                    problems.push(format!("{at}.format has an unclosed {{"));
                    break;
                };
                let name = &rest[start + 1..start + len];
                if name != "type" && name != "slug" {
                    problems.push(format!(
                        "{at}.format: unknown placeholder {{{name}}}; use {{type}} or {{slug}}"
                    ));
                }
                rest = &rest[start + len + 1..];
            }
        }
        if let Some(types) = &self.types {
            if types.is_empty() {
                problems.push(format!("{at}.types must not be empty"));
            }
            for t in types {
                let valid = !t.is_empty()
                    && t.chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
                if !valid {
                    problems.push(format!(
                        "{at}.types: {t:?} must be lowercase letters, digits, and hyphens"
                    ));
                }
            }
        }
        problems
    }

    /// Apply `self` over `base`, then the defaults.
    pub fn resolve(&self, base: &BranchConfig) -> BranchRules {
        BranchRules {
            format: self
                .format
                .clone()
                .or_else(|| base.format.clone())
                .unwrap_or_else(|| DEFAULT_BRANCH_FORMAT.to_owned()),
            types: self
                .types
                .clone()
                .or_else(|| base.types.clone())
                .unwrap_or_else(|| DEFAULT_BRANCH_TYPES.map(str::to_owned).to_vec()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_resolve_per_key() {
        let user = BranchConfig {
            format: Some("jeff/{type}/{slug}".into()),
            types: None,
        };
        let project = BranchConfig {
            format: None,
            types: Some(vec!["feature".into()]),
        };
        assert_eq!(
            project.resolve(&user),
            BranchRules {
                format: "jeff/{type}/{slug}".into(),
                types: vec!["feature".into()],
            }
        );
        assert_eq!(
            BranchConfig::default().resolve(&BranchConfig::default()),
            BranchRules {
                format: "{type}/{slug}".into(),
                types: vec!["feat".into(), "fix".into(), "bug".into()],
            }
        );
    }

    #[test]
    fn validation_reports_bad_formats_and_types() {
        let config = BranchConfig {
            format: Some("{type}/{ticket}".into()),
            types: Some(vec!["Feat".into(), String::new()]),
        };
        let problems = config.validate("branches");
        assert_eq!(problems.len(), 4, "{problems:#?}");
        assert!(problems[0].contains("{slug}"));
        assert!(problems[1].contains("{ticket}"));

        let empty = BranchConfig {
            format: Some("{slug".into()),
            types: Some(vec![]),
        };
        assert_eq!(empty.validate("b").len(), 3);
        assert!(BranchConfig::default().validate("b").is_empty());
    }
}
