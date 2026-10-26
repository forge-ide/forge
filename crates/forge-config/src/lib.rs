//! Project, workspace, and agent-profile config with layering.
//!
//! Every config file has a Rust type here; `just gen` writes its JSON Schema to `docs/schemas/`.
//! Unknown keys are errors so typos surface immediately.

use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

pub mod agent;
pub mod branches;
pub mod hooks;
pub mod paths;
pub mod policy;
pub mod project;
pub mod user;

pub use agent::{AGENTS_DIR, AgentProfileConfig, InvalidProfileId, ProfileMode, ProfileSandbox};
pub use branches::{BranchConfig, BranchRules};
pub use hooks::{
    HOOKS_DIR, HookActionType, HookConfig, HookEvent, HookMatch, HookPlace, HooksConfig,
    PROJECT_HOOKS_FILE,
};
pub use paths::{Paths, PathsError};
pub use policy::{
    PROJECT_POLICY_FILE, PolicyConfig, PolicyRule, ProtectedConfig, RuleListName, append_rule,
};
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

/// Largest config file read; a cloned repo supplies some of them.
const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Read and parse a TOML file. `Ok(None)` if it doesn't exist. Only a regular file up to
/// [`MAX_CONFIG_BYTES`] is read, so a repo can't hang or exhaust the daemon with one that
/// names a FIFO or a device.
fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<Option<T>, ConfigError> {
    use std::io::Read;
    let io = |source| ConfigError::Io {
        path: path.to_path_buf(),
        source,
    };
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() => {
            return Err(io(std::io::Error::other("not a regular file")));
        }
        Ok(meta) if meta.len() > MAX_CONFIG_BYTES => {
            return Err(io(std::io::Error::other("larger than 1 MiB")));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    }
    let mut text = String::new();
    std::fs::File::open(path)
        .and_then(|f| f.take(MAX_CONFIG_BYTES + 1).read_to_string(&mut text))
        .map_err(io)?;
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return Err(io(std::io::Error::other("larger than 1 MiB")));
    }
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
            "hooks.toml.schema.json",
            render(schemars::schema_for!(HooksConfig)),
        ),
        (
            "policy.toml.schema.json",
            render(schemars::schema_for!(PolicyConfig)),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_regular_files_of_bounded_size_are_read() {
        let dir = tempfile::tempdir().unwrap();
        let dev = Path::new("/dev/zero");
        assert!(matches!(
            read_toml::<toml::Value>(dev),
            Err(ConfigError::Io { .. })
        ));
        let link = dir.path().join("x.toml");
        std::os::unix::fs::symlink(dev, &link).unwrap();
        assert!(matches!(
            read_toml::<toml::Value>(&link),
            Err(ConfigError::Io { .. })
        ));
        let fifo = dir.path().join("fifo.toml");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        assert!(made.unwrap().success());
        assert!(matches!(
            read_toml::<toml::Value>(&fifo),
            Err(ConfigError::Io { .. })
        ));
        let big = dir.path().join("big.toml");
        let text = format!("a = \"{}\"\n", "x".repeat(MAX_CONFIG_BYTES as usize));
        std::fs::write(&big, text).unwrap();
        assert!(matches!(
            read_toml::<toml::Value>(&big),
            Err(ConfigError::Io { .. })
        ));
        let ok = dir.path().join("ok.toml");
        std::fs::write(&ok, "a = 1\n").unwrap();
        assert!(read_toml::<toml::Value>(&ok).unwrap().is_some());
        assert!(
            read_toml::<toml::Value>(&dir.path().join("missing"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn schemas_forbid_unknown_keys() {
        for (name, json) in super::schemas() {
            let value: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(value["additionalProperties"], false, "{name}");
        }
    }
}
