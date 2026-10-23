//! Where Forge keeps its data, socket, and user config on each platform.
//!
//! `FORGE_DATA_DIR`, `FORGE_RUNTIME_DIR`, and `FORGE_CONFIG_DIR` override the defaults; tests
//! set them so they never touch the real user directories. `FORGE_MANAGED_POLICY` names another
//! org-managed policy file, so tests never read the machine's; like the rest of the managed
//! layer, it constrains agents, not the user, who could run any daemon they like.

use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("cannot locate {what}: set {var} or HOME")]
pub struct PathsError {
    pub what: &'static str,
    pub var: &'static str,
}

/// Resolved Forge directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Database, worktrees, logs, installed servers and agents.
    pub data_dir: PathBuf,
    /// Holds the daemon socket.
    pub runtime_dir: PathBuf,
    /// User config and user policy.
    pub config_dir: PathBuf,
    /// The org-managed policy file, read-only to users.
    pub managed_policy: PathBuf,
}

impl Paths {
    /// Resolve from the process environment.
    pub fn from_env() -> Result<Self, PathsError> {
        Self::from_vars(|name| std::env::var_os(name).filter(|v| !v.is_empty()))
    }

    /// Resolve using `var` to look up environment variables.
    pub fn from_vars(var: impl Fn(&str) -> Option<OsString>) -> Result<Self, PathsError> {
        let home = var("HOME").map(PathBuf::from);
        let data_dir = match var("FORGE_DATA_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => platform::default_data_dir(&var, home.as_ref()).ok_or(PathsError {
                what: "the data directory",
                var: "FORGE_DATA_DIR",
            })?,
        };
        let runtime_dir = match var("FORGE_RUNTIME_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => platform::default_runtime_dir(&var, &data_dir),
        };
        let config_dir = match var("FORGE_CONFIG_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => platform::default_config_dir(&var, home.as_ref()).ok_or(PathsError {
                what: "the config directory",
                var: "FORGE_CONFIG_DIR",
            })?,
        };
        let managed_policy = var("FORGE_MANAGED_POLICY")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(platform::MANAGED_POLICY));
        Ok(Self {
            data_dir,
            runtime_dir,
            config_dir,
            managed_policy,
        })
    }

    pub fn socket(&self) -> PathBuf {
        self.runtime_dir.join("forged.sock")
    }

    pub fn database(&self) -> PathBuf {
        self.data_dir.join("forge.db")
    }

    pub fn daemon_log(&self) -> PathBuf {
        self.data_dir.join("forged.log")
    }

    pub fn user_config(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn user_policy(&self) -> PathBuf {
        self.config_dir.join("policy.toml")
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    pub const MANAGED_POLICY: &str = "/etc/forge/managed.toml";

    pub fn default_data_dir(
        var: impl Fn(&str) -> Option<OsString>,
        home: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        match var("XDG_DATA_HOME") {
            Some(dir) => Some(PathBuf::from(dir).join("forge")),
            None => home.map(|h| h.join(".local/share/forge")),
        }
    }

    /// `$XDG_RUNTIME_DIR/forge`; without it, the data dir, as on macOS.
    pub fn default_runtime_dir(var: impl Fn(&str) -> Option<OsString>, data_dir: &Path) -> PathBuf {
        match var("XDG_RUNTIME_DIR") {
            Some(dir) => PathBuf::from(dir).join("forge"),
            None => data_dir.to_path_buf(),
        }
    }

    pub fn default_config_dir(
        var: impl Fn(&str) -> Option<OsString>,
        home: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        match var("XDG_CONFIG_HOME") {
            Some(dir) => Some(PathBuf::from(dir).join("forge")),
            None => home.map(|h| h.join(".config/forge")),
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    pub const MANAGED_POLICY: &str = "/Library/Application Support/Forge/managed.toml";

    pub fn default_data_dir(
        _var: impl Fn(&str) -> Option<OsString>,
        home: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        home.map(|h| h.join("Library/Application Support/Forge"))
    }

    pub fn default_runtime_dir(
        _var: impl Fn(&str) -> Option<OsString>,
        data_dir: &Path,
    ) -> PathBuf {
        data_dir.to_path_buf()
    }

    pub fn default_config_dir(
        _var: impl Fn(&str) -> Option<OsString>,
        home: Option<&PathBuf>,
    ) -> Option<PathBuf> {
        home.map(|h| h.join(".config/forge"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolve(vars: &[(&str, &str)]) -> Result<Paths, PathsError> {
        let vars: HashMap<String, OsString> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), OsString::from(v)))
            .collect();
        Paths::from_vars(|name| vars.get(name).cloned())
    }

    #[test]
    fn overrides_win() {
        let paths = resolve(&[
            ("HOME", "/home/u"),
            ("FORGE_DATA_DIR", "/tmp/d"),
            ("FORGE_RUNTIME_DIR", "/tmp/r"),
            ("FORGE_CONFIG_DIR", "/tmp/c"),
            ("FORGE_MANAGED_POLICY", "/tmp/m.toml"),
        ])
        .unwrap();
        assert_eq!(paths.managed_policy, PathBuf::from("/tmp/m.toml"));
        assert_eq!(paths.user_policy(), PathBuf::from("/tmp/c/policy.toml"));
        assert_eq!(paths.socket(), PathBuf::from("/tmp/r/forged.sock"));
        assert_eq!(paths.database(), PathBuf::from("/tmp/d/forge.db"));
        assert_eq!(paths.user_config(), PathBuf::from("/tmp/c/config.toml"));
    }

    #[test]
    fn no_home_and_no_overrides_is_an_error() {
        assert!(resolve(&[]).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_defaults() {
        let paths = resolve(&[("HOME", "/home/u"), ("XDG_RUNTIME_DIR", "/run/user/1000")]).unwrap();
        assert_eq!(paths.data_dir, PathBuf::from("/home/u/.local/share/forge"));
        assert_eq!(
            paths.socket(),
            PathBuf::from("/run/user/1000/forge/forged.sock")
        );
        assert_eq!(paths.config_dir, PathBuf::from("/home/u/.config/forge"));
        assert_eq!(
            paths.managed_policy,
            PathBuf::from("/etc/forge/managed.toml")
        );

        let no_runtime = resolve(&[("HOME", "/home/u")]).unwrap();
        assert_eq!(
            no_runtime.socket(),
            PathBuf::from("/home/u/.local/share/forge/forged.sock")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_defaults() {
        let paths = resolve(&[("HOME", "/Users/u")]).unwrap();
        assert_eq!(
            paths.data_dir,
            PathBuf::from("/Users/u/Library/Application Support/Forge")
        );
        assert_eq!(paths.socket(), paths.data_dir.join("forged.sock"));
        assert_eq!(paths.config_dir, PathBuf::from("/Users/u/.config/forge"));
        assert_eq!(
            paths.managed_policy,
            PathBuf::from("/Library/Application Support/Forge/managed.toml")
        );
    }
}
