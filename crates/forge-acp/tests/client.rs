//! The ACP client against the fake agent binary.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use forge_acp::adapters::CommandSource;
use forge_acp::{
    AcpClient, AcpError, AgentCommand, AllowAll, DenyAll, EventSink, Gate, GateKind, GateRequest,
    GateVerdict, McpEndpoint, Root, SessionSetup, TerminalSignIn, Workspace,
};
use forge_api::TurnId;
use forge_api::events::{EventKind, EventPayload};
use forge_api::model::{Action, EditKind};
use futures_util::future::BoxFuture;

/// The fake agent binary, built on first use next to this test's own profile.
fn fake_agent() -> &'static Path {
    static BIN: OnceLock<PathBuf> = OnceLock::new();
    BIN.get_or_init(|| {
        let exe = std::env::current_exe().unwrap();
        // target/<profile>/deps/client-<hash>
        let profile_dir = exe.parent().unwrap().parent().unwrap().to_path_buf();
        let mut cmd = Command::new(env!("CARGO"));
        cmd.args(["build", "--quiet", "-p", "forge-fake-agent"]);
        if profile_dir.file_name() == Some("release".as_ref()) {
            cmd.arg("--release");
        }
        assert!(cmd.status().unwrap().success());
        profile_dir.join("forge-fake-agent")
    })
}

#[derive(Default)]
struct Collect(Mutex<Vec<EventPayload>>);

impl EventSink for Collect {
    fn emit(&self, payload: EventPayload) -> BoxFuture<'_, ()> {
        self.0.lock().unwrap().push(payload);
        Box::pin(async {})
    }
}

impl Collect {
    fn kinds(&self) -> Vec<EventKind> {
        self.0.lock().unwrap().iter().map(|p| p.kind()).collect()
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    sink: Arc<Collect>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("wt")).unwrap();
        std::fs::write(dir.path().join("wt/README.md"), "# hi\n").unwrap();
        Self {
            dir,
            sink: Arc::new(Collect::default()),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("wt").canonicalize().unwrap()
    }

    async fn start(&self, script: &str) -> AcpClient {
        self.start_with(script, Arc::new(AllowAll)).await
    }

    async fn start_with(&self, script: &str, gate: Arc<dyn Gate>) -> AcpClient {
        self.start_setup(script, gate, SessionSetup::default())
            .await
    }

    async fn start_setup(
        &self,
        script: &str,
        gate: Arc<dyn Gate>,
        setup: SessionSetup,
    ) -> AcpClient {
        let path = self.dir.path().join("script.yaml");
        std::fs::write(&path, script).unwrap();
        let workspace = Workspace::new(vec![Root {
            name: "wt".into(),
            path: self.root(),
        }])
        .unwrap();
        let command = AgentCommand {
            argv: vec![
                fake_agent().to_str().unwrap().into(),
                path.to_str().unwrap().into(),
            ],
            env: vec![],
        };
        AcpClient::start(
            command,
            setup,
            workspace,
            unsandboxed(),
            self.sink.clone(),
            gate,
        )
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn a_turn_reads_writes_runs_and_reports_events() {
    let fx = Fixture::new();
    let client = fx
        .start(
            r##"
steps:
  - expect_prompt: "*go*"
  - plan: ["one", "two"]
  - read: README.md
  - write: { path: a.txt, content: "a\n" }
  - write: { path: README.md, content: "# hi\nmore\n" }
  - write: { path: sub/dir/b.txt, content: "b\n" }
  - exec: ["sh", "-c", "cat a.txt; exit 2"]
  - message: "done"
  - end_turn
"##,
        )
        .await;
    let stop = client.prompt(TurnId::new(), "go now".into()).await.unwrap();
    assert_eq!(stop, "end_turn");
    assert_eq!(
        std::fs::read_to_string(fx.root().join("a.txt")).unwrap(),
        "a\n"
    );
    assert_eq!(
        std::fs::read_to_string(fx.root().join("sub/dir/b.txt")).unwrap(),
        "b\n"
    );

    use EventKind::*;
    assert_eq!(
        fx.sink.kinds(),
        [
            AgentPlan,
            ToolCall,
            Edit,
            ToolResult,
            ToolCall,
            Edit,
            ToolResult,
            ToolCall,
            Edit,
            ToolResult,
            ToolCall,
            TerminalOutput,
            ToolResult,
            AgentMessage
        ]
    );
    let events = fx.sink.0.lock().unwrap().clone();
    let edits: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            EventPayload::Edit(b) => Some((b.path.as_str(), b.kind, b.additions, b.deletions)),
            _ => None,
        })
        .collect();
    assert_eq!(
        edits,
        [
            ("a.txt", EditKind::Create, 1, 0),
            ("README.md", EditKind::Modify, 1, 0),
            ("sub/dir/b.txt", EditKind::Create, 1, 0),
        ]
    );
    match &events[12] {
        EventPayload::ToolResult(b) => {
            assert_eq!(b.status, "failed", "non-zero exit");
            assert_eq!(b.output.as_deref(), Some("a\n"));
        }
        other => panic!("{other:?}"),
    }
    match &events[10] {
        EventPayload::ToolCall(b) => assert_eq!(
            b.action,
            Action::Exec {
                argv: vec!["sh".into(), "-c".into(), "cat a.txt; exit 2".into()],
                cwd: ".".into()
            }
        ),
        other => panic!("{other:?}"),
    }
    client.shutdown().await;
}

#[tokio::test]
async fn writes_outside_the_worktree_fail_and_leave_nothing() {
    let fx = Fixture::new();
    std::fs::create_dir(fx.dir.path().join("outside")).unwrap();
    std::os::unix::fs::symlink(fx.dir.path().join("outside"), fx.root().join("link")).unwrap();
    for path in ["../escaped.txt", "link/escaped.txt", ".git/config"] {
        let client = fx
            .start(&format!(
                "steps:\n  - write: {{ path: {path:?}, content: x }}\n  - end_turn\n"
            ))
            .await;
        let result = client.prompt(TurnId::new(), "go".into()).await;
        assert!(result.is_err(), "{path} was written");
        client.shutdown().await;
    }
    assert!(!fx.dir.path().join("escaped.txt").exists());
    assert!(!fx.dir.path().join("outside/escaped.txt").exists());
    assert!(!fx.root().join(".git").exists());
    assert!(
        !fx.sink.kinds().contains(&EventKind::Edit),
        "refused writes are not edits"
    );
}

/// Records what it is asked and answers with a fixed verdict.
struct Recording {
    verdict: GateVerdict,
    asked: Mutex<Vec<GateRequest>>,
}

impl Gate for Recording {
    fn decide(&self, request: GateRequest) -> BoxFuture<'_, GateVerdict> {
        self.asked.lock().unwrap().push(request);
        let verdict = self.verdict.clone();
        Box::pin(async move { verdict })
    }
}

#[tokio::test]
async fn the_gate_decides_permission_requests_writes_and_terminals() {
    let fx = Fixture::new();
    let gate = Arc::new(Recording {
        verdict: GateVerdict::Deny("no".into()),
        asked: Mutex::default(),
    });
    let client = fx
        .start_with(
            r#"
steps:
  - request_permission: { action: exec, argv: ["pnpm", "add", "left-pad"] }
  - expect_permission: deny
  - request_permission: { action: edit, path: "sub/../notes.txt" }
  - write: { path: a.txt, content: "secret?", expect: deny }
  - exec: { argv: ["touch", "ran"], expect: deny }
  - end_turn
"#,
            gate.clone(),
        )
        .await;
    assert_eq!(
        client.prompt(TurnId::new(), "x".into()).await.unwrap(),
        "end_turn"
    );
    assert!(!fx.root().join("a.txt").exists());
    assert!(!fx.root().join("ran").exists());

    let asked = gate.asked.lock().unwrap().clone();
    let seen: Vec<_> = asked
        .iter()
        .map(|r| (r.kind, r.action.clone(), r.content.clone()))
        .collect();
    assert_eq!(
        seen,
        [
            (
                GateKind::Permission,
                Some(Action::Exec {
                    argv: vec!["pnpm".into(), "add".into(), "left-pad".into()],
                    cwd: ".".into()
                }),
                None
            ),
            (
                GateKind::Permission,
                Some(Action::Edit {
                    path: "notes.txt".into(),
                    kind: EditKind::Modify
                }),
                None
            ),
            (
                GateKind::Write,
                Some(Action::Edit {
                    path: "a.txt".into(),
                    kind: EditKind::Create
                }),
                Some("secret?".into())
            ),
            (
                GateKind::Terminal,
                Some(Action::Exec {
                    argv: vec!["touch".into(), "ran".into()],
                    cwd: ".".into()
                }),
                None
            ),
        ]
    );
    // Each refused write or command ends with a failed result naming the reason.
    use EventKind::*;
    assert_eq!(
        fx.sink.kinds(),
        [
            ToolCall, ToolCall, ToolCall, ToolResult, ToolCall, ToolResult
        ]
    );
    let events = fx.sink.0.lock().unwrap().clone();
    assert!(matches!(&events[3], EventPayload::ToolResult(b)
        if b.status == "failed" && b.output.as_deref() == Some("refused by policy: no")));
    client.shutdown().await;
}

#[tokio::test]
async fn codex_reads_and_forge_tool_approvals_name_what_they_ask() {
    let fx = Fixture::new();
    let gate = Arc::new(Recording {
        verdict: GateVerdict::Allow,
        asked: Mutex::default(),
    });
    let setup = SessionSetup {
        command_source: CommandSource::ToolCallArgv,
        mcp_servers: vec![McpEndpoint {
            name: "forge".into(),
            url: "http://127.0.0.1:9/mcp".into(),
        }],
        ..SessionSetup::default()
    };
    let client = fx
        .start_setup(
            r#"
steps:
  - request_permission: { action: parsed_read, path: README.md, command: "/usr/bin/bash -c \"sed -n '1,5p' README.md\"" }
  - expect_permission: allow
  - request_permission: { action: mcp_tool, tool: git_commit }
  - expect_permission: allow
  - end_turn
"#,
            gate.clone(),
            setup,
        )
        .await;
    assert_eq!(
        client.prompt(TurnId::new(), "x".into()).await.unwrap(),
        "end_turn"
    );
    // The read is decided as the command its approval names; Forge's own tool isn't asked
    // about here, since it asks Forge itself when it runs.
    let asked: Vec<_> = gate
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.action.clone())
        .collect();
    assert_eq!(
        asked,
        [Some(Action::Exec {
            argv: vec![
                "/usr/bin/bash".into(),
                "-c".into(),
                "sed -n '1,5p' README.md".into()
            ],
            cwd: ".".into()
        })]
    );
    client.shutdown().await;
}

#[tokio::test]
async fn an_agent_that_needs_signing_in_offers_its_methods_and_signs_in_with_authenticate() {
    let fx = Fixture::new();
    let home = fx.dir.path().join("agent-home");
    let script = fx.dir.path().join("auth.yaml");
    std::fs::write(
        &script,
        r#"
auth:
  login: .fake/login
  methods:
    - { id: browser, name: "Log in with a browser" }
    - { id: cli, name: "Log in in a terminal", kind: terminal, args: ["--login"] }
steps:
  - end_turn
"#,
    )
    .unwrap();
    let command = || AgentCommand {
        argv: vec![
            fake_agent().to_str().unwrap().into(),
            script.to_str().unwrap().into(),
        ],
        env: vec![("FORGE_FAKE_AGENT_HOME".into(), home.display().to_string())],
    };
    let workspace = || {
        Workspace::new(vec![Root {
            name: "wt".into(),
            path: fx.root(),
        }])
        .unwrap()
    };
    let open = async || {
        AcpClient::start(
            command(),
            SessionSetup::default(),
            workspace(),
            unsandboxed(),
            fx.sink.clone(),
            Arc::new(AllowAll),
        )
        .await
    };
    // Not signed in: the session is refused, naming both ways to sign in.
    let methods = match open().await {
        Err(AcpError::AuthRequired { methods }) => methods,
        other => panic!("expected auth_required, got {:?}", other.err()),
    };
    let listed = AcpClient::sign_in_methods(command(), &fx.root(), unsandboxed())
        .await
        .unwrap();
    assert_eq!(methods, listed);
    assert_eq!(
        listed
            .iter()
            .map(|m| (m.id.as_str(), m.terminal.clone()))
            .collect::<Vec<_>>(),
        [
            ("browser", None),
            (
                "cli",
                Some(TerminalSignIn {
                    args: vec!["--login".to_owned()],
                    env: vec![]
                })
            )
        ]
    );
    // A terminal method is never sent as `authenticate`, nor is one the agent doesn't offer.
    for id in ["cli", "other"] {
        assert!(
            AcpClient::authenticate(command(), &fx.root(), unsandboxed(), id)
                .await
                .is_err(),
            "{id}"
        );
    }
    assert!(!home.join(".fake/login").exists());
    // The agent's own method signs it in, and it keeps the login for the next process.
    AcpClient::authenticate(command(), &fx.root(), unsandboxed(), "browser")
        .await
        .unwrap();
    assert!(home.join(".fake/login").is_file());
    let client = open().await.unwrap();
    client.shutdown().await;
}

#[tokio::test]
async fn permission_edits_through_a_symlink_or_out_of_the_worktree_map_to_no_action() {
    let fx = Fixture::new();
    std::fs::write(fx.root().join(".env"), "TOKEN=1\n").unwrap();
    std::os::unix::fs::symlink(".env", fx.root().join("notes.txt")).unwrap();
    let gate = Arc::new(Recording {
        verdict: GateVerdict::Deny("no".into()),
        asked: Mutex::default(),
    });
    let client = fx
        .start_with(
            r#"
steps:
  - request_permission: { action: edit, path: notes.txt }
  - request_permission: { action: edit, path: ../outside.txt }
  - end_turn
"#,
            gate.clone(),
        )
        .await;
    client.prompt(TurnId::new(), "x".into()).await.unwrap();
    let actions: Vec<_> = gate
        .asked
        .lock()
        .unwrap()
        .iter()
        .map(|r| r.action.clone())
        .collect();
    assert_eq!(actions, [None, None]);
    client.shutdown().await;
}

/// Swaps `sub` for a symlink to `outside` while a write waits for its decision, as an agent's
/// own process could, then allows it.
struct SwapWhileWaiting {
    root: PathBuf,
    outside: PathBuf,
}

impl Gate for SwapWhileWaiting {
    fn decide(&self, request: GateRequest) -> BoxFuture<'_, GateVerdict> {
        if request.kind == GateKind::Write {
            std::fs::rename(self.root.join("sub"), self.root.join("sub-old")).unwrap();
            std::os::unix::fs::symlink(&self.outside, self.root.join("sub")).unwrap();
        }
        Box::pin(async { GateVerdict::Allow })
    }
}

#[tokio::test]
async fn a_path_swapped_for_a_symlink_while_waiting_is_not_written() {
    let fx = Fixture::new();
    let outside = fx.dir.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::create_dir(fx.root().join("sub")).unwrap();
    let gate = Arc::new(SwapWhileWaiting {
        root: fx.root(),
        outside: outside.clone(),
    });
    let client = fx
        .start_with(
            "steps:\n  - write: { path: sub/b.txt, content: x, expect: deny }\n  - end_turn\n",
            gate,
        )
        .await;
    assert_eq!(
        client.prompt(TurnId::new(), "x".into()).await.unwrap(),
        "end_turn"
    );
    assert!(!outside.join("b.txt").exists());
    assert!(!fx.root().join("sub-old/b.txt").exists());
    client.shutdown().await;
}

#[tokio::test]
async fn a_cancelled_permission_request_answers_cancelled() {
    let fx = Fixture::new();
    let gate = Arc::new(Recording {
        verdict: GateVerdict::Cancelled,
        asked: Mutex::default(),
    });
    let client = fx
        .start_with(
            r#"
steps:
  - request_permission: { action: exec, argv: ["ls"] }
  - expect_permission: deny
  - end_turn
"#,
            gate,
        )
        .await;
    assert_eq!(
        client.prompt(TurnId::new(), "x".into()).await.unwrap(),
        "end_turn"
    );
    client.shutdown().await;
}

#[tokio::test]
async fn cancel_ends_the_turn() {
    let fx = Fixture::new();
    let client = Arc::new(
        fx.start("steps:\n  - sleep_ms: 60000\n  - end_turn\n  - message: next\n  - end_turn\n")
            .await,
    );
    let turn = tokio::spawn({
        let client = client.clone();
        async move { client.prompt(TurnId::new(), "x".into()).await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    client.cancel().unwrap();
    let stop = tokio::time::timeout(Duration::from_secs(10), turn)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(stop, "cancelled");
    // The next prompt starts the script's next turn.
    assert_eq!(
        client.prompt(TurnId::new(), "y".into()).await.unwrap(),
        "end_turn"
    );
    assert_eq!(fx.sink.kinds(), [EventKind::AgentMessage]);
    client.shutdown().await;
}

#[tokio::test]
async fn an_agent_that_dies_reports_its_exit() {
    let fx = Fixture::new();
    let workspace = Workspace::new(vec![Root {
        name: "wt".into(),
        path: fx.root(),
    }])
    .unwrap();
    let command = AgentCommand {
        argv: vec!["sh".into(), "-c".into(), "echo boom >&2; exit 7".into()],
        env: vec![],
    };
    let deny = Arc::new(DenyAll {
        reason: String::new(),
    });
    let err = AcpClient::start(
        command,
        SessionSetup::default(),
        workspace,
        unsandboxed(),
        fx.sink.clone(),
        deny,
    )
    .await
    .err()
    .expect("start fails");
    match err {
        AcpError::Exited(exit) => {
            assert_eq!(exit.code, Some(7));
            assert_eq!(exit.stderr, ["boom"]);
            assert_eq!(exit.to_string(), "agent exited with code 7: boom");
        }
        other => panic!("{other:?}"),
    }
}

fn unsandboxed() -> Arc<dyn forge_sandbox::SessionSandbox> {
    Arc::new(forge_sandbox::Unsandboxed::new())
}
