//! Secret redaction against a real daemon driving the fake agent: secrets in prompts, terminal
//! output, tool results, and agent messages never reach the event log, its search index, or
//! subscribers, even when a secret arrives split across streamed chunks.

mod common;

use std::path::Path;

use common::{
    Daemon, all_events, collect_until, create_params, entered, fake_profile, git_repo_with_commit,
    open, subscribe, temp_home, wait_for_state,
};
use forge_api::events::{EventPayload, Topic};
use forge_api::methods::*;
use forge_api::model::SessionState;

const GITHUB: &str = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";
const ANTHROPIC: &str = "sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123";

#[tokio::test]
async fn secrets_never_reach_the_log_even_split_across_chunks() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    let (gh_head, gh_tail) = GITHUB.split_at(14);
    let (an_head, an_tail) = ANTHROPIC.split_at(16);
    // The command line shows neither part whole, so only its output can carry the token.
    let (gh_a, gh_b) = gh_head.split_at(7);
    fake_profile(
        &repo,
        "fake",
        &format!(
            r#"
steps:
  - expect_prompt: "*"
  - exec: ["sh", "-c", "printf 'token: {gh_a}''{gh_b}'; sleep 0.3; printf '{gh_tail}\n'"]
  - message: "Use {an_head}"
  - message: "{an_tail} for the API."
  - end_turn
"#
        ),
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(
            &project,
            "feat: deploy",
            &format!("deploy with {GITHUB}"),
        ))
        .await
        .unwrap();
    subscribe(
        &mut client,
        vec![Topic::Session(session.id.clone())],
        Some(0),
    )
    .await;
    let live = collect_until(&mut client, entered(SessionState::Review)).await;

    let leaks = |text: &str| {
        [GITHUB, gh_head, ANTHROPIC, an_head]
            .into_iter()
            .filter(|s| text.contains(s))
            .collect::<Vec<_>>()
    };
    // What subscribers saw, what a query returns, and what is on disk.
    let streamed = serde_json::to_string(&live).unwrap();
    assert!(leaks(&streamed).is_empty(), "{:?}", leaks(&streamed));
    let events = all_events(&mut client, &session.id).await;
    let queried = serde_json::to_string(&events).unwrap();
    assert!(leaks(&queried).is_empty(), "{:?}", leaks(&queried));
    let file = repo
        .join(".forge/sessions")
        .join(session.id.as_str())
        .join("events.jsonl");
    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert!(leaks(&on_disk).is_empty(), "{:?}", leaks(&on_disk));

    // Each place a secret was shows its kind instead, and the streams stayed whole.
    let output: String = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TerminalOutput(b) => Some(b.chunk.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(output, "token: [redacted:github-token]\n");
    let message: String = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::AgentMessage(b) => Some(b.delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(message, "Use [redacted:anthropic-key] for the API.");
    let prompt = events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::Prompt(b) => Some(b.text.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(prompt, "deploy with [redacted:github-token]");
    let result = events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResult(b) => b.output.clone(),
            _ => None,
        })
        .unwrap();
    assert!(result.contains("[redacted:github-token]"), "{result}");

    // The search index has the redacted text only.
    for text in [gh_head, "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8"] {
        let found = client
            .call::<EventsQuery>(&EventsQueryParams {
                session_id: session.id.clone(),
                kinds: None,
                text: Some(text.into()),
                before: None,
                limit: 100,
            })
            .await
            .unwrap();
        assert!(found.is_empty(), "the index found {text:?}");
    }
}

/// The value the fake keyring holds for `ANTHROPIC_API_KEY`; it matches no built-in pattern.
const INJECTED: &str = "hunter2-forge-injected-value";

/// Start a daemon whose keyring tool knows only `ANTHROPIC_API_KEY`, and fails while a
/// `locked` file sits next to it.
fn daemon_with_keyring(home: &Path) -> Daemon {
    use std::os::unix::fs::PermissionsExt;
    let tool = home.join("fake-keyring");
    let missing = if cfg!(target_os = "macos") { 44 } else { 1 };
    let locked = home.join("locked");
    let script = format!(
        "#!/bin/sh\n[ -e {locked:?} ] && exit 2\ncase \"$*\" in\n  *ANTHROPIC_API_KEY*) echo {INJECTED} ;;\n  *) exit {missing} ;;\nesac\n"
    );
    std::fs::write(&tool, script).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    Daemon::start_with(home, |cmd| {
        cmd.env("FORGE_KEYRING_TOOL", &tool);
    })
}

#[tokio::test]
async fn after_a_restart_the_first_prompt_is_redacted_with_the_sessions_secrets() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", "steps:\n  - end_turn\n  - end_turn\n");
    let profile = repo.join(".forge/agents/fake.toml");
    let text = std::fs::read_to_string(&profile).unwrap();
    std::fs::write(
        &profile,
        format!("{text}env_secrets = [\"ANTHROPIC_API_KEY\"]\n"),
    )
    .unwrap();
    let daemon = daemon_with_keyring(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: restart", ""))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Ready).await;
    drop(client);
    daemon.kill();

    let daemon = daemon_with_keyring(home.path());
    let mut client = daemon.client().await;
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: format!("the key is {INJECTED}, use it"),
        })
        .await
        .unwrap();
    let events = all_events(&mut client, &session.id).await;
    let prompt = events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::Prompt(b) => Some(b.text.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(prompt, "the key is [redacted:injected-secret], use it");
    let file = repo
        .join(".forge/sessions")
        .join(session.id.as_str())
        .join("events.jsonl");
    assert!(!std::fs::read_to_string(file).unwrap().contains(INJECTED));
}

#[tokio::test]
async fn a_private_key_printed_in_pieces_leaves_no_trace_in_the_log() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    // A key in the worktree, printed in two writes.
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - exec: ["sh", "-c", "head -c 60 key.txt; sleep 0.3; tail -c +61 key.txt; echo after"]
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: key", ""))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Ready).await;
    let worktree = repo.join(".worktrees").join(&session.branch);
    std::fs::write(
        worktree.join("key.txt"),
        "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nQyNTUxOQAAACBrZXlib2R5\n-----END OPENSSH PRIVATE KEY-----\n",
    )
    .unwrap();
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "go".into(),
        })
        .await
        .unwrap();
    collect_until(&mut client, entered(SessionState::Review)).await;
    let events = all_events(&mut client, &session.id).await;
    let output: String = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TerminalOutput(b) => Some(b.chunk.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        output,
        "[redacted:private-key; output withheld until END]\nafter\n"
    );
    let file = repo
        .join(".forge/sessions")
        .join(session.id.as_str())
        .join("events.jsonl");
    let on_disk = std::fs::read_to_string(file).unwrap();
    for piece in ["b3BlbnNz", "QyNTUx", "END OPENSSH", "BEGIN OPENSSH"] {
        assert!(!on_disk.contains(piece), "{piece} reached the log");
    }
}

#[tokio::test]
async fn a_prompt_fails_when_the_sessions_secrets_cant_be_loaded() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", "steps:\n  - end_turn\n");
    let profile = repo.join(".forge/agents/fake.toml");
    let text = std::fs::read_to_string(&profile).unwrap();
    std::fs::write(
        &profile,
        format!("{text}env_secrets = [\"ANTHROPIC_API_KEY\"]\n"),
    )
    .unwrap();
    let daemon = daemon_with_keyring(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: locked", ""))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Ready).await;
    drop(client);
    daemon.kill();

    // After the restart the keyring is locked: the prompt is refused, not recorded.
    std::fs::write(home.path().join("locked"), "").unwrap();
    let daemon = daemon_with_keyring(home.path());
    let mut client = daemon.client().await;
    let refused = client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: format!("the key is {INJECTED}"),
        })
        .await;
    assert!(refused.is_err(), "{refused:?}");
    let events = all_events(&mut client, &session.id).await;
    assert!(
        events
            .iter()
            .all(|e| !matches!(e.payload, EventPayload::Prompt(_)))
    );
    let state = client
        .call::<SessionGet>(&SessionIdParams {
            session_id: session.id.clone(),
        })
        .await
        .unwrap()
        .state;
    assert_eq!(state, SessionState::Ready);
}

#[tokio::test]
async fn stderr_between_parts_of_a_token_doesnt_split_it_in_the_log() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    let (head, tail) = GITHUB.split_at(14);
    let (a, b) = head.split_at(7);
    fake_profile(
        &repo,
        "fake",
        &format!(
            r#"
steps:
  - exec: ["sh", "-c", "printf 'token: {a}''{b}'; echo compiling >&2; sleep 0.3; printf '{tail}\n'"]
  - end_turn
"#
        ),
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: pipes", "go"))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    let file = repo
        .join(".forge/sessions")
        .join(session.id.as_str())
        .join("events.jsonl");
    let on_disk = std::fs::read_to_string(file).unwrap();
    assert!(!on_disk.contains(head), "{on_disk}");
    let events = all_events(&mut client, &session.id).await;
    let result = events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResult(b) => b.output.clone(),
            _ => None,
        })
        .unwrap();
    assert!(
        result.contains("token: [redacted:github-token]"),
        "{result}"
    );
    assert!(result.contains("compiling"), "{result}");
}

#[tokio::test]
async fn two_pipes_ending_mid_line_dont_hide_a_key_in_the_result() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - exec: ["sh", "-c", "printf 'AKIA''0123456789ABCDEF'; printf warn >&2"]
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: seam", "go"))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    let events = all_events(&mut client, &session.id).await;
    let result = events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::ToolResult(b) => b.output.clone(),
            _ => None,
        })
        .unwrap();
    assert!(result.contains("[redacted:aws-access-key]"), "{result}");
    assert!(result.contains("warn"), "{result}");
    // The command line shows the key only in quoted pieces; no output carries it whole.
    let logged = serde_json::to_string(&events).unwrap();
    assert!(!logged.contains("AKIA0123456789ABCDEF"), "{logged}");
}
