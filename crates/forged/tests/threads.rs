//! Review threads and requests for changes, with the fake agent: a change, a comment, Request
//! changes, the agent's fix and its reply through `review_reply`, and the thread resolved.

mod common;

use common::{
    Daemon, all_events, collect_until, create_params, expect_api_error, fake_profile,
    git_repo_with_commit, open, subscribe, temp_home,
};
use forge_api::client::{Client, ClientError};
use forge_api::events::{EventPayload, ThreadChange, Topic, TurnSource};
use forge_api::methods::*;
use forge_api::model::{CommentAuthor, LineRange, SessionState, Thread, ThreadStatus};
use forge_api::{ErrorKind, SessionId, ThreadId};

/// A token the agent echoes in its reply, which the thread must never hold.
const TOKEN: &str = "ghp_a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";

/// Ten lines, the fourth holding `max`.
fn lib(max: &str) -> String {
    (1..=10)
        .map(|n| match n {
            4 => format!("let max = {max};\n"),
            n => format!("// line {n}\n"),
        })
        .collect()
}

fn script() -> String {
    let yaml = |text: String| serde_json::to_string(&text).unwrap();
    format!(
        r#"
steps:
  - write: {{ path: lib.rs, content: {one} }}
  - write: {{ path: notes.txt, content: "notes\n" }}
  - end_turn
  - expect_review_comments: 1
  - expect_prompt: "*MAX should come from config*"
  - write: {{ path: lib.rs, content: {two} }}
  - reply_thread: {{ index: 0, body: "Moved to RetryConfig::max_attempts; tested with {TOKEN}" }}
  - end_turn
"#,
        one = yaml(lib("3")),
        two = yaml(lib("config.max_attempts")),
    )
}

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

async fn comment(
    client: &mut Client,
    session: &SessionId,
    path: &str,
    lines: (u32, u32),
    body: &str,
) -> Result<Thread, ClientError> {
    client
        .call::<ReviewComment>(&ReviewCommentParams {
            session_id: session.clone(),
            repo: None,
            path: path.into(),
            range: LineRange {
                start_line: lines.0,
                end_line: lines.1,
            },
            body: body.into(),
        })
        .await
}

async fn threads(client: &mut Client, session: &SessionId) -> Vec<Thread> {
    client
        .call::<ReviewThreads>(&ReviewThreadsParams {
            session_id: session.clone(),
            status: None,
        })
        .await
        .unwrap()
}

async fn changeset(client: &mut Client, session: &SessionId) -> forge_api::model::Changeset {
    client
        .call::<ReviewChangeset>(&ReviewChangesetParams {
            session_id: session.clone(),
            from_checkpoint: None,
        })
        .await
        .unwrap()
}

async fn set_viewed(client: &mut Client, session: &SessionId, path: &str, viewed: bool) {
    client
        .call::<ReviewSetViewed>(&ReviewSetViewedParams {
            session_id: session.clone(),
            repo: None,
            path: path.into(),
            viewed,
        })
        .await
        .unwrap();
}

/// Each `review.thread` event's change, in order, for `thread`.
fn thread_changes(events: &[forge_api::events::Event], thread: &ThreadId) -> Vec<ThreadChange> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ReviewThread(b) if &b.thread_id == thread => Some(b.change),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_comment_goes_to_the_agent_and_its_reply_comes_back_to_the_thread() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", &script());
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: retry", "go"))
        .await
        .unwrap();
    let id = session.id.clone();
    wait_entered(&mut client, &id, SessionState::Review, 1).await;

    // Round 1: nothing reviewed yet; files can be marked viewed.
    let first = changeset(&mut client, &id).await;
    assert_eq!((first.round, first.last_reviewed.as_ref()), (1, None));
    let cp1 = first.head.clone();
    set_viewed(&mut client, &id, "lib.rs", true).await;
    let viewed: Vec<_> = changeset(&mut client, &id)
        .await
        .files
        .into_iter()
        .map(|f| (f.path, f.viewed))
        .collect();
    assert_eq!(
        viewed,
        [("lib.rs".to_owned(), true), ("notes.txt".to_owned(), false)]
    );

    // A comment on line 4 is a thread on checkpoint 1, quoting it.
    let body = "MAX should come from config, not a constant.";
    let thread = comment(&mut client, &id, "lib.rs", (4, 4), body)
        .await
        .unwrap();
    assert_eq!(thread.status, ThreadStatus::Open);
    assert_eq!(thread.checkpoint_id.to_string(), cp1);
    assert_eq!(thread.repo.as_deref(), Some(&*project.repos[0].name));
    assert_eq!(thread.quote, ["let max = 3;"]);
    assert_eq!(
        thread.current,
        Some(LineRange {
            start_line: 4,
            end_line: 4
        })
    );
    assert_eq!(thread.comments.len(), 1);
    assert_eq!(thread.comments[0].author, CommentAuthor::Human);
    // Only on lines of a file in the changeset, with something to say.
    for (path, lines, text) in [
        ("README.md", (1, 1), "not changed"),
        ("lib.rs", (10, 11), "past the end"),
        ("lib.rs", (0, 1), "line zero"),
        ("lib.rs", (5, 4), "backwards"),
        ("lib.rs", (1, 1), "  "),
    ] {
        expect_api_error(
            comment(&mut client, &id, path, lines, text).await,
            ErrorKind::ValidationFailed,
        );
    }
    let long = "x".repeat(16 * 1024 + 1);
    expect_api_error(
        comment(&mut client, &id, "lib.rs", (1, 1), &long).await,
        ErrorKind::ValidationFailed,
    );
    assert_eq!(
        threads(&mut client, &id).await,
        std::slice::from_ref(&thread)
    );

    // Request changes: one structured message, a review turn, and round 2 after it.
    let turn = client
        .call::<ReviewRequestChanges>(&SessionIdParams {
            session_id: id.clone(),
        })
        .await
        .unwrap()
        .turn_id;
    wait_entered(&mut client, &id, SessionState::Review, 2).await;
    let events = all_events(&mut client, &id).await;
    let started = events
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::TurnStarted(b) if b.turn_id == turn => Some(b.source),
            _ => None,
        })
        .unwrap();
    assert_eq!(started, TurnSource::Review);
    let second = changeset(&mut client, &id).await;
    assert_eq!(second.round, 2);
    assert_eq!(
        second.last_reviewed.as_ref().map(|c| c.to_string()),
        Some(cp1.clone())
    );
    // Viewed is per round.
    assert!(second.files.iter().all(|f| !f.viewed));
    // Since last review: only the fix, not the notes file the first round already saw.
    let since = client
        .call::<ReviewChangeset>(&ReviewChangesetParams {
            session_id: id.clone(),
            from_checkpoint: second.last_reviewed.clone(),
        })
        .await
        .unwrap();
    let changed: Vec<_> = since
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.additions, f.deletions))
        .collect();
    assert_eq!(changed, [("lib.rs", 1, 1)]);

    // The agent's fix changed the quoted line, so the thread is outdated, and its reply is in.
    let after = threads(&mut client, &id).await;
    assert_eq!(after.len(), 1);
    let after = &after[0];
    assert_eq!(after.status, ThreadStatus::Outdated);
    assert_eq!(after.current, None);
    assert_eq!(after.quote, ["let max = 3;"]);
    let said: Vec<_> = after.comments.iter().map(|c| c.author).collect();
    assert_eq!(said, [CommentAuthor::Human, CommentAuthor::Agent]);
    assert_eq!(after.comments[0].body, body);
    // The agent's reply, with the secret it echoed redacted.
    let reply = &after.comments[1].body;
    assert!(
        reply.starts_with("Moved to RetryConfig::max_attempts; tested with "),
        "{reply}"
    );
    assert!(
        !reply.contains(TOKEN) && !reply.contains("ghp_a1B2"),
        "{reply}"
    );
    // Nothing is left to send: the agent has answered every thread.
    expect_api_error(
        client
            .call::<ReviewRequestChanges>(&SessionIdParams {
                session_id: id.clone(),
            })
            .await,
        ErrorKind::ValidationFailed,
    );

    // The human resolves it from outdated; it stays resolved, replies and all.
    let resolve = |thread: ThreadId| ThreadIdParams { thread_id: thread };
    let resolved = client
        .call::<ReviewResolve>(&resolve(thread.id.clone()))
        .await
        .unwrap();
    assert_eq!(resolved.status, ThreadStatus::Resolved);
    expect_api_error(
        client
            .call::<ReviewResolve>(&resolve(thread.id.clone()))
            .await,
        ErrorKind::InvalidState,
    );
    let replied = client
        .call::<ReviewReply>(&ReviewReplyParams {
            thread_id: thread.id.clone(),
            body: "Thanks.".into(),
        })
        .await
        .unwrap();
    assert_eq!(replied.status, ThreadStatus::Resolved);
    assert_eq!(replied.comments.len(), 3);
    expect_api_error(
        client
            .call::<ReviewResolve>(&resolve(ThreadId::new()))
            .await,
        ErrorKind::NotFound,
    );
    let events = all_events(&mut client, &id).await;
    assert_eq!(
        thread_changes(&events, &thread.id),
        [
            ThreadChange::Created,
            ThreadChange::Reply,
            ThreadChange::Outdated,
            ThreadChange::Resolved,
            ThreadChange::Reply,
        ]
    );

    // Another session's agent can't reply to this thread, nor see it.
    fake_profile(
        &repo,
        "other",
        &format!(
            r#"
steps:
  - mcp_call: {{ tool: review_reply, arguments: {{ threadId: "{}", body: "mine now" }}, expect: deny }}
  - end_turn
"#,
            thread.id
        ),
    );
    let mut params = create_params(&project, "feat: other", "go");
    params.agent_profile = "other".into();
    let other = client.call::<SessionCreate>(&params).await.unwrap();
    wait_entered(&mut client, &other.id, SessionState::Review, 1).await;
    let other_events = all_events(&mut client, &other.id).await;
    assert!(
        other_events.iter().all(|e| !matches!(
            &e.payload,
            EventPayload::TurnEnded(b) if b.stop_reason != "end_turn"
        )),
        "the other session's reply wasn't refused"
    );
    assert_eq!(threads(&mut client, &id).await[0].comments.len(), 3);
    assert!(threads(&mut client, &other.id).await.is_empty());
    daemon.stop();
}

#[tokio::test]
async fn a_thread_follows_its_lines_and_a_rewind_that_takes_them_outdates_it() {
    let numbered = |extra: &str| -> String {
        let mut text = String::from(extra);
        text.extend((1..=10).map(|n| format!("{n}\n")));
        text
    };
    let yaml = |text: String| serde_json::to_string(&text).unwrap();
    let script = format!(
        r#"
steps:
  - write: {{ path: a.txt, content: {one} }}
  - end_turn
  - expect_prompt: "second"
  - write: {{ path: a.txt, content: {two} }}
  - end_turn
"#,
        one = yaml(numbered("")),
        two = yaml(numbered("new\nnew\n")),
    );
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", &script);
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: follow", "go"))
        .await
        .unwrap();
    let id = session.id.clone();
    wait_entered(&mut client, &id, SessionState::Review, 1).await;
    let cp1 = changeset(&mut client, &id).await.head;
    let on_five = comment(&mut client, &id, "a.txt", (5, 6), "about five")
        .await
        .unwrap();
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: id.clone(),
            text: "second".into(),
        })
        .await
        .unwrap();
    wait_entered(&mut client, &id, SessionState::Review, 2).await;
    // Two lines went in above: the thread follows its lines down.
    let now = threads(&mut client, &id).await;
    assert_eq!(now[0].status, ThreadStatus::Open);
    assert_eq!(
        now[0].current,
        Some(LineRange {
            start_line: 7,
            end_line: 8
        })
    );
    assert_eq!(now[0].range.start_line, 5);
    let on_new = comment(&mut client, &id, "a.txt", (1, 1), "why this line?")
        .await
        .unwrap();
    assert_eq!(on_new.quote, ["new"]);

    // Rewinding to checkpoint 1 takes the line the second thread is on; the first is back
    // where it started.
    client
        .call::<SessionRewind>(&SessionRewindParams {
            session_id: id.clone(),
            checkpoint_id: cp1.parse().unwrap(),
        })
        .await
        .unwrap();
    let now = threads(&mut client, &id).await;
    let by_id = |t: &Thread| {
        now.iter()
            .find(|n| n.id == t.id)
            .map(|n| (n.status, n.current))
            .unwrap()
    };
    assert_eq!(
        by_id(&on_five),
        (
            ThreadStatus::Open,
            Some(LineRange {
                start_line: 5,
                end_line: 6
            })
        )
    );
    assert_eq!(by_id(&on_new), (ThreadStatus::Outdated, None));
    daemon.stop();
}
