//! Session tests against a real daemon driving the fake agent: a full turn, lossless reconnect, every
//! session method, branch collisions, and recovery after a daemon restart.

mod common;

use common::{
    Daemon, all_events, collect_until, create_params, entered, expect_api_error, fake_profile, git,
    git_repo_with_commit, hello_tiers, open, subscribe, temp_home, transitions, wait_for_state,
};
use forge_api::client::Client;
use forge_api::events::{Event, EventKind, EventNotification, EventPayload, Topic};
use forge_api::methods::*;
use forge_api::model::{Action, Project, SandboxTier, Session, SessionState};
use forge_api::{ErrorKind, SessionId};

const EDIT_AND_RUN: &str = r#"
steps:
  - expect_prompt: "*greeting*"
  - plan: ["Write files", "Run a command"]
  - read: README.md
  - write: { path: hello.txt, content: "hello\n" }
  - write: { path: src/lib.txt, content: "one\ntwo\n" }
  - exec: ["sh", "-c", "cat hello.txt"]
  - message: "Done."
  - end_turn
"#;

/// Waits on the first prompt for a minute: long enough for tests to act mid-turn.
const SLOW: &str = r#"
steps:
  - expect_prompt: "*"
  - sleep_ms: 60000
  - end_turn
"#;

#[tokio::test]
async fn fake_agent_edits_two_files_and_runs_a_command_in_a_worktree() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", EDIT_AND_RUN);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;

    let session = client
        .call::<SessionCreate>(&create_params(
            &project,
            "feat: add greeting",
            "add a greeting",
        ))
        .await
        .unwrap();
    assert_eq!(session.branch, "feat/add-greeting");
    assert_eq!(session.state, SessionState::Provisioning);
    assert_eq!(session.sandbox, SandboxTier::Unsandboxed);

    subscribe(
        &mut client,
        vec![Topic::Session(session.id.clone())],
        Some(0),
    )
    .await;
    let notes = collect_until(&mut client, entered(SessionState::Review)).await;
    let events: Vec<Event> = notes.iter().map(|n| n.event.clone()).collect();

    // Notifications carry a strictly increasing stream cursor; events a gap-free session seq.
    assert!(notes.windows(2).all(|w| w[0].seq < w[1].seq));
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=events.len() as u64).collect::<Vec<_>>());

    assert_eq!(
        transitions(&events),
        vec![
            (SessionState::Provisioning, SessionState::Ready),
            (SessionState::Ready, SessionState::Working),
            (SessionState::Working, SessionState::Review),
        ]
    );

    // Every step of the turn is in the log, in order.
    let kinds: Vec<EventKind> = events.iter().map(|e| e.payload.kind()).collect();
    let expected = [
        EventKind::TurnStarted,
        EventKind::Prompt,
        EventKind::AgentPlan,
        EventKind::ToolCall,
        EventKind::Edit,
        EventKind::ToolResult,
        EventKind::ToolCall,
        EventKind::Edit,
        EventKind::ToolResult,
        EventKind::ToolCall,
        EventKind::TerminalOutput,
        EventKind::ToolResult,
        EventKind::AgentMessage,
        EventKind::TurnEnded,
    ];
    let mut rest = kinds.iter();
    for want in expected {
        assert!(
            rest.any(|k| *k == want),
            "missing {want:?} in order; got {kinds:?}"
        );
    }
    let edited: Vec<&str> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Edit(b) => Some(b.path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(edited, ["hello.txt", "src/lib.txt"]);
    let exec = events.iter().find_map(|e| match &e.payload {
        EventPayload::ToolCall(b) => match &b.action {
            Action::Exec { argv, cwd } => Some((argv.clone(), cwd.clone())),
            _ => None,
        },
        _ => None,
    });
    assert_eq!(
        exec,
        Some((
            vec!["sh".into(), "-c".into(), "cat hello.txt".into()],
            ".".into()
        ))
    );
    let output: String = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TerminalOutput(b) => Some(b.chunk.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(output, "hello\n");

    // The work happened in the session's worktree on its branch; the main checkout is untouched.
    let worktree = repo.join(".worktrees/feat/add-greeting");
    assert_eq!(
        std::fs::read_to_string(worktree.join("hello.txt")).unwrap(),
        "hello\n"
    );
    assert_eq!(
        std::fs::read_to_string(worktree.join("src/lib.txt")).unwrap(),
        "one\ntwo\n"
    );
    assert!(!repo.join("hello.txt").exists());
    assert_eq!(
        git(&worktree, &["branch", "--show-current"]).trim(),
        "feat/add-greeting"
    );
    assert!(!git(&repo, &["status", "--porcelain"]).contains(".worktrees"));

    // The query API and the JSONL log hold exactly what subscribers saw.
    assert_eq!(all_events(&mut client, &session.id).await, events);
    let log = repo.join(format!(".forge/sessions/{}/events.jsonl", session.id));
    let logged: Vec<Event> = std::fs::read_to_string(&log)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(logged, events);
    let query = client
        .call::<EventsQuery>(&EventsQueryParams {
            session_id: session.id.clone(),
            kinds: Some(vec![EventKind::Edit]),
            text: None,
            before: None,
            limit: 10,
        })
        .await
        .unwrap();
    assert_eq!(query.len(), 2);

    let got = client
        .call::<SessionGet>(&SessionIdParams {
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(got.state, SessionState::Review);
    let listed = client
        .call::<SessionList>(&SessionListParams {
            project_id: project.id.clone(),
            states: Some(vec![SessionState::Review]),
        })
        .await
        .unwrap();
    assert_eq!(
        listed.iter().map(|s| &s.id).collect::<Vec<_>>(),
        [&session.id]
    );

    client
        .call::<SessionClose>(&SessionCloseParams {
            session_id: session.id.clone(),
            outcome: SessionOutcome::Abandoned,
        })
        .await
        .unwrap();
    let closed = client
        .call::<SessionGet>(&SessionIdParams {
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(closed.state, SessionState::Done);
    assert!(worktree.exists(), "close keeps the worktree");
}

#[tokio::test]
async fn a_client_killed_mid_stream_resumes_with_after_seq_and_loses_nothing() {
    let mut script = String::from("steps:\n  - expect_prompt: \"*\"\n");
    for i in 0..20 {
        script.push_str(&format!("  - message: \"chunk {i} \"\n  - sleep_ms: 25\n"));
    }
    script.push_str("  - end_turn\n");

    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", &script);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "stream", "go"))
        .await
        .unwrap();
    let topics = vec![Topic::Board, Topic::Session(session.id.clone())];

    // The first client reads part of the stream, then dies without closing the connection.
    let mut first = daemon.client().await;
    subscribe(&mut first, topics.clone(), Some(0)).await;
    let mut before = Vec::new();
    while before.len() < 12 {
        before.push(first.next_event().await.unwrap());
    }
    drop(first);
    let last = before.last().unwrap().seq;

    let mut second = daemon.client().await;
    subscribe(&mut second, topics, Some(last)).await;
    let after = collect_until(&mut second, entered(SessionState::Review)).await;

    let all: Vec<EventNotification> = before.into_iter().chain(after).collect();
    assert!(
        all.windows(2).all(|w| w[0].seq < w[1].seq),
        "no duplicates or reordering across the reconnect"
    );
    let received: Vec<Event> = all.into_iter().map(|n| n.event).collect();
    let logged = all_events(&mut client, &session.id).await;
    assert_eq!(received, logged, "no events lost across the reconnect");
    let messages = logged
        .iter()
        .filter(|e| e.payload.kind() == EventKind::AgentMessage)
        .count();
    assert_eq!(messages, 20);
}

#[tokio::test]
async fn subscribing_past_the_end_of_the_stream_is_rejected() {
    let home = temp_home();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let ahead = client
        .call::<EventsSubscribe>(&EventsSubscribeParams {
            topics: vec![Topic::Board],
            after_seq: Some(1_000),
        })
        .await;
    expect_api_error(ahead, ErrorKind::ValidationFailed);
}

#[tokio::test]
async fn sessions_need_an_available_sandbox_tier() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", EDIT_AND_RUN);
    // A profile that has not opted in to running unsandboxed.
    std::fs::write(
        repo.join(".forge/agents/strict.toml"),
        format!(
            "agent = \"fake\"\ncommand = [{:?}]\n",
            common::fake_agent().to_str().unwrap()
        ),
    )
    .unwrap();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;

    // Tiers this machine can't run are refused with the reason and a fix, never swapped for
    // another. The tiers it can run are covered by the sandbox tests.
    let available = hello_tiers(&mut client).await;
    for tier in [
        SandboxTier::Standard,
        SandboxTier::Light,
        SandboxTier::Container,
    ] {
        if available.contains(&tier) {
            continue;
        }
        let mut params = create_params(&project, "x", "go");
        params.sandbox = Some(tier);
        params.agent_profile = "strict".into();
        let err = expect_api_error(
            client.call::<SessionCreate>(&params).await,
            ErrorKind::SandboxUnavailable,
        );
        assert!(err.detail().contains("not available"), "{}", err.detail());
    }
    let mut params = create_params(&project, "x", "go");
    params.agent_profile = "strict".into();
    let err = expect_api_error(
        client.call::<SessionCreate>(&params).await,
        ErrorKind::SandboxUnavailable,
    );
    assert!(
        err.detail().contains("allow_unsandboxed"),
        "{}",
        err.detail()
    );

    let mut params = create_params(&project, "x", "go");
    params.agent_profile = "missing".into();
    expect_api_error(
        client.call::<SessionCreate>(&params).await,
        ErrorKind::NotFound,
    );
    let mut params = create_params(&project, "x", "go");
    params.agent_profile = "../escape".into();
    expect_api_error(
        client.call::<SessionCreate>(&params).await,
        ErrorKind::ValidationFailed,
    );

    let sessions = client
        .call::<SessionList>(&SessionListParams {
            project_id: project.id,
            states: None,
        })
        .await
        .unwrap();
    assert!(sessions.is_empty(), "rejected creates leave nothing behind");
    assert!(!repo.join(".worktrees/feat/x").exists());
}

#[tokio::test]
async fn branch_names_follow_the_configured_format() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", SLOW);
    std::fs::write(
        home.path().join("config/config.toml"),
        "[branches]\nformat = \"jeff/{type}/{slug}\"\ntypes = [\"feat\", \"fix\", \"chore\"]\n",
    )
    .unwrap();
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;

    for (title, branch) in [
        (
            "fix: Retry loop never ends!",
            "jeff/fix/retry-loop-never-ends",
        ),
        ("chore(deps): bump tokio", "jeff/chore/bump-tokio"),
        ("Add a greeting", "jeff/feat/add-a-greeting"),
        (
            "bug: not a configured type",
            "jeff/feat/bug-not-a-configured-type",
        ),
    ] {
        let preview = client
            .call::<SessionBranchPreview>(&BranchPreviewParams {
                project_id: project.id.clone(),
                title: title.into(),
                branch: None,
            })
            .await
            .unwrap();
        assert_eq!(preview.branch, branch, "{title}");
        assert!(preview.collisions.is_empty());
    }

    let explicit = client
        .call::<SessionBranchPreview>(&BranchPreviewParams {
            project_id: project.id.clone(),
            title: "anything".into(),
            branch: Some("topic/my-own".into()),
        })
        .await
        .unwrap();
    assert_eq!(explicit.branch, "topic/my-own");

    for bad in ["has space", "double..dot", "-leading"] {
        let result = client
            .call::<SessionBranchPreview>(&BranchPreviewParams {
                project_id: project.id.clone(),
                title: "x".into(),
                branch: Some(bad.into()),
            })
            .await;
        expect_api_error(result, ErrorKind::ValidationFailed);
    }
    let no_slug = client
        .call::<SessionBranchPreview>(&BranchPreviewParams {
            project_id: project.id.clone(),
            title: "!!!".into(),
            branch: None,
        })
        .await;
    expect_api_error(no_slug, ErrorKind::ValidationFailed);
}

async fn preview(client: &mut Client, project: &Project, title: &str) -> BranchPreview {
    client
        .call::<SessionBranchPreview>(&BranchPreviewParams {
            project_id: project.id.clone(),
            title: title.into(),
            branch: None,
        })
        .await
        .unwrap()
}

fn collision_kinds(preview: &BranchPreview) -> Vec<CollisionKind> {
    preview.collisions.iter().map(|c| c.kind).collect()
}

#[tokio::test]
async fn name_collisions_ask_the_user_and_honor_their_choice() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", SLOW);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;

    // A live session owns its worktree: nobody else may take it, whatever they choose.
    let live = client
        .call::<SessionCreate>(&create_params(&project, "feat: shared", ""))
        .await
        .unwrap();
    wait_for_state(&mut client, &live.id, SessionState::Ready).await;
    let p = preview(&mut client, &project, "feat: shared").await;
    assert!(collision_kinds(&p).contains(&CollisionKind::OwnedByLiveSession));
    assert_eq!(p.collisions[0].session_id.as_ref(), Some(&live.id));
    for choice in [
        None,
        Some(CollisionResolution::ReuseBranch),
        Some(CollisionResolution::AdoptWorktree),
    ] {
        let mut params = create_params(&project, "feat: shared", "");
        params.on_collision = choice;
        expect_api_error(
            client.call::<SessionCreate>(&params).await,
            ErrorKind::Conflict,
        );
    }

    // Once that session is done, its worktree can be adopted.
    client
        .call::<SessionClose>(&SessionCloseParams {
            session_id: live.id.clone(),
            outcome: SessionOutcome::Abandoned,
        })
        .await
        .unwrap();
    let p = preview(&mut client, &project, "feat: shared").await;
    assert_eq!(
        collision_kinds(&p),
        [CollisionKind::BranchExists, CollisionKind::WorktreeExists]
    );
    expect_api_error(
        client
            .call::<SessionCreate>(&create_params(&project, "feat: shared", ""))
            .await,
        ErrorKind::Conflict,
    );
    let mut reuse = create_params(&project, "feat: shared", "");
    reuse.on_collision = Some(CollisionResolution::ReuseBranch);
    expect_api_error(
        client.call::<SessionCreate>(&reuse).await,
        ErrorKind::Conflict,
    );
    let mut adopt = create_params(&project, "feat: shared", "");
    adopt.on_collision = Some(CollisionResolution::AdoptWorktree);
    let adopted = client.call::<SessionCreate>(&adopt).await.unwrap();
    assert_eq!(adopted.branch, "feat/shared");
    wait_for_state(&mut client, &adopted.id, SessionState::Ready).await;

    // A branch with no worktree can be reused, and the worktree starts from its commits.
    let worktree_dir = repo.join(".worktrees/feat/existing");
    git(&repo, &["branch", "feat/existing"]);
    git(
        &repo,
        &["commit", "--quiet", "--allow-empty", "-m", "on main only"],
    );
    let p = preview(&mut client, &project, "existing").await;
    assert_eq!(collision_kinds(&p), [CollisionKind::BranchExists]);
    expect_api_error(
        client
            .call::<SessionCreate>(&create_params(&project, "existing", ""))
            .await,
        ErrorKind::Conflict,
    );
    let mut adopt = create_params(&project, "existing", "");
    adopt.on_collision = Some(CollisionResolution::AdoptWorktree);
    expect_api_error(
        client.call::<SessionCreate>(&adopt).await,
        ErrorKind::Conflict,
    );
    let mut reuse = create_params(&project, "existing", "");
    reuse.on_collision = Some(CollisionResolution::ReuseBranch);
    let reused = client.call::<SessionCreate>(&reuse).await.unwrap();
    wait_for_state(&mut client, &reused.id, SessionState::Ready).await;
    assert_eq!(
        git(&worktree_dir, &["log", "-1", "--format=%s"]).trim(),
        "initial"
    );

    // A branch checked out in the main checkout can't be reused.
    let mut main = create_params(&project, "x", "");
    main.branch = Some("main".into());
    main.on_collision = Some(CollisionResolution::ReuseBranch);
    expect_api_error(
        client.call::<SessionCreate>(&main).await,
        ErrorKind::Conflict,
    );
}

#[tokio::test]
async fn a_repo_path_that_escapes_the_project_through_a_symlink_is_refused() {
    let home = temp_home();
    let outside = git_repo_with_commit(home.path(), "outside");
    let root = home.path().join("project");
    std::fs::create_dir(&root).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("inner")).unwrap();
    std::fs::write(
        root.join("forge.toml"),
        "[project]\nname = \"p\"\n[[repo]]\nname = \"inner\"\npath = \"inner\"\nbase_branch = \"main\"\n",
    )
    .unwrap();
    fake_profile(&root, "fake", SLOW);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &root.canonicalize().unwrap()).await;

    let err = expect_api_error(
        client
            .call::<SessionCreate>(&create_params(&project, "escape", ""))
            .await,
        ErrorKind::ValidationFailed,
    );
    assert!(
        err.detail().contains("outside the project"),
        "{}",
        err.detail()
    );
    assert!(!outside.join(".worktrees").exists());
}

#[tokio::test]
async fn methods_check_the_session_state() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", SLOW);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;

    let unknown = SessionIdParams {
        session_id: SessionId::new(),
    };
    expect_api_error(
        client.call::<SessionGet>(&unknown).await,
        ErrorKind::NotFound,
    );
    expect_api_error(
        client.call::<SessionCancel>(&unknown).await,
        ErrorKind::NotFound,
    );

    let session = client
        .call::<SessionCreate>(&create_params(&project, "slow", ""))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Ready).await;
    let id = SessionIdParams {
        session_id: session.id.clone(),
    };
    expect_api_error(
        client.call::<SessionCancel>(&id).await,
        ErrorKind::InvalidState,
    );

    let turn = client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "take your time".into(),
        })
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Working).await;
    let busy = client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "again".into(),
        })
        .await;
    expect_api_error(busy, ErrorKind::InvalidState);
    let close_working = client
        .call::<SessionClose>(&SessionCloseParams {
            session_id: session.id.clone(),
            outcome: SessionOutcome::Abandoned,
        })
        .await;
    expect_api_error(close_working, ErrorKind::InvalidState);

    // Cancel ends the turn and returns the session to Ready.
    client.call::<SessionCancel>(&id).await.unwrap();
    let mut watcher = daemon.client().await;
    subscribe(
        &mut watcher,
        vec![Topic::Session(session.id.clone())],
        Some(0),
    )
    .await;
    let notes = collect_until(&mut watcher, |e| {
        matches!(&e.payload, EventPayload::SessionState(b)
            if b.from == SessionState::Working && b.to == SessionState::Ready)
    })
    .await;
    let ended = notes.iter().find_map(|n| match &n.event.payload {
        EventPayload::TurnEnded(b) => Some(b.clone()),
        _ => None,
    });
    let ended = ended.expect("turn.ended before Ready");
    assert_eq!(ended.turn_id, turn.turn_id);
    assert_eq!(ended.stop_reason, "cancelled");

    client
        .call::<SessionClose>(&SessionCloseParams {
            session_id: session.id.clone(),
            outcome: SessionOutcome::Abandoned,
        })
        .await
        .unwrap();
    for result in [
        client
            .call::<SessionPrompt>(&SessionPromptParams {
                session_id: session.id.clone(),
                text: "after close".into(),
            })
            .await
            .map(|_| ()),
        client
            .call::<SessionClose>(&SessionCloseParams {
                session_id: session.id.clone(),
                outcome: SessionOutcome::Abandoned,
            })
            .await
            .map(|_| ()),
    ] {
        expect_api_error(result, ErrorKind::InvalidState);
    }
}

#[tokio::test]
async fn a_daemon_restart_pauses_working_sessions() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", SLOW);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "slow", "go"))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Working).await;
    drop(client);
    daemon.kill();

    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let got: Session = client
        .call::<SessionGet>(&SessionIdParams {
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(got.state, SessionState::Paused);
    let events = all_events(&mut client, &session.id).await;
    let last = events.last().unwrap();
    match &last.payload {
        EventPayload::SessionState(b) => {
            assert_eq!(
                (b.from, b.to),
                (SessionState::Working, SessionState::Paused)
            );
            assert_eq!(b.reason.as_deref(), Some("Interrupted by daemon restart"));
        }
        other => panic!("expected a state change, got {other:?}"),
    }
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=events.len() as u64).collect::<Vec<_>>());

    // A paused session takes a new prompt, relaunching its agent.
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "carry on".into(),
        })
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Working).await;
}

#[tokio::test]
async fn each_launch_starts_the_agents_own_config_empty() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", "steps:\n  - end_turn\n  - end_turn\n");
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: homes", "first"))
        .await
        .unwrap();
    subscribe(
        &mut client,
        vec![Topic::Session(session.id.clone())],
        Some(0),
    )
    .await;
    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    daemon.stop();

    // What an earlier command could have left in the sandbox home.
    let scratch_home = home
        .path()
        .join("data/sessions")
        .join(session.id.as_str())
        .join("home");
    std::fs::create_dir_all(scratch_home.join(".codex/rules")).unwrap();
    std::fs::write(
        scratch_home.join(".codex/config.toml"),
        "notify = [\"sh\"]\n",
    )
    .unwrap();
    let outside = home.path().join("outside.json");
    std::fs::write(&outside, "{}").unwrap();
    std::os::unix::fs::symlink(&outside, scratch_home.join(".claude.json")).unwrap();
    std::fs::write(scratch_home.join("notes.txt"), "kept").unwrap();

    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "again".into(),
        })
        .await
        .unwrap();
    // The second turn's Review, after the relaunch.
    subscribe(
        &mut client,
        vec![Topic::Session(session.id.clone())],
        Some(0),
    )
    .await;
    let mut reviews = 0;
    let mut review = entered(SessionState::Review);
    collect_until(&mut client, |e| {
        reviews += usize::from(review(e));
        reviews == 2
    })
    .await;
    assert!(!scratch_home.join(".codex").exists());
    assert!(std::fs::symlink_metadata(scratch_home.join(".claude.json")).is_err());
    assert_eq!(
        std::fs::read_to_string(&outside).unwrap(),
        "{}",
        "the link's target is untouched"
    );
    assert_eq!(
        std::fs::read_to_string(scratch_home.join("notes.txt")).unwrap(),
        "kept"
    );
    daemon.stop();
}
