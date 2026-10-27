//! Checkpoints after each turn, and `session.rewind` back to one, with the fake agent.

mod common;

use std::path::{Path, PathBuf};

use common::{
    Daemon, all_events, collect_until, create_params, expect_api_error, fake_profile, git,
    git_repo_with_commit, open, subscribe, temp_home,
};
use forge_api::client::Client;
use forge_api::events::{Event, EventPayload, Topic};
use forge_api::methods::*;
use forge_api::model::{Session, SessionState};
use forge_api::{CheckpointId, ErrorKind, SessionId, TurnId};

const SCRIPT: &str = r#"
steps:
  - write: { path: a.txt, content: "turn one\n" }
  - end_turn
  - expect_prompt: "second"
  - write: { path: b.txt, content: "turn two\n" }
  - exec: ["sh", "-c", "echo build > out.log"]
  - exec: ["sh", "-c", "echo 'gitdir: /nowhere' > .git"]
  - end_turn
  - expect_prompt: "*rewound the worktree to checkpoint 1*third"
  - sleep_ms: 2000
  - end_turn
"#;

/// Wait until the session has entered `state` `times` times in all. Each call subscribes
/// again from the start, so an event can arrive more than once; each counts once.
async fn wait_entered(client: &mut Client, session: &SessionId, state: SessionState, times: usize) {
    subscribe(client, vec![Topic::Session(session.clone())], Some(0)).await;
    let mut seen = std::collections::BTreeSet::new();
    collect_until(client, |e| {
        if &e.session_id == session
            && matches!(&e.payload, EventPayload::SessionState(b) if b.to == state)
        {
            seen.insert(e.seq);
        }
        seen.len() == times
    })
    .await;
}

async fn prompt(client: &mut Client, session: &SessionId, text: &str) -> TurnId {
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.clone(),
            text: text.into(),
        })
        .await
        .unwrap()
        .turn_id
}

async fn rewind(
    client: &mut Client,
    session: &SessionId,
    checkpoint: &CheckpointId,
) -> Result<Session, forge_api::client::ClientError> {
    client
        .call::<SessionRewind>(&SessionRewindParams {
            session_id: session.clone(),
            checkpoint_id: checkpoint.clone(),
        })
        .await
}

/// `(checkpoint, turn, ref)` for each checkpoint event, in order.
fn checkpoints(events: &[Event]) -> Vec<(CheckpointId, Option<TurnId>, String)> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Checkpoint(b) => Some((
                b.checkpoint_id.clone(),
                b.turn_id.clone(),
                b.git_ref.clone(),
            )),
            _ => None,
        })
        .collect()
}

fn turns(events: &[Event]) -> Vec<TurnId> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TurnStarted(b) => Some(b.turn_id.clone()),
            _ => None,
        })
        .collect()
}

fn read(path: PathBuf) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

fn files_at(repo: &Path, rev: &str) -> Vec<String> {
    git(repo, &["ls-tree", "-r", "--name-only", rev])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn each_turn_is_checkpointed_and_rewind_goes_back_to_one() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "--quiet", "-m", "ignore logs"]);
    fake_profile(&repo, "fake", SCRIPT);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: rewind", "first"))
        .await
        .unwrap();
    let id = session.id.clone();
    let wt = repo.join(".worktrees").join(&session.branch);
    wait_entered(&mut client, &id, SessionState::Review, 1).await;
    prompt(&mut client, &id, "second").await;
    wait_entered(&mut client, &id, SessionState::Review, 2).await;

    // One checkpoint per turn, on its shadow ref, holding the files but not ignored output.
    let events = all_events(&mut client, &id).await;
    let cps = checkpoints(&events);
    let started = turns(&events);
    assert_eq!(cps.len(), 2, "{cps:?}");
    for (n, (_, turn, git_ref)) in cps.iter().enumerate() {
        assert_eq!(git_ref, &format!("refs/forge/{id}/cp/{}", n + 1));
        assert_eq!(turn.as_ref(), Some(&started[n]));
    }
    assert_eq!(
        files_at(&repo, &cps[1].2),
        [".gitignore", "README.md", "a.txt", "b.txt"]
    );
    assert_eq!(
        files_at(&repo, &cps[0].2),
        [".gitignore", "README.md", "a.txt"]
    );
    assert_eq!(read(wt.join("out.log")).as_deref(), Some("build\n"));

    // Unknown checkpoints aren't found.
    expect_api_error(
        rewind(&mut client, &id, &CheckpointId::new()).await,
        ErrorKind::NotFound,
    );

    // Back to checkpoint 1: turn two's files are gone, ignored output stays, and the session
    // stays in Review.
    let main_before = git(&repo, &["rev-parse", "main"]);
    let after = rewind(&mut client, &id, &cps[0].0).await.unwrap();
    assert_eq!(after.state, SessionState::Review);
    assert_eq!(read(wt.join("a.txt")).as_deref(), Some("turn one\n"));
    assert_eq!(read(wt.join("b.txt")), None);
    assert_eq!(read(wt.join("out.log")).as_deref(), Some("build\n"));
    assert_eq!(git(&repo, &["rev-parse", "main"]), main_before);
    assert_eq!(
        read(repo.join("a.txt")),
        None,
        "the main checkout is untouched"
    );

    let events = all_events(&mut client, &id).await;
    let rewound: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::SessionRewound(b) => Some(b.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(rewound.len(), 1);
    assert_eq!(rewound[0].checkpoint_id, cps[0].0);
    assert_eq!(rewound[0].discarded_turns, [started[1].clone()]);
    // What the worktree held just before is kept as a checkpoint of no turn, so the rewind
    // can be undone by hand from its ref.
    let cps_now = checkpoints(&events);
    assert_eq!(cps_now.len(), 3);
    assert_eq!(cps_now[2].1, None);
    assert!(files_at(&repo, &cps_now[2].2).contains(&"b.txt".to_owned()));

    // Discarded checkpoints can't be rewound to.
    expect_api_error(
        rewind(&mut client, &id, &cps[1].0).await,
        ErrorKind::InvalidState,
    );

    // The agent hears about the rewind with its next prompt, and nothing rewinds mid-turn.
    prompt(&mut client, &id, "third").await;
    wait_entered(&mut client, &id, SessionState::Working, 3).await;
    expect_api_error(
        rewind(&mut client, &id, &cps[0].0).await,
        ErrorKind::InvalidState,
    );
    wait_entered(&mut client, &id, SessionState::Review, 3).await;
    let events = all_events(&mut client, &id).await;
    let last_prompt = events
        .iter()
        .rev()
        .find_map(|e| match &e.payload {
            EventPayload::Prompt(b) => Some(b.text.clone()),
            _ => None,
        })
        .unwrap();
    assert!(
        last_prompt.contains("rewound the worktree to checkpoint 1"),
        "{last_prompt}"
    );
    // The third turn changed nothing, so its checkpoint follows checkpoint 1, the one rewound
    // to, and reuses its commit: discarded checkpoints aren't part of the history.
    let cps_now = checkpoints(&events);
    assert_eq!(cps_now.len(), 4);
    assert_eq!(cps_now[3].1.as_ref(), turns(&events).last());
    assert_eq!(
        git(&repo, &["rev-parse", &cps_now[3].2]),
        git(&repo, &["rev-parse", &cps[0].2])
    );
    // Every turn ended as scripted.
    assert!(events.iter().all(|e| !matches!(
        &e.payload,
        EventPayload::TurnEnded(b) if b.stop_reason != "end_turn"
    )));
    daemon.stop();
}

#[tokio::test]
async fn a_turn_too_large_to_checkpoint_is_reported_and_the_session_carries_on() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    // Sparse, so the test writes almost nothing; the cap counts the size on disk git sees.
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - exec: ["sh", "-c", "truncate -s 65M big.bin"]
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: big", "go"))
        .await
        .unwrap();
    wait_entered(&mut client, &session.id, SessionState::Review, 1).await;
    let events = all_events(&mut client, &session.id).await;
    assert!(checkpoints(&events).is_empty());
    let errors: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Error(d) => Some((d.kind, d.detail.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].0, ErrorKind::ValidationFailed);
    assert!(
        errors[0].1.starts_with("no checkpoint after this turn: ") && errors[0].1.contains("MiB"),
        "{}",
        errors[0].1
    );
    // Nothing of it reached the repo's objects.
    assert!(
        git(&repo, &["for-each-ref", "refs/forge/"]).is_empty(),
        "a checkpoint ref was made"
    );
    daemon.stop();
}
