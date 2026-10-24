//! `.forge/hooks.toml`: shell commands Forge runs at points in a session, whatever the agent.

use std::path::Path;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{ConfigError, read_toml};

/// The project's hooks file, relative to the project root.
pub const PROJECT_HOOKS_FILE: &str = ".forge/hooks.toml";

/// Where hook scripts named by a relative path live, relative to the project root.
pub const HOOKS_DIR: &str = ".forge/hooks";

/// Longest timeout a hook may set, in milliseconds.
pub const MAX_TIMEOUT_MS: u64 = 600_000;

/// A hooks file. A missing file has no hooks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HooksConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hook: Vec<HookConfig>,
}

/// One hook.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookConfig {
    /// Shown in decisions (`hook:<name>`) and the timeline: 1 to 64 letters, digits, `.`, `_`,
    /// or `-`. Defaults to the file name of the program.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub event: HookEvent,
    /// Run only for matching actions; every action when unset.
    #[serde(default, rename = "match", skip_serializing_if = "Option::is_none")]
    pub filter: Option<HookMatch>,
    /// Program and arguments. A program or argument given as a relative path under
    /// `.forge/hooks/` is taken from the project root, never a session's worktree; any other
    /// relative program is refused. A bare program name is found through the daemon's `PATH` and
    /// must be in a system or toolchain directory the sandbox already shows.
    pub run: Vec<String>,
    /// Defaults to 5000 for tool and edit hooks and 300000 for `task.done`; at most 600000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Where the hook runs; only `sandbox`, the session's own sandbox, is supported so far.
    #[serde(default, rename = "where", skip_serializing_if = "Option::is_none")]
    pub run_in: Option<HookPlace>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum HookEvent {
    #[serde(rename = "session.start")]
    SessionStart,
    #[serde(rename = "tool.pre")]
    ToolPre,
    #[serde(rename = "tool.post")]
    ToolPost,
    #[serde(rename = "edit.pre")]
    EditPre,
    #[serde(rename = "edit.post")]
    EditPost,
    #[serde(rename = "commit.pre")]
    CommitPre,
    #[serde(rename = "task.done")]
    TaskDone,
    #[serde(rename = "approval.requested")]
    ApprovalRequested,
}

impl HookEvent {
    /// The name hooks see in their input and in `hook.run` events.
    pub fn name(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "session.start",
            HookEvent::ToolPre => "tool.pre",
            HookEvent::ToolPost => "tool.post",
            HookEvent::EditPre => "edit.pre",
            HookEvent::EditPost => "edit.post",
            HookEvent::CommitPre => "commit.pre",
            HookEvent::TaskDone => "task.done",
            HookEvent::ApprovalRequested => "approval.requested",
        }
    }

    /// Whether Forge runs hooks for this event yet.
    pub fn supported(self) -> bool {
        !matches!(self, HookEvent::SessionStart | HookEvent::ApprovalRequested)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookPlace {
    Sandbox,
    Daemon,
}

/// Which actions a hook runs for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookMatch {
    /// The action type: `exec`, `edit`, `mcp`, or `git`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<HookActionType>,
    /// A glob over a command's space-joined argv, as in policy `exec` patterns; it matches the
    /// whole line or any simple command in it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv: Option<String>,
    /// A gitignore-style glob over an edit's repo-relative path, as in policy `edit` patterns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HookActionType {
    Exec,
    Edit,
    Mcp,
    Git,
}

impl HooksConfig {
    /// Load and validate the file at `path`; a missing file has no hooks.
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

    /// Every problem with the hooks' shape. Glob syntax is checked when the runner compiles
    /// them.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (i, hook) in self.hook.iter().enumerate() {
            let at = format!("hook[{i}]");
            if let Some(name) = &hook.name
                && !valid_name(name)
            {
                problems.push(format!(
                    "{at}: name {name:?} must be 1 to 64 letters, digits, '.', '_', or '-'"
                ));
            }
            if !hook.event.supported() {
                problems.push(format!(
                    "{at}: Forge doesn't run {} hooks yet",
                    hook.event.name()
                ));
            }
            if hook.run_in == Some(HookPlace::Daemon) {
                problems.push(format!(
                    "{at}: hooks can only run in the session's sandbox so far; remove where = \
                     \"daemon\""
                ));
            }
            match hook.run.first() {
                None => problems.push(format!("{at}: run needs a program")),
                Some(program) if program.trim().is_empty() => {
                    problems.push(format!("{at}: run's program is empty"));
                }
                Some(program) if program.contains('/') && !program.starts_with('/') => {
                    if hook_script(program).is_none() {
                        problems.push(format!(
                            "{at}: {program:?} must be a script under {HOOKS_DIR}/, which Forge \
                             takes from the project root"
                        ));
                    }
                }
                Some(_) => {}
            }
            if let Some(ms) = hook.timeout_ms
                && !(1..=MAX_TIMEOUT_MS).contains(&ms)
            {
                problems.push(format!(
                    "{at}: timeout_ms must be between 1 and {MAX_TIMEOUT_MS}"
                ));
            }
            if let Some(filter) = &hook.filter {
                let edits = matches!(hook.event, HookEvent::EditPre | HookEvent::EditPost);
                let tools = matches!(hook.event, HookEvent::ToolPre | HookEvent::ToolPost);
                if !edits && !tools {
                    problems.push(format!("{at}: match only applies to tool and edit hooks"));
                }
                if filter.argv.is_some() && edits {
                    problems.push(format!("{at}: edit hooks match path, not argv"));
                }
                if filter.path.is_some() && tools {
                    problems.push(format!("{at}: tool hooks match argv, not path"));
                }
                if edits && filter.action.is_some_and(|a| a != HookActionType::Edit) {
                    problems.push(format!("{at}: edit hooks only see edits"));
                }
                if tools && filter.action == Some(HookActionType::Edit) {
                    problems.push(format!("{at}: edits run edit hooks, not tool hooks"));
                }
                if filter.argv.as_ref().is_some_and(|p| p.trim().is_empty())
                    || filter.path.as_ref().is_some_and(|p| p.trim().is_empty())
                {
                    problems.push(format!("{at}: match patterns must not be empty"));
                }
            }
        }
        problems
    }
}

/// A hook name fit for a decision's source and the timeline.
fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// The path under `.forge/hooks/` that a relative program names, without `.` or `..`
/// segments, or `None` if it names anything else.
pub fn hook_script(program: &str) -> Option<String> {
    let mut parts = Vec::new();
    for seg in program.split('/') {
        match seg {
            "" | "." => {}
            ".." => return None,
            seg => parts.push(seg),
        }
    }
    let prefix: Vec<&str> = HOOKS_DIR.split('/').collect();
    (parts.len() > prefix.len() && parts[..prefix.len()] == prefix[..]).then(|| parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> HooksConfig {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn loads_the_spec_example() {
        let config = parse(
            r#"
[[hook]]
event = "tool.pre"
match = { action = "exec", argv = "npm install*" }
run = [".forge/hooks/block-npm.sh"]
timeout_ms = 5000
where = "sandbox"
"#,
        );
        assert!(config.validate().is_empty(), "{:?}", config.validate());
        let hook = &config.hook[0];
        assert_eq!(hook.event, HookEvent::ToolPre);
        assert_eq!(
            hook.filter.as_ref().unwrap().argv.as_deref(),
            Some("npm install*")
        );
        assert_eq!(hook.run_in, Some(HookPlace::Sandbox));
    }

    #[test]
    fn refuses_what_forge_cant_run_safely_or_yet() {
        let config = parse(
            r#"
[[hook]]
event = "session.start"
run = ["true"]

[[hook]]
event = "tool.pre"
run = ["notify"]
where = "daemon"

[[hook]]
event = "tool.pre"
run = ["scripts/check.sh"]

[[hook]]
event = "tool.pre"
run = [".forge/hooks/../agents/x.sh"]

[[hook]]
event = "edit.pre"
match = { argv = "x*" }
run = ["true"]

[[hook]]
event = "task.done"
run = []
timeout_ms = 0

[[hook]]
event = "tool.pre"
name = "mode:auto"
run = ["true"]
"#,
        );
        let problems = config.validate();
        assert_eq!(problems.len(), 8, "{problems:#?}");
        assert!(problems[7].contains("mode:auto"));
        assert!(problems[0].contains("session.start"));
        assert!(problems[1].contains("daemon"));
        assert!(problems[2].contains(".forge/hooks/"));
        assert!(problems[3].contains(".forge/hooks/"));
    }

    #[test]
    fn hook_scripts_stay_under_the_hooks_dir() {
        assert_eq!(
            hook_script(".forge/hooks/a.sh").as_deref(),
            Some(".forge/hooks/a.sh")
        );
        assert_eq!(
            hook_script("./.forge//hooks/sub/a.sh").as_deref(),
            Some(".forge/hooks/sub/a.sh")
        );
        assert_eq!(hook_script(".forge/hooks"), None);
        assert_eq!(hook_script(".forge/hooks/../policy.toml"), None);
        assert_eq!(hook_script("scripts/a.sh"), None);
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(
            toml::from_str::<HooksConfig>(
                "[[hook]]\nevent = \"tool.pre\"\nrun = [\"x\"]\nextra = 1\n"
            )
            .is_err()
        );
        assert!(
            toml::from_str::<HooksConfig>("[[hook]]\nevent = \"tool.later\"\nrun = [\"x\"]\n")
                .is_err()
        );
    }
}
