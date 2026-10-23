//! What the helper binary enforces on itself before it execs the command.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Which restrictions the helper applies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Landlock plus the Light seccomp filter.
    Light,
    /// Inside bubblewrap: the Standard seccomp filter only; namespaces do the rest.
    Standard,
}

/// Passed to the helper as one JSON argument.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub mode: Mode,
    /// Readable and executable (Light).
    #[serde(default)]
    pub read: Vec<PathBuf>,
    /// Readable, writable, and executable (Light).
    #[serde(default)]
    pub write: Vec<PathBuf>,
    /// TCP ports the command may connect to, on any address (Light).
    #[serde(default)]
    pub connect_ports: Vec<u16>,
}

impl Policy {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("policy serializes")
    }

    pub fn from_json(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("invalid sandbox policy: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_rejects_unknown_fields() {
        let policy = Policy {
            mode: Mode::Light,
            read: vec!["/usr".into()],
            write: vec!["/w".into()],
            connect_ports: vec![],
        };
        assert_eq!(Policy::from_json(&policy.to_json()).unwrap(), policy);
        assert!(Policy::from_json(r#"{"mode":"light","extra":1}"#).is_err());
    }
}
