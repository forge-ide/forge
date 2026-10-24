//! Approval tests against a real daemon driving the fake agent: an Ask blocks the agent until
//! `approval.decide`, writes and commands pass the policy engine, remembered decisions,
//! cancelling while waiting, `session.setMode`, and pending approvals across a restart.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use common::{
    Daemon, all_events, collect_until, create_params, entered, expect_api_error, fake_profile,
    git_repo_with_commit, open, subscribe, temp_home, wait_for_state,
};
use forge_api::client::Client;
use forge_api::events::{
    ApprovalDecision, Event, EventKind, EventPayload, Remember, Topic, Verdict,
};
use forge_api::methods::*;
use forge_api::model::{Action, Approval, Mode, Project, SandboxTier, Session, SessionState};
use forge_api::{ApprovalId, ErrorKind};

/// Asks for a command no rule allows, then writes a file only if it was allowed.
const ASK_THEN_WRITE: &str = r#"
steps:
  - expect_prompt: "*"
  - request_permission: { action: exec, argv: ["pnpm", "add", "left-pad"] }
  - expect_permission: allow
  - write: { path: after.txt, content: "allowed\n" }
  - end_turn
"#;

async fn start(client: &mut Client, project: &Project, mode: Mode, prompt: &str) -> Session {
    let mut params = create_params(project, "feat: approvals", prompt);
    params.mode = mode;
    let session = client.call::<SessionCreate>(&params).await.unwrap();
    subscribe(client, vec![Topic::Session(session.id.clone())], Some(0)).await;
    session
}

async fn pending(client: &mut Client, project: Option<&Project>) -> Vec<Approval> {
    client
        .call::<ApprovalList>(&ApprovalListParams {
            project_id: project.map(|p| p.id.clone()),
        })
        .await
        .unwrap()
}

/// Wait for the next `approval.requested` and return its ID and action.
async fn next_request(client: &mut Client) -> (ApprovalId, Action) {
    let notes = collect_until(client, |e| e.payload.kind() == EventKind::ApprovalRequested).await;
    match &notes.last().unwrap().event.payload {
        EventPayload::ApprovalRequested(b) => (b.approval_id.clone(), b.action.clone()),
        _ => unreachable!(),
    }
}

async fn decide(
    client: &mut Client,
    id: &ApprovalId,
    decision: ApprovalDecision,
    remember: Option<Remember>,
) {
    client
        .call::<ApprovalDecide>(&ApprovalDecideParams {
            approval_id: id.clone(),
            decision,
            remember,
            note: None,
        })
        .await
        .unwrap();
}

async fn state(client: &mut Client, session: &Session) -> SessionState {
    client
        .call::<SessionGet>(&SessionIdParams {
            session_id: session.id.clone(),
        })
        .await
        .unwrap()
        .state
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

fn count(events: &[Event], kind: EventKind) -> usize {
    events.iter().filter(|e| e.payload.kind() == kind).count()
}

#[tokio::test]
async fn an_ask_blocks_the_agent_until_approval_decide_arrives() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", ASK_THEN_WRITE);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::AcceptEdits, "go").await;

    let (id, action) = next_request(&mut client).await;
    assert_eq!(
        action,
        Action::Exec {
            argv: vec!["pnpm".into(), "add".into(), "left-pad".into()],
            cwd: ".".into(),
        }
    );
    collect_until(&mut client, entered(SessionState::Waiting)).await;
    let listed = pending(&mut client, None).await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].session_id, session.id);
    assert_eq!(listed[0].action, action);
    assert_eq!(pending(&mut client, Some(&project)).await.len(), 1);

    // The agent stays blocked: nothing after the request happens while no one decides.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(state(&mut client, &session).await, SessionState::Waiting);
    assert!(!worktree(&repo, &session).join("after.txt").exists());

    decide(&mut client, &id, ApprovalDecision::Allow, None).await;
    collect_until(&mut client, entered(SessionState::Review)).await;
    assert_eq!(
        std::fs::read_to_string(worktree(&repo, &session).join("after.txt")).unwrap(),
        "allowed\n"
    );
    assert!(pending(&mut client, None).await.is_empty());

    let events = all_events(&mut client, &session.id).await;
    let states: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::SessionState(b) => Some((b.from, b.to)),
            _ => None,
        })
        .collect();
    use SessionState::*;
    assert_eq!(
        states,
        [
            (Provisioning, Ready),
            (Ready, Working),
            (Working, Waiting),
            (Waiting, Working),
            (Working, Review),
        ]
    );
    // The ask, then the write the mode allows.
    assert_eq!(
        decisions(&events),
        [
            (Verdict::Ask, "mode:accept-edits".to_owned()),
            (Verdict::Allow, "mode:accept-edits".to_owned()),
        ]
    );
    let decided: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ApprovalDecided(b) => Some((b.approval_id.clone(), b.decision)),
            _ => None,
        })
        .collect();
    assert_eq!(decided, [(id.clone(), ApprovalDecision::Allow)]);

    // A decided approval can't be decided again.
    let again = client
        .call::<ApprovalDecide>(&ApprovalDecideParams {
            approval_id: id,
            decision: ApprovalDecision::Deny,
            remember: None,
            note: None,
        })
        .await;
    expect_api_error(again, ErrorKind::InvalidState);
    let unknown = client
        .call::<ApprovalDecide>(&ApprovalDecideParams {
            approval_id: ApprovalId::new(),
            decision: ApprovalDecision::Deny,
            remember: None,
            note: None,
        })
        .await;
    expect_api_error(unknown, ErrorKind::NotFound);
}

#[tokio::test]
async fn a_denied_approval_refuses_the_agent() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    let script = ASK_THEN_WRITE.replace("expect_permission: allow", "expect_permission: deny");
    fake_profile(&repo, "fake", &script);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::AcceptEdits, "go").await;

    let (id, _) = next_request(&mut client).await;
    decide(&mut client, &id, ApprovalDecision::Deny, None).await;
    collect_until(&mut client, entered(SessionState::Review)).await;
    assert!(pending(&mut client, None).await.is_empty());
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(count(&events, EventKind::ApprovalDecided), 1);
}

#[tokio::test]
async fn writes_and_commands_ask_in_ask_mode_and_a_denial_stops_them() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - write: { path: a.txt, content: "x", expect: deny }
  - exec: { argv: ["touch", "ran"], expect: deny }
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::Ask, "go").await;

    let (write, action) = next_request(&mut client).await;
    assert_eq!(
        action,
        Action::Edit {
            path: "a.txt".into(),
            kind: forge_api::model::EditKind::Create,
        }
    );
    decide(&mut client, &write, ApprovalDecision::Deny, None).await;
    let (exec, action) = next_request(&mut client).await;
    assert!(matches!(action, Action::Exec { argv, .. } if argv == ["touch", "ran"]));
    decide(&mut client, &exec, ApprovalDecision::Deny, None).await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let dir = worktree(&repo, &session);
    assert!(!dir.join("a.txt").exists());
    assert!(!dir.join("ran").exists());
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(count(&events, EventKind::Edit), 0);
    // Each refused action ends with a failed result.
    let failed = events
        .iter()
        .filter(|e| matches!(&e.payload, EventPayload::ToolResult(b) if b.status == "failed"))
        .count();
    assert_eq!(failed, 2);
}

#[tokio::test]
async fn hard_denies_and_guards_refuse_without_asking_even_in_full_access() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - write: { path: .env, content: "TOKEN=1", expect: deny }
  - exec: { argv: ["git", "commit", "-m", "x"], expect: deny }
  - write: { path: ok.txt, content: "fine" }
  - exec: ["touch", "ran"]
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::FullAccess, "go").await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let dir = worktree(&repo, &session);
    assert!(!dir.join(".env").exists());
    assert!(dir.join("ok.txt").exists());
    assert!(dir.join("ran").exists());
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(count(&events, EventKind::ApprovalRequested), 0);
    assert_eq!(
        decisions(&events),
        [
            (Verdict::Deny, "guard:protected-path".to_owned()),
            (Verdict::Deny, "builtin:git-shared-state".to_owned()),
            (Verdict::Allow, "mode:full-access".to_owned()),
            (Verdict::Allow, "mode:full-access".to_owned()),
        ]
    );
}

#[tokio::test]
async fn secrets_in_a_command_are_redacted_from_the_log_and_the_inbox() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    let token = format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
    fake_profile(
        &repo,
        "fake",
        &format!(
            r#"
steps:
  - expect_prompt: "*"
  - exec: {{ argv: ["curl", "-H", "Authorization: token {token}", "https://example.com"], expect: deny }}
  - end_turn
"#
        ),
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::FullAccess, "go").await;

    let (id, action) = next_request(&mut client).await;
    let shown = serde_json::to_string(&action).unwrap();
    assert!(!shown.contains(&token), "{shown}");
    let listed = serde_json::to_string(&pending(&mut client, None).await).unwrap();
    assert!(!listed.contains(&token), "{listed}");
    decide(&mut client, &id, ApprovalDecision::Deny, None).await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let events = serde_json::to_string(&all_events(&mut client, &session.id).await).unwrap();
    assert!(!events.contains(&token));
    assert!(events.contains("guard:secret-scan"));
}

#[tokio::test]
async fn a_decision_remembered_for_the_session_is_not_asked_again() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - exec: ["touch", "one"]
  - exec: ["touch", "one"]
  - exec: { argv: ["touch", "two"], expect: deny }
  - exec: { argv: ["touch", "two"], expect: deny }
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::AcceptEdits, "go").await;

    let (one, _) = next_request(&mut client).await;
    decide(
        &mut client,
        &one,
        ApprovalDecision::Allow,
        Some(Remember::Session),
    )
    .await;
    let (two, _) = next_request(&mut client).await;
    decide(
        &mut client,
        &two,
        ApprovalDecision::Deny,
        Some(Remember::Session),
    )
    .await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let events = all_events(&mut client, &session.id).await;
    assert_eq!(count(&events, EventKind::ApprovalRequested), 2);
    assert_eq!(
        decisions(&events),
        [
            (Verdict::Ask, "mode:accept-edits".to_owned()),
            (Verdict::Allow, "session:allow".to_owned()),
            (Verdict::Ask, "mode:accept-edits".to_owned()),
            (Verdict::Deny, "session:deny".to_owned()),
        ]
    );
}

#[tokio::test]
async fn a_decision_remembered_for_the_project_is_written_to_its_policy() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    std::fs::create_dir_all(repo.join(".forge")).unwrap();
    std::fs::write(
        repo.join(".forge/policy.toml"),
        "# Team rules\n[[ask]]\nexec = [\"npm publish*\"]\n",
    )
    .unwrap();
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - exec: ["touch", "a*b"]
  - exec: ["touch", "a*b"]
  - exec: { argv: ["rm", "-rf", "target"], expect: deny }
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::Ask, "go").await;

    let (exec, _) = next_request(&mut client).await;
    decide(
        &mut client,
        &exec,
        ApprovalDecision::Allow,
        Some(Remember::Project),
    )
    .await;
    let (rm, _) = next_request(&mut client).await;
    decide(
        &mut client,
        &rm,
        ApprovalDecision::Deny,
        Some(Remember::Project),
    )
    .await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let text = std::fs::read_to_string(repo.join(".forge/policy.toml")).unwrap();
    assert!(text.starts_with("# Team rules\n[[ask]]\n"), "{text}");
    let policy: forge_config::PolicyConfig = toml::from_str(&text).unwrap();
    assert_eq!(policy.ask.len(), 1);
    assert_eq!(policy.allow.len(), 1);
    assert_eq!(policy.allow[0].exec, [r"touch a\*b"]);
    assert_eq!(policy.deny.len(), 1);
    assert_eq!(policy.deny[0].exec, ["rm -rf target"]);
    // The rule takes effect at once: the second identical command isn't asked about.
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(count(&events, EventKind::ApprovalRequested), 2);
}

#[tokio::test]
async fn what_a_rule_cant_name_exactly_or_safely_is_not_remembered_for_the_project() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    let token = format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
    fake_profile(
        &repo,
        "fake",
        &format!(
            r#"
steps:
  - expect_prompt: "*"
  - write: {{ path: scripts, content: "x", expect: deny }}
  - exec: {{ argv: ["rm", "-rf", "build cache"], expect: deny }}
  - exec: {{ argv: ["echo", "{token}"], expect: deny }}
  - end_turn
"#
        ),
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    start(&mut client, &project, Mode::Ask, "go").await;

    // An edit pattern would cover everything under the path; a word with a space would also
    // match it split in two; a secret must not land in a committed file.
    for _ in 0..3 {
        let (id, _) = next_request(&mut client).await;
        let result = client
            .call::<ApprovalDecide>(&ApprovalDecideParams {
                approval_id: id.clone(),
                decision: ApprovalDecision::Deny,
                remember: Some(Remember::Project),
                note: None,
            })
            .await;
        expect_api_error(result, ErrorKind::ValidationFailed);
        decide(
            &mut client,
            &id,
            ApprovalDecision::Deny,
            Some(Remember::Session),
        )
        .await;
    }
    collect_until(&mut client, entered(SessionState::Review)).await;
    assert!(!repo.join(".forge/policy.toml").exists());
}

#[tokio::test]
async fn an_allowed_permission_request_does_not_skip_the_secret_scan_of_the_write() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    let token = format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8");
    fake_profile(
        &repo,
        "fake",
        &format!(
            r#"
steps:
  - expect_prompt: "*"
  - request_permission: {{ action: edit, path: notes.txt }}
  - expect_permission: allow
  - write: {{ path: notes.txt, content: "token={token}", expect: deny }}
  - request_permission: {{ action: edit, path: plain.txt }}
  - expect_permission: allow
  - write: {{ path: plain.txt, content: "fine" }}
  - end_turn
"#
        ),
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::Ask, "go").await;

    let (permission, _) = next_request(&mut client).await;
    decide(&mut client, &permission, ApprovalDecision::Allow, None).await;
    // The write still asks: the permission request never showed its content.
    let (write, _) = next_request(&mut client).await;
    decide(&mut client, &write, ApprovalDecision::Deny, None).await;
    // Without a secret, the grant covers the write the mode would have asked about.
    let (permission, _) = next_request(&mut client).await;
    decide(&mut client, &permission, ApprovalDecision::Allow, None).await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let dir = worktree(&repo, &session);
    assert!(!dir.join("notes.txt").exists());
    assert_eq!(
        std::fs::read_to_string(dir.join("plain.txt")).unwrap(),
        "fine"
    );
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(count(&events, EventKind::ApprovalRequested), 3);
    assert!(decisions(&events).contains(&(Verdict::Allow, "approval".to_owned())));
}

#[tokio::test]
async fn remembering_what_a_rule_would_still_ask_about_is_refused() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - exec: { argv: ["sh", "-c", "$CMD"], expect: deny }
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::FullAccess, "go").await;

    // The opaque-command guard asks; no remembered allow could get past it.
    let (id, _) = next_request(&mut client).await;
    for remember in [Remember::Session, Remember::Project] {
        let result = client
            .call::<ApprovalDecide>(&ApprovalDecideParams {
                approval_id: id.clone(),
                decision: ApprovalDecision::Allow,
                remember: Some(remember),
                note: None,
            })
            .await;
        expect_api_error(result, ErrorKind::ValidationFailed);
    }
    assert_eq!(pending(&mut client, None).await.len(), 1);
    decide(&mut client, &id, ApprovalDecision::Deny, None).await;
    collect_until(&mut client, entered(SessionState::Review)).await;
    assert!(!repo.join(".forge/policy.toml").exists());
    let _ = session;
}

#[tokio::test]
async fn cancelling_a_waiting_session_refuses_its_pending_approvals() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", ASK_THEN_WRITE);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::AcceptEdits, "go").await;

    let (id, _) = next_request(&mut client).await;
    collect_until(&mut client, entered(SessionState::Waiting)).await;
    client
        .call::<SessionCancel>(&SessionIdParams {
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    collect_until(&mut client, entered(SessionState::Ready)).await;
    assert!(pending(&mut client, None).await.is_empty());
    assert!(!worktree(&repo, &session).join("after.txt").exists());
    let late = client
        .call::<ApprovalDecide>(&ApprovalDecideParams {
            approval_id: id,
            decision: ApprovalDecision::Allow,
            remember: None,
            note: None,
        })
        .await;
    expect_api_error(late, ErrorKind::InvalidState);
}

#[tokio::test]
async fn set_mode_changes_later_decisions_but_not_the_sandbox() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - write: { path: a.txt, content: "x" }
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::Ask, "").await;
    wait_for_state(&mut client, &session.id, SessionState::Ready).await;

    let sandbox = client
        .call::<SessionSetMode>(&SessionSetModeParams {
            session_id: session.id.clone(),
            mode: Mode::AcceptEdits,
            sandbox: Some(SandboxTier::Standard),
        })
        .await;
    expect_api_error(sandbox, ErrorKind::ValidationFailed);
    let updated = client
        .call::<SessionSetMode>(&SessionSetModeParams {
            session_id: session.id.clone(),
            mode: Mode::AcceptEdits,
            sandbox: Some(SandboxTier::Unsandboxed),
        })
        .await
        .unwrap();
    assert_eq!(updated.mode, Mode::AcceptEdits);
    assert_eq!(updated.sandbox, SandboxTier::Unsandboxed);
    assert_eq!(
        client
            .call::<SessionGet>(&SessionIdParams {
                session_id: session.id.clone(),
            })
            .await
            .unwrap()
            .mode,
        Mode::AcceptEdits
    );

    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "go".into(),
        })
        .await
        .unwrap();
    collect_until(&mut client, entered(SessionState::Review)).await;
    assert!(worktree(&repo, &session).join("a.txt").exists());
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(count(&events, EventKind::ApprovalRequested), 0);
}

#[tokio::test]
async fn a_daemon_restart_cancels_pending_approvals_and_tells_the_agent() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", ASK_THEN_WRITE);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, Mode::AcceptEdits, "go").await;
    next_request(&mut client).await;
    collect_until(&mut client, entered(SessionState::Waiting)).await;
    drop(client);
    daemon.kill();

    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    assert_eq!(state(&mut client, &session).await, SessionState::Paused);
    assert!(pending(&mut client, None).await.is_empty());

    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "carry on".into(),
        })
        .await
        .unwrap();
    subscribe(
        &mut client,
        vec![Topic::Session(session.id.clone())],
        Some(0),
    )
    .await;
    // The new agent process starts its script over and asks again: a second wait.
    let mut waits = 0;
    collect_until(&mut client, |e| {
        waits += usize::from(entered(SessionState::Waiting)(e));
        waits == 2
    })
    .await;
    let prompts: Vec<String> = all_events(&mut client, &session.id)
        .await
        .into_iter()
        .filter_map(|e| match e.payload {
            EventPayload::Prompt(b) => Some(b.text),
            _ => None,
        })
        .collect();
    assert_eq!(prompts.len(), 2);
    let resumed = &prompts[1];
    assert!(resumed.ends_with("carry on"), "{resumed}");
    assert!(resumed.contains("pnpm add left-pad"), "{resumed}");
    assert!(resumed.contains("cancelled"), "{resumed}");
}

#[tokio::test]
async fn reads_inside_the_worktree_are_allowed_and_other_unmapped_requests_refused() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - request_permission: { action: read, path: README.md }
  - expect_permission: allow
  - request_permission: { action: read, path: ../../../../outside.txt }
  - expect_permission: deny
  - request_permission: { action: read }
  - expect_permission: deny
  - request_permission: { action: fetch }
  - expect_permission: deny
  - end_turn
"#,
    );
    std::fs::write(home.path().join("outside.txt"), "x").unwrap();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    // Even in Ask mode, nothing here waits on a human.
    let session = start(&mut client, &project, Mode::Ask, "go").await;
    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(
        decisions(&events),
        [
            (Verdict::Allow, "read:worktree".into()),
            (Verdict::Deny, "guard:outside-worktree".into()),
            (Verdict::Deny, "guard:outside-worktree".into()),
            (Verdict::Deny, "network:allowlist".into()),
        ]
    );
    assert_eq!(count(&events, EventKind::ApprovalRequested), 0);
    daemon.stop();
}
