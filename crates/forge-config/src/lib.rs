//! Project, workspace, and agent-profile config with layering.
//!
//! Every config file has a Rust type here; `just gen` writes its JSON Schema to `docs/schemas/`.
//! Unknown keys are errors so typos surface immediately.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

pub mod agent;
pub mod branches;
pub mod paths;
pub mod policy;
pub mod project;
pub mod user;

pub use agent::{AGENTS_DIR, AgentProfileConfig, InvalidProfileId, ProfileMode, ProfileSandbox};
pub use branches::{BranchConfig, BranchRules};
pub use paths::{Paths, PathsError};
pub use policy::{PROJECT_POLICY_FILE, PolicyConfig, PolicyRule, ProtectedConfig};
pub use project::{PROJECT_CONFIG_FILE, ProjectConfig, RepoConfig};
pub use user::{SandboxConfig, UserConfig};

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{}: {message}", path.display())]
    Parse { path: PathBuf, message: String },
    #[error("{}: {}", path.display(), problems.join("; "))]
    Invalid {
        path: PathBuf,
        problems: Vec<String>,
    },
}

/// Read and parse a TOML file. `Ok(None)` if it doesn't exist.
fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    toml::from_str(&text)
        .map(Some)
        .map_err(|e| ConfigError::Parse {
            path: path.to_path_buf(),
            message: e.to_string(),
        })
}

/// JSON Schemas for every config file, as `(file name, pretty JSON)`.
pub fn schemas() -> Vec<(&'static str, String)> {
    let render = |schema: schemars::Schema| {
        let mut json = serde_json::to_string_pretty(&schema).expect("schemas serialize");
        json.push('\n');
        json
    };
    vec![
        (
            "forge.toml.schema.json",
            render(schemars::schema_for!(ProjectConfig)),
        ),
        (
            "user-config.toml.schema.json",
            render(schemars::schema_for!(UserConfig)),
        ),
        (
            "agent-profile.toml.schema.json",
            render(schemars::schema_for!(AgentProfileConfig)),
        ),
        (
            "policy.toml.schema.json",
            render(schemars::schema_for!(PolicyConfig)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn schemas_forbid_unknown_keys() {
        for (name, json) in super::schemas() {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["additionalProperties"], false, "{name}");
        }
    }
}
