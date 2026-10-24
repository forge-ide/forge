//! What an agent's permission request asks to do, as Forge can check it.

use std::collections::VecDeque;
use std::path::PathBuf;

use agent_client_protocol::schema::v1::{self as acp, RequestPermissionRequest};
use forge_api::model::{Action, EditKind};

use crate::adapters::CommandSource;
use crate::workspace::Workspace;

/// Commands remembered from tool calls, for agents whose permission requests leave them out.
const REMEMBERED_COMMANDS: usize = 256;

/// What a permission request asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// An action the policy engine decides.
    Action(Action),
    /// Reading or searching files, which the policy engine has no action for: the sandbox
    /// bounds every read, so Forge only checks that each named path is inside a worktree.
    Read { inside_worktree: bool },
    /// Fetching a URL with the agent's own tool. The egress proxy's allowlist is the whole
    /// network policy, so these are refused and the agent fetches through a command instead.
    Fetch,
    /// Nothing Forge can check.
    Unknown,
}

/// The commands an agent reported in its tool calls, by tool call ID, as first reported.
#[derive(Debug, Default)]
pub struct ToolCommands(VecDeque<(String, String)>);

impl ToolCommands {
    /// Remember `call`'s command, if it reports one and none was remembered for it yet.
    pub fn record(&mut self, id: &str, raw_input: Option<&serde_json::Value>) {
        let Some(command) = raw_input.and_then(|r| r.get("command")?.as_str()) else {
            return;
        };
        if self.0.iter().any(|(known, _)| known == id) {
            return;
        }
        if self.0.len() == REMEMBERED_COMMANDS {
            self.0.pop_front();
        }
        self.0.push_back((id.to_owned(), command.to_owned()));
    }

    fn get(&self, id: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, command)| command.as_str())
    }
}

/// The [`Target`] of a permission request, from an agent whose execute requests carry their
/// command the way `source` says.
pub fn target(
    req: &RequestPermissionRequest,
    workspace: &Workspace,
    source: CommandSource,
    commands: &ToolCommands,
) -> Target {
    let fields = &req.tool_call.fields;
    let raw = fields.raw_input.as_ref();
    let Some(kind) = fields.kind else {
        return Target::Unknown;
    };
    match kind {
        acp::ToolKind::Execute => {
            let argv = match source {
                CommandSource::RawInput => raw_command(raw),
                CommandSource::Title => fields.title.as_deref().map(shell_line),
                CommandSource::ToolCallArgv => {
                    commands.get(&req.tool_call.tool_call_id.0).map(quoted_argv)
                }
            };
            match argv {
                Some(argv) if !argv.is_empty() => Target::Action(Action::Exec {
                    argv,
                    cwd: ".".into(),
                }),
                _ => Target::Unknown,
            }
        }
        acp::ToolKind::Edit | acp::ToolKind::Delete | acp::ToolKind::Move => {
            let Some(path) = fields
                .locations
                .as_ref()
                .and_then(|l| l.first())
                .map(|l| l.path.clone())
                .or_else(|| raw.and_then(|r| r.get("path")?.as_str().map(PathBuf::from)))
            else {
                return Target::Unknown;
            };
            // Resolved as a write would be, so the path decided is the file that would really
            // change: a symlink the agent planted can't carry an allowed edit elsewhere.
            let Ok(resolved) = workspace.resolve_write(&workspace.cwd().join(path)) else {
                return Target::Unknown;
            };
            let kind = if kind == acp::ToolKind::Delete {
                EditKind::Delete
            } else {
                EditKind::Modify
            };
            Target::Action(Action::Edit {
                path: resolved.display,
                kind,
            })
        }
        acp::ToolKind::Read | acp::ToolKind::Search => {
            let mut paths: Vec<PathBuf> = fields
                .locations
                .iter()
                .flatten()
                .map(|l| l.path.clone())
                .collect();
            if paths.is_empty()
                && let Some(path) = raw.and_then(|r| r.get("path")?.as_str())
            {
                paths.push(PathBuf::from(path));
            }
            // A search names its directory or searches the working directory; a read must name
            // what it reads.
            if paths.is_empty() {
                return match kind {
                    acp::ToolKind::Search => Target::Read {
                        inside_worktree: true,
                    },
                    _ => Target::Read {
                        inside_worktree: false,
                    },
                };
            }
            let inside_worktree = paths
                .iter()
                .all(|p| workspace.resolve_read(&workspace.cwd().join(p)).is_ok());
            Target::Read { inside_worktree }
        }
        acp::ToolKind::Fetch => Target::Fetch,
        _ => Target::Unknown,
    }
}

/// `rawInput.argv` or `rawInput.command`: an array is kept as given, and a string is a shell
/// line, which is what the agent's shell runs; splitting it here would keep its quotes inside
/// the words.
fn raw_command(raw: Option<&serde_json::Value>) -> Option<Vec<String>> {
    match raw?.get("argv").or_else(|| raw?.get("command"))? {
        serde_json::Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
        ),
        serde_json::Value::String(line) => Some(shell_line(line)),
        _ => None,
    }
}

fn shell_line(line: &str) -> Vec<String> {
    vec!["sh".to_owned(), "-c".to_owned(), line.to_owned()]
}

/// An argv the agent reported shell-quoted (`/bin/bash -c 'cargo test'`), as its words. A line
/// with anything a shell would expand or treat as an operator outside quotes isn't a quoted
/// argv, so it is decided as the shell line it is.
fn quoted_argv(line: &str) -> Vec<String> {
    split_quoted(line).unwrap_or_else(|| shell_line(line))
}

/// The words of `line` under POSIX quoting, if it is only words: plain characters, single
/// quotes, double quotes (with `\` escaping `\`, `"`, `$`, and `` ` ``), and backslash escapes.
fn split_quoted(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if let Some(w) = word.take() {
                    words.push(w);
                }
            }
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => w.push(c),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            c @ ('\\' | '"' | '$' | '`') => w.push(c),
                            '\n' => {}
                            c => {
                                w.push('\\');
                                w.push(c);
                            }
                        },
                        '$' | '`' => return None,
                        c => w.push(c),
                    }
                }
            }
            '\\' => match chars.next()? {
                '\n' => {}
                c => word.get_or_insert_with(String::new).push(c),
            },
            c if "$`;&|<>(){}*?[]~#!\n\r".contains(c) => return None,
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    // A first word with `=` is a variable assignment, not the program.
    if words.first().is_some_and(|w| w.contains('=')) {
        return None;
    }
    Some(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        kind: acp::ToolKind,
        f: impl FnOnce(acp::ToolCallUpdateFields) -> acp::ToolCallUpdateFields,
    ) -> RequestPermissionRequest {
        RequestPermissionRequest::new(
            acp::SessionId::new("s"),
            acp::ToolCallUpdate::new("t1", f(acp::ToolCallUpdateFields::new().kind(kind))),
            vec![],
        )
    }

    fn workspace() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("a.rs"), "").unwrap();
        std::fs::write(dir.path().join("outside.txt"), "").unwrap();
        let ws = Workspace::new(vec![crate::Root {
            name: "repo".into(),
            path: root,
        }])
        .unwrap();
        (dir, ws)
    }

    fn exec(target: Target) -> Vec<String> {
        match target {
            Target::Action(Action::Exec { argv, .. }) => argv,
            other => panic!("not an exec: {other:?}"),
        }
    }

    fn decide(req: &RequestPermissionRequest, ws: &Workspace, source: CommandSource) -> Target {
        target(req, ws, source, &ToolCommands::default())
    }

    #[test]
    fn raw_input_commands_are_shell_lines_or_argv_as_given() {
        let (_dir, ws) = workspace();
        let line = r#"git -C "a b" commit -m 'x y'"#;
        let req = request(acp::ToolKind::Execute, |f| {
            f.raw_input(serde_json::json!({ "command": line }))
        });
        assert_eq!(
            exec(decide(&req, &ws, CommandSource::RawInput)),
            ["sh", "-c", line]
        );
        let req = request(acp::ToolKind::Execute, |f| {
            f.raw_input(serde_json::json!({ "argv": ["git", "-C", "a b", "status"] }))
        });
        assert_eq!(
            exec(decide(&req, &ws, CommandSource::RawInput)),
            ["git", "-C", "a b", "status"]
        );
        let req = request(acp::ToolKind::Execute, |f| {
            f.raw_input(serde_json::json!({ "command": ["ls", "-la"] }))
        });
        assert_eq!(
            exec(decide(&req, &ws, CommandSource::RawInput)),
            ["ls", "-la"]
        );
        // Without a command it names nothing.
        let req = request(acp::ToolKind::Execute, |f| f.title("Run".to_owned()));
        assert_eq!(decide(&req, &ws, CommandSource::RawInput), Target::Unknown);
    }

    #[test]
    fn a_title_is_the_shell_line_for_agents_that_put_it_there() {
        let (_dir, ws) = workspace();
        let req = request(acp::ToolKind::Execute, |f| {
            f.title("cargo test && ls".to_owned())
        });
        assert_eq!(
            exec(decide(&req, &ws, CommandSource::Title)),
            ["sh", "-c", "cargo test && ls"]
        );
        // Other agents' titles are only labels.
        assert_eq!(decide(&req, &ws, CommandSource::RawInput), Target::Unknown);
    }

    #[test]
    fn a_tool_call_argv_is_decided_as_the_words_the_agent_runs() {
        let (_dir, ws) = workspace();
        let mut commands = ToolCommands::default();
        let req = request(acp::ToolKind::Execute, |f| {
            // What the approval itself carries, with the shell prefix stripped.
            f.raw_input(serde_json::json!({ "command": "cargo test" }))
        });
        // Nothing reported for the call: nothing to decide.
        assert_eq!(
            target(&req, &ws, CommandSource::ToolCallArgv, &commands),
            Target::Unknown
        );
        let reported = serde_json::json!({ "command": "/usr/bin/bash -c 'cargo test'" });
        commands.record("t1", Some(&reported));
        // A later report for the same call doesn't replace the first.
        commands.record("t1", Some(&serde_json::json!({ "command": "ls" })));
        assert_eq!(
            exec(target(&req, &ws, CommandSource::ToolCallArgv, &commands)),
            ["/usr/bin/bash", "-c", "cargo test"]
        );
        // A login shell stays one, so allow rules still never match it.
        let mut commands = ToolCommands::default();
        commands.record(
            "t1",
            Some(&serde_json::json!({ "command": r#"bash -lc 'echo "a b" '"'"'c'"'"''"# })),
        );
        assert_eq!(
            exec(target(&req, &ws, CommandSource::ToolCallArgv, &commands)),
            ["bash", "-lc", r#"echo "a b" 'c'"#]
        );
        // A line that isn't only quoted words is decided as a shell line.
        let mut commands = ToolCommands::default();
        let line = "bash -c 'x' && curl $HOST";
        commands.record("t1", Some(&serde_json::json!({ "command": line })));
        assert_eq!(
            exec(target(&req, &ws, CommandSource::ToolCallArgv, &commands)),
            ["sh", "-c", line]
        );
    }

    #[test]
    fn quoted_words_split_like_a_shell() {
        assert_eq!(
            split_quoted(r#"a 'b c' "d \"e\" \$f" g\ h '' --x=y"#).unwrap(),
            ["a", "b c", r#"d "e" $f"#, "g h", "", "--x=y"]
        );
        for line in [
            "a $b", "a `b`", "a; b", "a | b", "a > b", "a 'b", "a \"$b\"", "x=1 a", "a *",
        ] {
            assert_eq!(split_quoted(line), None, "{line}");
        }
    }

    #[test]
    fn reads_and_searches_are_checked_against_the_worktrees() {
        let (dir, ws) = workspace();
        let inside = ws.cwd().join("a.rs");
        let outside = dir.path().join("outside.txt");
        let read = |path: &std::path::Path, kind| {
            let req = request(kind, |f| {
                f.locations(vec![acp::ToolCallLocation::new(path.to_path_buf())])
            });
            decide(&req, &ws, CommandSource::RawInput)
        };
        assert_eq!(
            read(&inside, acp::ToolKind::Read),
            Target::Read {
                inside_worktree: true
            }
        );
        assert_eq!(
            read(&outside, acp::ToolKind::Read),
            Target::Read {
                inside_worktree: false
            }
        );
        assert_eq!(
            read(&outside, acp::ToolKind::Search),
            Target::Read {
                inside_worktree: false
            }
        );
        // Through a symlink out of the worktree.
        std::os::unix::fs::symlink(&outside, ws.cwd().join("link")).unwrap();
        assert_eq!(
            read(&ws.cwd().join("link"), acp::ToolKind::Read),
            Target::Read {
                inside_worktree: false
            }
        );
        // A read that names nothing is refused; a search with no path searches the worktree.
        let req = request(acp::ToolKind::Read, |f| f);
        assert_eq!(
            decide(&req, &ws, CommandSource::RawInput),
            Target::Read {
                inside_worktree: false
            }
        );
        let req = request(acp::ToolKind::Search, |f| f);
        assert_eq!(
            decide(&req, &ws, CommandSource::RawInput),
            Target::Read {
                inside_worktree: true
            }
        );
        let req = request(acp::ToolKind::Search, |f| {
            f.raw_input(serde_json::json!({ "path": "/etc" }))
        });
        assert_eq!(
            decide(&req, &ws, CommandSource::RawInput),
            Target::Read {
                inside_worktree: false
            }
        );
    }

    #[test]
    fn fetches_and_other_tools_are_not_actions() {
        let (_dir, ws) = workspace();
        let req = request(acp::ToolKind::Fetch, |f| {
            f.raw_input(serde_json::json!({ "url": "https://example.com" }))
        });
        assert_eq!(decide(&req, &ws, CommandSource::RawInput), Target::Fetch);
        for kind in [
            acp::ToolKind::Think,
            acp::ToolKind::Other,
            acp::ToolKind::SwitchMode,
        ] {
            let req = request(kind, |f| f.title("mcp__x__y".to_owned()));
            assert_eq!(decide(&req, &ws, CommandSource::RawInput), Target::Unknown);
        }
    }

    #[test]
    fn remembered_commands_are_bounded() {
        let mut commands = ToolCommands::default();
        for i in 0..=REMEMBERED_COMMANDS {
            commands.record(
                &format!("t{i}"),
                Some(&serde_json::json!({ "command": "ls" })),
            );
        }
        assert_eq!(commands.get("t0"), None);
        assert_eq!(commands.get(&format!("t{REMEMBERED_COMMANDS}")), Some("ls"));
    }
}
