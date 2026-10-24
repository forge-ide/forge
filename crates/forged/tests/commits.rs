//! `git_commit` through Forge's MCP endpoint: the fake agent calls it like a real agent
//! would, and the daemon commits on the session branch after policy and `commit.pre`.

mod common;

use std::path::{Path, PathBuf};

use common::{
    Daemon, all_events, collect_until, create_params, fake_profile, git, git_repo_with_commit,
    open, subscribe, temp_home, wait_for_state,
};
use forge_api::client::Client;
use forge_api::events::{ApprovalDecision, Event, EventKind, EventPayload, Topic, Verdict};
use forge_api::methods::*;
use forge_api::model::{Action, Mode, Project, Session, SessionState};

/// Writes a file through the client, then commits it with `git_commit`.
fn write_then_commit(expect: &str) -> String {
    format!(
        r#"
steps:
  - write: {{ path: src/f.rs, content: "pub fn f() -> u32 {{ 1 }}\n" }}
  - mcp_call: {{ tool: git_commit, arguments: {{ message: "add f" }}, expect: {expect} }}
  - end_turn
"#
    )
}

fn daemon(home: &Path) -> Daemon {
    Daemon::start_with(home, |cmd| {
        cmd.env("GIT_AUTHOR_NAME", "Dev")
            .env("GIT_AUTHOR_EMAIL", "dev@example.com")
            .env("GIT_COMMITTER_NAME", "Dev")
            .env("GIT_COMMITTER_EMAIL", "dev@example.com");
    })
}

async fn start(client: &mut Client, project: &Project, mode: Mode) -> Session {
    let mut params = create_params(project, "feat: commit", "go");
    params.mode = mode;
    let session = client.call::<SessionCreate>(&params).await.unwrap();
    subscribe(client, vec![Topic::Session(session.id.clone())], Some(0)).await;
    session
}

fn worktree(repo: &Path, session: &Session) -> PathBuf {
    repo.join(".worktrees").join(&session.branch)
}

fn decisions(events: &[Event]) -> Vec<(Verdict, String)> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::PolicyDecision(b) => Some((b.verdict, b.source.clone())),
            _ => None,
        })
        .collect()
}

/// Every turn ended normally: the fake agent fails a turn whose script didn't go as written.
fn assert_turns_ended(events: &[Event]) {
    let stops: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TurnEnded(b) => Some(b.stop_reason.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        !stops.is_empty() && stops.iter().all(|s| *s == "end_turn"),
        "{stops:?}"
    );
}

fn git_calls(events: &[Event]) -> usize {
    events
        .iter()
        .filter(|e| matches!(&e.payload, EventPayload::ToolCall(b) if b.action == Action::Git { op: "commit".into() }))
        .count()
}

#[tokio::test]
async fn an_agent_commits_through_git_commit_after_a_human_approves() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", &write_then_commit("allow"));
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::AcceptEdits).await;

    // Accept edits allows the write and asks about the commit.
    let notes = collect_until(&mut client, |e| {
        e.payload.kind() == EventKind::ApprovalRequested
    })
    .await;
    let EventPayload::ApprovalRequested(asked) = &notes.last().unwrap().event.payload else {
        unreachable!()
    };
    assert_eq!(
        asked.action,
        Action::Git {
            op: "commit".into()
        }
    );
    // The human sees what the commit changes, not only the agent's message.
    assert!(
        asked.reason.contains("changes src/f.rs"),
        "{}",
        asked.reason
    );
    let main_before = git(&repo, &["rev-parse", "main"]);
    let branch_before = git(&repo, &["rev-parse", &session.branch]);
    assert_eq!(
        main_before, branch_before,
        "nothing committed before the approval"
    );
    client
        .call::<ApprovalDecide>(&ApprovalDecideParams {
            approval_id: asked.approval_id.clone(),
            decision: ApprovalDecision::Allow,
            remember: None,
            note: None,
        })
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Review).await;

    // The commit is on the session branch, with the agent's message and the new file; the
    // base branch and the main checkout are untouched, and the worktree is clean.
    assert_eq!(
        git(&repo, &["log", "-1", "--format=%s", &session.branch]).trim(),
        "add f"
    );
    assert_eq!(
        git(
            &repo,
            &["show", "--name-only", "--format=", &session.branch]
        )
        .trim(),
        "src/f.rs"
    );
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before);
    assert!(!repo.join("src/f.rs").exists());
    assert_eq!(
        git(&worktree(&repo, &session), &["status", "--porcelain"]),
        ""
    );

    let events = all_events(&mut client, &session.id).await;
    assert_turns_ended(&events);
    assert_eq!(git_calls(&events), 1);
    assert!(
        decisions(&events).contains(&(Verdict::Ask, "mode:accept-edits".into())),
        "{:?}",
        decisions(&events)
    );
    daemon.stop();
}

#[tokio::test]
async fn protected_paths_commit_pre_hooks_and_denials_keep_the_branch_where_it_was() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    // A commit.pre hook that blocks commits touching src/blocked.rs.
    std::fs::create_dir_all(repo.join(".forge/hooks")).unwrap();
    let hook = repo.join(".forge/hooks/check.sh");
    std::fs::write(
        &hook,
        "#!/bin/sh\ncase \"$(cat)\" in *blocked.rs*) echo '{\"verdict\":\"deny\",\"reason\":\"tests fail\"}';; esac\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        repo.join(".forge/hooks.toml"),
        "[[hook]]\nevent = \"commit.pre\"\nname = \"check\"\nrun = [\".forge/hooks/check.sh\"]\n",
    )
    .unwrap();
    git(&repo, &["add", ".forge"]);
    git(&repo, &["commit", "--quiet", "-m", "hooks"]);
    // In Full access the commit itself would be allowed; each of these is refused anyway.
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - exec: ["sh", "-c", "echo 'use flake' > .envrc"]
  - mcp_call: { tool: git_commit, arguments: { message: "envrc" }, expect: deny }
  - exec: ["rm", ".envrc"]
  - write: { path: src/blocked.rs, content: "x\n" }
  - mcp_call: { tool: git_commit, arguments: { message: "blocked" }, expect: deny }
  - mcp_call: { tool: git_commit, arguments: { message: "outside", paths: ["../x"] }, expect: deny }
  - mcp_call: { tool: git_commit, arguments: { message: "nothing", paths: ["README.md"] }, expect: deny }
  - end_turn
"#,
    );
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::FullAccess).await;
    // The session branch starts at main.
    let before = git(&repo, &["rev-parse", "main"]);
    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    assert_eq!(git(&repo, &["rev-parse", &session.branch]), before);

    let events = all_events(&mut client, &session.id).await;
    assert_turns_ended(&events);
    assert_eq!(git_calls(&events), 4);
    let decided = decisions(&events);
    assert!(
        decided.contains(&(Verdict::Deny, "guard:protected-path".into())),
        "{decided:?}"
    );
    assert!(
        decided.contains(&(Verdict::Deny, "hook:check".into())),
        "{decided:?}"
    );
    daemon.stop();
}

#[tokio::test]
async fn a_human_denial_refuses_the_commit() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", &write_then_commit("deny"));
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::Ask).await;
    // Ask mode: first the write, then the commit.
    for _ in 0..2 {
        let notes = collect_until(&mut client, |e| {
            e.payload.kind() == EventKind::ApprovalRequested
        })
        .await;
        let EventPayload::ApprovalRequested(asked) = &notes.last().unwrap().event.payload else {
            unreachable!()
        };
        let decision = match asked.action {
            Action::Git { .. } => ApprovalDecision::Deny,
            _ => ApprovalDecision::Allow,
        };
        client
            .call::<ApprovalDecide>(&ApprovalDecideParams {
                approval_id: asked.approval_id.clone(),
                decision,
                remember: None,
                note: Some("not yet".into()),
            })
            .await
            .unwrap();
    }
    let before = git(&repo, &["rev-parse", "main"]);
    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    assert_eq!(git(&repo, &["rev-parse", &session.branch]), before);
    assert_turns_ended(&all_events(&mut client, &session.id).await);
    daemon.stop();
}

#[tokio::test]
async fn a_worktree_that_changes_while_the_commit_is_checked_is_not_committed() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    // A commit.pre hook that passes, after changing what it checked.
    std::fs::create_dir_all(repo.join(".forge/hooks")).unwrap();
    let hook = repo.join(".forge/hooks/flip.sh");
    std::fs::write(
        &hook,
        "#!/bin/sh\ncat >/dev/null\necho flipped >> src/f.rs\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        repo.join(".forge/hooks.toml"),
        "[[hook]]\nevent = \"commit.pre\"\nrun = [\".forge/hooks/flip.sh\"]\n",
    )
    .unwrap();
    git(&repo, &["add", ".forge"]);
    git(&repo, &["commit", "--quiet", "-m", "hooks"]);
    fake_profile(&repo, "fake", &write_then_commit("deny"));
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::FullAccess).await;
    let before = git(&repo, &["rev-parse", "main"]);
    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    assert_eq!(git(&repo, &["rev-parse", &session.branch]), before);
    let events = all_events(&mut client, &session.id).await;
    assert_turns_ended(&events);
    let refused = events.iter().any(|e| {
        matches!(&e.payload,
            EventPayload::ToolResult(r) if r.status == "failed"
                && r.output.as_deref().is_some_and(|o| o.contains("changed while the commit")))
    });
    assert!(refused, "{events:?}");
    daemon.stop();
}
