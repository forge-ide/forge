//! The YAML script format: a list of steps replayed across prompts.

use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// Each step is a one-key map (`- read: path`) or a bare name (`- end_turn`).
    #[serde(with = "serde_norway::with::singleton_map_recursive")]
    pub steps: Vec<Step>,
}

/// One scripted action. Steps run in order; `end_turn` finishes the current prompt and the next
/// prompt continues from the step after it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// Fail the turn unless the prompt matches this glob (`*` matches anything).
    ExpectPrompt(String),
    /// Publish a plan with these entries, all pending.
    Plan(Vec<String>),
    /// Read a file through the client, relative to the session's working directory.
    Read(String),
    /// Write a file through the client.
    Write(WriteStep),
    /// Run a command in a client terminal and wait for it to exit.
    Exec(Vec<String>),
    /// Ask the client for permission.
    RequestPermission(PermissionStep),
    /// Fail the turn unless the last permission request got this answer.
    ExpectPermission(Expected),
    /// Stream an agent message.
    Message(String),
    /// Wait, unless the turn is cancelled first.
    SleepMs(u64),
    /// Finish the turn with stop reason `end_turn`.
    EndTurn,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WriteStep {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionStep {
    pub action: PermissionAction,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Exec,
    Edit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Expected {
    Allow,
    Deny,
}

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("reading {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("parsing script: {0}")]
    Parse(#[from] serde_norway::Error),
}

impl Script {
    pub fn parse(text: &str) -> Result<Self, ScriptError> {
        Ok(serde_norway::from_str(text)?)
    }

    pub fn load(path: &Path) -> Result<Self, ScriptError> {
        let text = std::fs::read_to_string(path).map_err(|source| ScriptError::Read {
            path: path.display().to_string(),
            source,
        })?;
        Self::parse(&text)
    }
}

/// Whether `text` matches `pattern`, where `*` matches any run of characters.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*');
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = text.strip_prefix(first) else {
        return false;
    };
    let parts: Vec<&str> = parts.collect();
    let Some((last, middle)) = parts.split_last() else {
        return rest.is_empty();
    };
    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }
    rest.ends_with(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_step() {
        let script = Script::parse(
            r#"
steps:
  - expect_prompt: "*retry*"
  - plan: ["Read client", "Add backoff"]
  - read: src/webhook.rs
  - write: { path: src/a.rs, content: "x" }
  - exec: ["cargo", "test", "webhook"]
  - request_permission: { action: exec, argv: ["pnpm", "add", "left-pad"] }
  - expect_permission: deny
  - message: "Done."
  - sleep_ms: 10
  - end_turn
"#,
        )
        .unwrap();
        assert_eq!(script.steps.len(), 10);
        assert_eq!(script.steps[0], Step::ExpectPrompt("*retry*".into()));
        assert_eq!(
            script.steps[5],
            Step::RequestPermission(PermissionStep {
                action: PermissionAction::Exec,
                argv: vec!["pnpm".into(), "add".into(), "left-pad".into()],
                path: None,
            })
        );
        assert_eq!(script.steps[9], Step::EndTurn);
    }

    #[test]
    fn rejects_unknown_steps_and_keys() {
        assert!(Script::parse("steps:\n  - dance: 1\n").is_err());
        assert!(Script::parse("steps:\n  - write: { path: a, content: b, mode: x }\n").is_err());
        assert!(Script::parse("steps: []\nextra: 1\n").is_err());
    }

    #[test]
    fn globs() {
        assert!(glob_match("*retry*", "add retry backoff"));
        assert!(glob_match("*", ""));
        assert!(glob_match("exact", "exact"));
        assert!(glob_match("a*c", "abc"));
        assert!(glob_match("a*b*c", "a-b-c"));
        assert!(!glob_match("a*c", "ab"));
        assert!(!glob_match("exact", "exactly"));
        assert!(!glob_match("*retry*", "backoff"));
    }
}
