//! The ACP client against the fake agent binary.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use forge_acp::{AcpClient, AcpError, AgentCommand, DenyAll, EventSink, Root, Workspace};
use forge_api::TurnId;
use forge_api::events::{EventKind, EventPayload, Verdict};
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
        let deny = Arc::new(DenyAll {
            reason: "no policy".into(),
        });
        AcpClient::start(command, workspace, unsandboxed(), self.sink.clone(), deny)
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

#[tokio::test]
async fn permission_requests_are_denied_and_logged() {
    let fx = Fixture::new();
    let client = fx
        .start(
            r#"
steps:
  - request_permission: { action: exec, argv: ["pnpm", "add", "left-pad"] }
  - expect_permission: deny
  - end_turn
"#,
        )
        .await;
    assert_eq!(
        client.prompt(TurnId::new(), "x".into()).await.unwrap(),
        "end_turn"
    );
    let events = fx.sink.0.lock().unwrap().clone();
    assert!(matches!(&events[0], EventPayload::ToolCall(b)
        if b.action == Action::Exec { argv: vec!["pnpm".into(), "add".into(), "left-pad".into()], cwd: ".".into() }));
    match &events[1] {
        EventPayload::PolicyDecision(b) => {
            assert_eq!(b.verdict, Verdict::Deny);
            assert_eq!(b.reason, "no policy");
        }
        other => panic!("{other:?}"),
    }
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
    let err = AcpClient::start(command, workspace, unsandboxed(), fx.sink.clone(), deny)
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
