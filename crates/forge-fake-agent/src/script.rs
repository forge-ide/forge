//! The YAML script format: a list of steps replayed across prompts.

use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// Each step is a one-key map (`- read: path`) or a bare name (`- end_turn`).
    #[serde(with = "serde_norway::with::singleton_map_recursive")]
    pub steps: Vec<Step>,
    /// Sign-in the agent needs before it opens a session, as a real agent keeps it: a login
    /// file in its home that a sign-in writes and later processes find.
    #[serde(default)]
    pub auth: Option<AuthScript>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthScript {
    /// The login file, relative to `$HOME` (or `$FORGE_FAKE_AGENT_HOME`).
    pub login: String,
    /// The methods `initialize` advertises; terminal ones only to a client that runs them.
    pub methods: Vec<AuthMethodScript>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthMethodScript {
    pub id: String,
    pub name: String,
    /// `agent` (signed in by `authenticate`) or `terminal` (by running the agent with `args`).
    #[serde(default = "agent_kind")]
    pub kind: String,
    #[serde(default)]
    pub args: Vec<String>,
}

fn agent_kind() -> String {
    "agent".into()
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
    /// Run a command in a client terminal and wait for it to exit: `exec: [argv...]`, or
    /// `exec: { argv: [...], expect: deny }` for a command the client should refuse.
    Exec(ExecStep),
    /// Ask the client for permission.
    RequestPermission(PermissionStep),
    /// Call a tool on the MCP server the client named `forge` at session start. The turn
    /// fails unless the call succeeds, or with `expect: deny`, unless the tool refuses.
    McpCall(McpCallStep),
    /// Fail the turn unless the last permission request got this answer.
    ExpectPermission(Expected),
    /// Stream an agent message; `{forge_mcp_url}` in it is replaced by the URL of the client's
    /// `forge` MCP server.
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
    /// `deny` if the client should refuse the write; the turn fails if it doesn't.
    #[serde(default)]
    pub expect: Option<Expected>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "ExecForm")]
pub struct ExecStep {
    pub argv: Vec<String>,
    /// `deny` if the client should refuse to run it; the turn fails if it doesn't.
    pub expect: Option<Expected>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ExecForm {
    Argv(Vec<String>),
    Full(FullExec),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FullExec {
    argv: Vec<String>,
    #[serde(default)]
    expect: Option<Expected>,
}

impl From<ExecForm> for ExecStep {
    fn from(form: ExecForm) -> Self {
        match form {
            ExecForm::Argv(argv) => Self { argv, expect: None },
            ExecForm::Full(FullExec { argv, expect }) => Self { argv, expect },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpCallStep {
    pub tool: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
    #[serde(default)]
    pub expect: Option<Expected>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionStep {
    pub action: PermissionAction,
    #[serde(default)]
    pub argv: Vec<String>,
    #[serde(default)]
    pub path: Option<String>,
    /// For an edit: the paths it changes, as its locations (a patch touching several files).
    #[serde(default)]
    pub paths: Vec<String>,
    /// For a parsed read: the command the request names.
    #[serde(default)]
    pub command: Option<String>,
    /// For an MCP tool: its name on the client's `forge` server.
    #[serde(default)]
    pub tool: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionAction {
    Exec,
    Edit,
    /// Read `path`, as a tool of kind `read` with it as the location.
    Read,
    /// Fetch a URL, as a tool of kind `fetch`.
    Fetch,
    /// Run `command`, reported as Codex reports a command it parses as a read: a tool call
    /// of kind `read` with `path` as its location and no command, then a request of kind
    /// `execute` whose `rawInput.command` is the command.
    ParsedRead,
    /// Call `tool` on the `forge` server, as Codex asks: a tool call marked
    /// `is_mcp_tool_call` naming the server and tool, then a request for it marked
    /// `is_mcp_tool_approval` that names nothing else.
    McpTool,
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
  - mcp_call: { tool: git_commit, arguments: { message: "add f" }, expect: allow }
  - end_turn
"#,
        )
        .unwrap();
        assert_eq!(script.steps.len(), 11);
        assert_eq!(
            script.steps[9],
            Step::McpCall(McpCallStep {
                tool: "git_commit".into(),
                arguments: serde_json::json!({ "message": "add f" }),
                expect: Some(Expected::Allow),
            })
        );
        assert_eq!(script.steps[0], Step::ExpectPrompt("*retry*".into()));
        assert_eq!(
            script.steps[5],
            Step::RequestPermission(PermissionStep {
                action: PermissionAction::Exec,
                argv: vec!["pnpm".into(), "add".into(), "left-pad".into()],
                path: None,
                paths: vec![],
                command: None,
                tool: None,
            })
        );
        assert_eq!(script.steps[10], Step::EndTurn);
    }

    #[test]
    fn writes_and_commands_can_expect_a_refusal() {
        let script = Script::parse(
            r#"
steps:
  - write: { path: .env, content: "x", expect: deny }
  - exec: { argv: ["git", "push"], expect: deny }
  - exec: { argv: ["true"] }
"#,
        )
        .unwrap();
        let Step::Write(w) = &script.steps[0] else {
            panic!("not a write: {:?}", script.steps[0]);
        };
        assert_eq!(w.expect, Some(Expected::Deny));
        assert_eq!(
            script.steps[1],
            Step::Exec(ExecStep {
                argv: vec!["git".into(), "push".into()],
                expect: Some(Expected::Deny),
            })
        );
        assert_eq!(
            script.steps[2],
            Step::Exec(ExecStep {
                argv: vec!["true".into()],
                expect: None,
            })
        );
        assert!(Script::parse("steps:\n  - exec: { argv: [a], mode: x }\n").is_err());
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
