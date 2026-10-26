//! What an agent's permission request asks to do, as Forge can check it.

use std::collections::VecDeque;
use std::path::PathBuf;

use agent_client_protocol::schema::v1::{self as acp, RequestPermissionRequest};
use forge_api::model::{Action, EditKind};

use crate::adapters::CommandSource;
use crate::workspace::Workspace;

/// Commands remembered from tool calls, for agents whose permission requests leave them out.
const REMEMBERED_COMMANDS: usize = 256;

/// The most of an MCP tool call's arguments, as JSON, kept for its approval and events.
const MCP_ARGS_LIMIT: usize = 64 * 1024;

/// What a permission request asks for.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// An action the policy engine decides.
    Action(Action),
    /// Edits to several paths in one request (a patch, a move), each decided on its own; the
    /// request is allowed only if every one is.
    Edits(Vec<Action>),
    /// Reading or searching files, which the policy engine has no action for: the sandbox
    /// bounds every read, so Forge only checks that each named path is inside a worktree.
    Read { inside_worktree: bool },
    /// Fetching a URL with the agent's own tool. The egress proxy's allowlist is the whole
    /// network policy, so these are refused and the agent fetches through a command instead.
    Fetch,
    /// Nothing Forge can check.
    Unknown,
}

/// The commands and file changes an agent reported in its tool calls, by tool call ID, as
/// first reported, the calls it reported as reads that name no command, and its MCP tool calls.
#[derive(Debug, Default)]
pub struct ToolCommands(
    VecDeque<(String, String)>,
    VecDeque<(String, Vec<Change>)>,
    VecDeque<String>,
    VecDeque<(String, Action)>,
);

/// A file a tool call reported it changes: the path its diff names (for a move, where the file
/// goes) and whether it is deleted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub path: PathBuf,
    pub delete: bool,
}

impl ToolCommands {
    /// Remember the files `call`'s diffs change, adding to what earlier reports of it named:
    /// every file any report names is decided.
    pub fn record_changes(&mut self, id: &str, content: &[acp::ToolCallContent]) {
        let changes: Vec<Change> = content
            .iter()
            .filter_map(|c| match c {
                acp::ToolCallContent::Diff(diff) => Some(Change {
                    path: diff.path.clone(),
                    delete: diff
                        .meta
                        .as_ref()
                        .and_then(|m| m.get("kind")?.as_str())
                        .is_some_and(|k| k == "delete"),
                }),
                _ => None,
            })
            .collect();
        if changes.is_empty() {
            return;
        }
        if let Some((_, known)) = self.1.iter_mut().find(|(known, _)| known == id) {
            for change in changes {
                if !known.contains(&change) {
                    known.push(change);
                }
            }
            return;
        }
        if self.1.len() == REMEMBERED_COMMANDS {
            self.1.pop_front();
        }
        self.1.push_back((id.to_owned(), changes));
    }

    fn changes(&self, id: &str) -> Option<&[Change]> {
        self.1
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, changes)| changes.as_slice())
    }

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

    /// Remember that `id` was reported as a read or search with no command: Codex reports a
    /// command it parses as one that way, and only its approval names the command.
    pub fn record_read(&mut self, id: &str) {
        if self.2.iter().any(|known| known == id) {
            return;
        }
        if self.2.len() == REMEMBERED_COMMANDS {
            self.2.pop_front();
        }
        self.2.push_back(id.to_owned());
    }

    /// Remember `call` as the MCP tool call it reports, if it is one: Codex reports them with
    /// `is_mcp_tool_call` in `_meta` and the server, tool, and arguments as `rawInput`.
    pub fn record_mcp(
        &mut self,
        id: &str,
        meta: Option<&serde_json::Map<String, serde_json::Value>>,
        raw_input: Option<&serde_json::Value>,
    ) {
        let marked = meta
            .and_then(|m| m.get("is_mcp_tool_call")?.as_bool())
            .unwrap_or(false);
        let (Some(raw), true) = (raw_input, marked) else {
            return;
        };
        let (Some(server), Some(tool)) = (
            raw.get("server").and_then(|v| v.as_str()),
            raw.get("tool").and_then(|v| v.as_str()),
        ) else {
            return;
        };
        if self.3.iter().any(|(known, _)| known == id) {
            return;
        }
        if self.3.len() == REMEMBERED_COMMANDS {
            self.3.pop_front();
        }
        let action = Action::Mcp {
            server: server.to_owned(),
            tool: tool.to_owned(),
            args: clipped(raw.get("arguments").cloned().unwrap_or_default()),
        };
        self.3.push_back((id.to_owned(), action));
    }

    fn mcp(&self, id: &str) -> Option<&Action> {
        self.3
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, action)| action)
    }

    fn reported_read(&self, id: &str) -> bool {
        self.2.iter().any(|known| known == id)
    }

    fn get(&self, id: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, command)| command.as_str())
    }
}

/// `args`, or past [`MCP_ARGS_LIMIT`] as JSON, only its size and argument names, marked as
/// clipped. None of the text is kept: a cut through a secret would leave a prefix that redaction
/// can't recognize.
fn clipped(args: serde_json::Value) -> serde_json::Value {
    let bytes = args.to_string().len();
    if bytes <= MCP_ARGS_LIMIT {
        return args;
    }
    let keys: Vec<&String> = args
        .as_object()
        .map(|o| o.keys().collect())
        .unwrap_or_default();
    serde_json::json!({ "clipped": true, "bytes": bytes, "keys": keys })
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
    // An approval for an MCP tool call names only the call, which reported the tool.
    let mcp_approval = req
        .meta
        .as_ref()
        .and_then(|m| m.get("is_mcp_tool_approval")?.as_bool())
        .unwrap_or(false);
    if mcp_approval {
        return match commands.mcp(&req.tool_call.tool_call_id.0) {
            Some(action) => Target::Action(action.clone()),
            None => Target::Unknown,
        };
    }
    let Some(kind) = fields.kind else {
        return Target::Unknown;
    };
    match kind {
        acp::ToolKind::Execute => {
            let argv = match source {
                CommandSource::RawInput => raw_command(raw),
                CommandSource::Title => fields.title.as_deref().map(shell_line),
                CommandSource::ToolCallArgv => {
                    let id = &req.tool_call.tool_call_id.0;
                    match commands.get(id) {
                        Some(line) => Some(quoted_argv(line)),
                        // The approval's command, which loses its shell prefix where Codex
                        // recognizes the shell: a script left as one quoted word is the
                        // shell line it was.
                        None if commands.reported_read(id) => raw
                            .and_then(|r| r.get("command")?.as_str())
                            .map(|line| match quoted_argv(line).as_slice() {
                                [script] => shell_line(script),
                                _ => quoted_argv(line),
                            }),
                        None => None,
                    }
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
            let kind = if kind == acp::ToolKind::Delete {
                EditKind::Delete
            } else {
                EditKind::Modify
            };
            // Every path the request names: a patch or a move changes them all.
            let mut paths: Vec<(PathBuf, EditKind)> = fields
                .locations
                .iter()
                .flatten()
                .map(|l| (l.path.clone(), kind))
                .collect();
            if let Some(path) = raw.and_then(|r| r.get("path")?.as_str()) {
                paths.push((PathBuf::from(path), kind));
            }
            // And every file its tool call's diffs change: Codex names only a patch's sources in
            // the request, while its diffs name where moved files go, and which are deleted.
            let id = &req.tool_call.tool_call_id.0;
            match commands.changes(id) {
                Some(changes) => paths.extend(changes.iter().map(|c| {
                    let kind = if c.delete { EditKind::Delete } else { kind };
                    (c.path.clone(), kind)
                })),
                // Without its diffs (too large to report), what a Codex patch writes is unknown.
                None if source == CommandSource::ToolCallArgv => return Target::Unknown,
                None => {}
            }
            if paths.is_empty() {
                return Target::Unknown;
            }
            let mut edits: Vec<Action> = Vec::new();
            for (path, kind) in paths {
                // Resolved as a write would be, so the path decided is the file that would
                // really change: a symlink the agent planted can't carry an allowed edit
                // elsewhere. One path that can't be resolved leaves nothing to decide.
                let Ok(resolved) = workspace.resolve_write(&workspace.cwd().join(path)) else {
                    return Target::Unknown;
                };
                let edit = Action::Edit {
                    path: resolved.display,
                    kind,
                };
                if !edits.contains(&edit) {
                    edits.push(edit);
                }
            }
            match edits.len() {
                1 => Target::Action(edits.remove(0)),
                _ => Target::Edits(edits),
            }
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
    fn a_reported_read_is_decided_as_the_command_its_approval_names() {
        let (_dir, ws) = workspace();
        let full = r#"/usr/bin/bash -c "sed -n '1,5p' Cargo.toml""#;
        let req = request(acp::ToolKind::Execute, |f| {
            f.raw_input(serde_json::json!({ "command": full }))
        });
        // A call Codex never reported stays unknown.
        let mut commands = ToolCommands::default();
        assert_eq!(
            target(&req, &ws, CommandSource::ToolCallArgv, &commands),
            Target::Unknown
        );
        // One it reported as a read, which carries no command: the approval's own.
        commands.record_read("t1");
        assert_eq!(
            exec(target(&req, &ws, CommandSource::ToolCallArgv, &commands)),
            ["/usr/bin/bash", "-c", "sed -n '1,5p' Cargo.toml"]
        );
        // With its shell prefix stripped, the script is a shell line.
        for (line, script) in [
            (r#""sed -n '1,5p' Cargo.toml""#, "sed -n '1,5p' Cargo.toml"),
            ("cat a && rm b", "cat a && rm b"),
        ] {
            let req = request(acp::ToolKind::Execute, |f| {
                f.raw_input(serde_json::json!({ "command": line }))
            });
            assert_eq!(
                exec(target(&req, &ws, CommandSource::ToolCallArgv, &commands)),
                ["sh", "-c", script]
            );
        }
        // A command the call reported wins over the approval's.
        commands.record("t1", Some(&serde_json::json!({ "command": "cargo test" })));
        assert_eq!(
            exec(target(&req, &ws, CommandSource::ToolCallArgv, &commands)),
            ["cargo", "test"]
        );
    }

    #[test]
    fn an_mcp_tool_approval_is_the_tool_call_it_belongs_to() {
        let (_dir, ws) = workspace();
        let mut meta = serde_json::Map::new();
        meta.insert("is_mcp_tool_approval".into(), true.into());
        let req = RequestPermissionRequest::new(
            acp::SessionId::new("s"),
            acp::ToolCallUpdate::new(
                "t1",
                acp::ToolCallUpdateFields::new().kind(acp::ToolKind::Execute),
            ),
            vec![],
        )
        .meta(meta);
        let mut commands = ToolCommands::default();
        // Without the tool call: nothing to decide.
        assert_eq!(
            target(&req, &ws, CommandSource::ToolCallArgv, &commands),
            Target::Unknown
        );
        let args = serde_json::json!({ "message": "Add notes", "paths": ["NOTES.md"] });
        let input =
            serde_json::json!({ "server": "forge", "tool": "git_commit", "arguments": args });
        // A call that isn't reported as an MCP tool call doesn't count.
        commands.record_mcp("t1", None, Some(&input));
        assert_eq!(
            target(&req, &ws, CommandSource::ToolCallArgv, &commands),
            Target::Unknown
        );
        let mut call_meta = serde_json::Map::new();
        call_meta.insert("is_mcp_tool_call".into(), true.into());
        commands.record_mcp("t1", Some(&call_meta), Some(&input));
        assert_eq!(
            target(&req, &ws, CommandSource::ToolCallArgv, &commands),
            Target::Action(Action::Mcp {
                server: "forge".into(),
                tool: "git_commit".into(),
                args,
            })
        );
        // Without the approval's mark, the same request is a command with none reported.
        let plain = RequestPermissionRequest::new(
            acp::SessionId::new("s"),
            acp::ToolCallUpdate::new(
                "t1",
                acp::ToolCallUpdateFields::new().kind(acp::ToolKind::Execute),
            ),
            vec![],
        );
        assert_eq!(
            target(&plain, &ws, CommandSource::ToolCallArgv, &commands),
            Target::Unknown
        );
    }

    #[test]
    fn large_mcp_arguments_are_clipped_and_marked() {
        let mut meta = serde_json::Map::new();
        meta.insert("is_mcp_tool_call".into(), true.into());
        let big = "x".repeat(MCP_ARGS_LIMIT * 2);
        let input =
            serde_json::json!({ "server": "docs", "tool": "put", "arguments": { "body": big } });
        let mut commands = ToolCommands::default();
        commands.record_mcp("t1", Some(&meta), Some(&input));
        let Some(Action::Mcp { args, .. }) = commands.mcp("t1") else {
            panic!("not recorded");
        };
        assert_eq!(args["clipped"], true);
        assert_eq!(
            args["bytes"],
            serde_json::to_string(&input["arguments"]).unwrap().len()
        );
        // Only the argument names: a cut through a secret would leave a prefix the redaction
        // scanner can't match.
        assert_eq!(args["keys"], serde_json::json!(["body"]));
        assert!(!args.to_string().contains("xxx"));
        // Small ones are kept as given.
        let small =
            serde_json::json!({ "server": "docs", "tool": "get", "arguments": { "id": 1 } });
        commands.record_mcp("t2", Some(&meta), Some(&small));
        assert!(
            matches!(commands.mcp("t2"), Some(Action::Mcp { args, .. }) if args == &small["arguments"])
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
    fn a_patch_is_decided_on_every_path_it_names() {
        let (dir, ws) = workspace();
        let at = |p: &std::path::Path| acp::ToolCallLocation::new(p.to_path_buf());
        let edit = |path: &str| Action::Edit {
            path: path.into(),
            kind: EditKind::Modify,
        };
        let req = request(acp::ToolKind::Edit, |f| {
            f.locations(vec![
                at(&ws.cwd().join("a.rs")),
                at(&ws.cwd().join(".github/workflows/ci.yml")),
                at(&ws.cwd().join("a.rs")),
            ])
        });
        assert_eq!(
            decide(&req, &ws, CommandSource::RawInput),
            Target::Edits(vec![edit("a.rs"), edit(".github/workflows/ci.yml")])
        );
        // One path out of the worktree, wherever it comes, leaves nothing to allow.
        let req = request(acp::ToolKind::Edit, |f| {
            f.locations(vec![at(&ws.cwd().join("a.rs")), at(&dir.path().join("x"))])
        });
        assert_eq!(decide(&req, &ws, CommandSource::RawInput), Target::Unknown);
        // A move names both ends; a delete deletes each.
        let req = request(acp::ToolKind::Delete, |f| {
            f.locations(vec![at(&ws.cwd().join("a.rs")), at(&ws.cwd().join("b.rs"))])
        });
        let delete = |path: &str| Action::Edit {
            path: path.into(),
            kind: EditKind::Delete,
        };
        assert_eq!(
            decide(&req, &ws, CommandSource::RawInput),
            Target::Edits(vec![delete("a.rs"), delete("b.rs")])
        );
        let req = request(acp::ToolKind::Edit, |f| f);
        assert_eq!(decide(&req, &ws, CommandSource::RawInput), Target::Unknown);
    }

    #[test]
    fn a_codex_patch_is_decided_on_where_its_diffs_move_and_delete_files() {
        let (_dir, ws) = workspace();
        let mut commands = ToolCommands::default();
        let diff = |path: &std::path::Path, kind: &str| {
            let mut meta = serde_json::Map::new();
            meta.insert("kind".into(), kind.into());
            acp::ToolCallContent::Diff(acp::Diff::new(path.to_path_buf(), "x").meta(meta))
        };
        // Diffs reported over two updates both count.
        commands.record_changes(
            "t1",
            &[diff(&ws.cwd().join(".github/workflows/ci.yml"), "update")],
        );
        commands.record_changes("t1", &[diff(&ws.cwd().join("old.rs"), "delete")]);
        // The request itself names only the patch's source.
        let req = request(acp::ToolKind::Edit, |f| {
            f.locations(vec![acp::ToolCallLocation::new(ws.cwd().join("a.rs"))])
        });
        let edit = |path: &str, kind| Action::Edit {
            path: path.into(),
            kind,
        };
        assert_eq!(
            target(&req, &ws, CommandSource::ToolCallArgv, &commands),
            Target::Edits(vec![
                edit("a.rs", EditKind::Modify),
                edit(".github/workflows/ci.yml", EditKind::Modify),
                edit("old.rs", EditKind::Delete),
            ])
        );
        // A Codex patch whose diffs weren't reported names nothing Forge can allow.
        let none = ToolCommands::default();
        assert_eq!(
            target(&req, &ws, CommandSource::ToolCallArgv, &none),
            Target::Unknown
        );
        // Other agents' edits are decided on what the request names.
        assert_eq!(
            target(&req, &ws, CommandSource::RawInput, &none),
            Target::Action(edit("a.rs", EditKind::Modify))
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
