//! Reverting hunks and shipping, with the fake agent: `review.revertHunk` puts a hunk's lines
//! back as a commit on the session branch, and `review.ship` merges the reviewed checkpoint
//! into the base branch or pushes it, refusing whenever the worktree or branch moved on since.

mod common;

use std::path::{Path, PathBuf};

use common::{
    Daemon, all_events, collect_until, create_params, expect_api_error, fake_profile, git,
    git_repo, git_repo_with_commit, open, subscribe, temp_home,
};
use forge_api::client::{Client, ClientError};
use forge_api::events::{EventPayload, Topic};
use forge_api::methods::*;
use forge_api::model::{Changeset, Hunk, LineRange, Project, Session, SessionState, ThreadStatus};
use forge_api::{CheckpointId, ErrorKind, SessionId};

/// Hooks that would leave a marker in the repo's git dir if any ran.
const HOOKS: &[&str] = &[
    "pre-commit",
    "commit-msg",
    "post-commit",
    "post-checkout",
    "post-merge",
    "pre-push",
    "reference-transaction",
];

fn daemon(home: &Path) -> Daemon {
    Daemon::start_with(home, |cmd| {
        cmd.env("GIT_AUTHOR_NAME", "Dev")
            .env("GIT_AUTHOR_EMAIL", "dev@example.com")
            .env("GIT_COMMITTER_NAME", "Dev")
            .env("GIT_COMMITTER_EMAIL", "dev@example.com");
    })
}

fn install_hooks(repo: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let dir = repo.join(".git/hooks");
    std::fs::create_dir_all(&dir).unwrap();
    for hook in HOOKS {
        let path = dir.join(hook);
        std::fs::write(
            &path,
            format!("#!/bin/sh\ntouch \"$GIT_DIR/../ran-{hook}\"\n"),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn no_hook_ran(repo: &Path) {
    for hook in HOOKS {
        assert!(
            !repo.join(format!("ran-{hook}")).exists(),
            "the {hook} hook ran"
        );
    }
}

/// `n` numbered lines, with line `k` replaced by `edit(k)` where it says so.
fn lines(n: u32, edit: impl Fn(u32) -> Option<String>) -> String {
    (1..=n)
        .map(|k| edit(k).unwrap_or_else(|| format!("line {k}\n")))
        .collect()
}

fn yaml(text: &str) -> String {
    serde_json::to_string(text).unwrap()
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

async fn start(client: &mut Client, project: &Project, title: &str) -> Session {
    start_with(client, project, title, "fake").await
}

async fn start_with(client: &mut Client, project: &Project, title: &str, agent: &str) -> Session {
    let mut params = create_params(project, title, "go");
    params.agent_profile = agent.into();
    let session = client.call::<SessionCreate>(&params).await.unwrap();
    wait_entered(client, &session.id, SessionState::Review, 1).await;
    session
}

fn worktree(repo: &Path, session: &Session) -> PathBuf {
    repo.join(".worktrees").join(&session.branch)
}

async fn changeset(client: &mut Client, id: &SessionId) -> Changeset {
    client
        .call::<ReviewChangeset>(&ReviewChangesetParams {
            session_id: id.clone(),
            from_checkpoint: None,
        })
        .await
        .unwrap()
}

async fn hunks(
    client: &mut Client,
    id: &SessionId,
    path: &str,
    from: Option<&CheckpointId>,
) -> Vec<Hunk> {
    client
        .call::<ReviewFileDiff>(&ReviewFileDiffParams {
            session_id: id.clone(),
            repo: None,
            path: path.into(),
            from_checkpoint: from.cloned(),
        })
        .await
        .unwrap()
        .hunks
}

async fn revert(
    client: &mut Client,
    id: &SessionId,
    path: &str,
    hunk: &Hunk,
    from: Option<&CheckpointId>,
) -> Result<Empty, ClientError> {
    client
        .call::<ReviewRevertHunk>(&ReviewRevertHunkParams {
            session_id: id.clone(),
            repo: None,
            path: path.into(),
            hunk_id: hunk.id.clone(),
            from_checkpoint: from.cloned(),
        })
        .await
}

fn ship_params(id: &SessionId, checkpoint: &str, strategy: ShipStrategy) -> ReviewShipParams {
    ReviewShipParams {
        session_id: id.clone(),
        checkpoint_id: checkpoint.parse().unwrap(),
        strategy,
        branch: None,
        include_withheld: None,
    }
}

async fn ship(client: &mut Client, params: &ReviewShipParams) -> Result<ShipResult, ClientError> {
    client.call::<ReviewShip>(params).await
}

async fn state(client: &mut Client, id: &SessionId) -> SessionState {
    client
        .call::<SessionGet>(&SessionIdParams {
            session_id: id.clone(),
        })
        .await
        .unwrap()
        .state
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn subjects(repo: &Path, rev: &str, n: usize) -> Vec<String> {
    git(repo, &["log", &format!("-{n}"), "--format=%s", rev])
        .lines()
        .map(str::to_owned)
        .collect()
}

#[tokio::test]
async fn reverting_a_hunk_puts_its_lines_back_as_a_commit_the_agent_hears_about() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    std::fs::write(repo.join("lib.rs"), lines(30, |_| None)).unwrap();
    git(&repo, &["add", "lib.rs"]);
    git(&repo, &["commit", "--quiet", "-m", "lib"]);
    install_hooks(&repo);
    let edited = |k: u32| match k {
        3 => Some("three\n".to_owned()),
        25 => Some("line 25\nextra\n".to_owned()),
        _ => None,
    };
    // After the revert of line 3's hunk, the agent edits line 10 on top of what's left.
    let second = |k: u32| match k {
        10 => Some("ten\n".to_owned()),
        k => edited(k).filter(|_| k == 25),
    };
    let script = format!(
        r#"
steps:
  - write: {{ path: lib.rs, content: {one} }}
  - end_turn
  - expect_prompt: "*reverted*again"
  - write: {{ path: lib.rs, content: {two} }}
  - sleep_ms: 1500
  - end_turn
"#,
        one = yaml(&lines(30, edited)),
        two = yaml(&lines(30, second)),
    );
    fake_profile(&repo, "fake", &script);
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "feat: revert").await;
    let (id, wt) = (session.id.clone(), worktree(&repo, &session));

    let before = hunks(&mut client, &id, "lib.rs", None).await;
    assert_eq!(before.len(), 2, "{before:?}");
    let thread = client
        .call::<ReviewComment>(&ReviewCommentParams {
            session_id: id.clone(),
            repo: None,
            path: "lib.rs".into(),
            range: LineRange {
                start_line: 3,
                end_line: 3,
            },
            body: "Why spell it out?".into(),
        })
        .await
        .unwrap();

    // A hunk the diff doesn't have.
    let mut unknown = before[0].clone();
    unknown.id = forge_api::HunkId::new();
    expect_api_error(
        revert(&mut client, &id, "lib.rs", &unknown, None).await,
        ErrorKind::NotFound,
    );
    // A worktree that changed since the newest checkpoint.
    std::fs::write(wt.join("stray.txt"), "stray\n").unwrap();
    expect_api_error(
        revert(&mut client, &id, "lib.rs", &before[0], None).await,
        ErrorKind::Conflict,
    );
    std::fs::remove_file(wt.join("stray.txt")).unwrap();

    revert(&mut client, &id, "lib.rs", &before[0], None)
        .await
        .unwrap();
    let reverted = lines(30, |k| edited(k).filter(|_| k == 25));
    assert_eq!(read(wt.join("lib.rs")), reverted);
    // The agent's uncommitted edit went on the branch first, then the revert.
    let log = subjects(&repo, &session.branch, 2);
    assert!(log[0].starts_with("Revert a hunk of lib.rs"), "{log:?}");
    assert!(
        log[1].starts_with("Changes from Forge checkpoint"),
        "{log:?}"
    );
    assert_eq!(
        git(&repo, &["show", &format!("{}:lib.rs", session.branch)]),
        reverted
    );
    assert_eq!(git(&wt, &["status", "--porcelain"]), "");
    // A checkpoint of no turn holds it, so the review shows only the other hunk, and the
    // thread on the reverted line is outdated.
    let events = all_events(&mut client, &id).await;
    let last = events
        .iter()
        .rev()
        .find_map(|e| match &e.payload {
            EventPayload::Checkpoint(b) => Some(b.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(last.turn_id, None);
    let files = changeset(&mut client, &id).await.files;
    let lib = files.iter().find(|f| f.path == "lib.rs").unwrap();
    assert_eq!((lib.additions, lib.deletions), (1, 0));
    let threads = client
        .call::<ReviewThreads>(&ReviewThreadsParams {
            session_id: id.clone(),
            status: None,
        })
        .await
        .unwrap();
    assert_eq!(threads[0].id, thread.id);
    assert_eq!(threads[0].status, ThreadStatus::Outdated);

    // Not while the agent is working; the agent's prompt says what happened.
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: id.clone(),
            text: "again".into(),
        })
        .await
        .unwrap();
    let now = hunks(&mut client, &id, "lib.rs", None).await;
    expect_api_error(
        revert(&mut client, &id, "lib.rs", &now[0], None).await,
        ErrorKind::InvalidState,
    );
    wait_entered(&mut client, &id, SessionState::Review, 2).await;

    // A hunk of the changes since a checkpoint reverts to that checkpoint's lines.
    let from: CheckpointId = last.checkpoint_id.clone();
    let since = hunks(&mut client, &id, "lib.rs", Some(&from)).await;
    assert_eq!(since.len(), 1, "{since:?}");
    // The same hunk's ID differs in the whole changeset's diff.
    expect_api_error(
        revert(&mut client, &id, "lib.rs", &since[0], None).await,
        ErrorKind::NotFound,
    );
    revert(&mut client, &id, "lib.rs", &since[0], Some(&from))
        .await
        .unwrap();
    assert_eq!(read(wt.join("lib.rs")), reverted);
    no_hook_ran(&repo);
}

#[tokio::test]
async fn merge_local_ships_exactly_the_reviewed_checkpoint() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    install_hooks(&repo);
    let script = r#"
steps:
  - write: { path: a.txt, content: "from the agent\n" }
  - exec: ["sh", "-c", "yes x | head -n 1000001 > big.txt"]
  - end_turn
"#;
    fake_profile(&repo, "fake", script);
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "feat: ship").await;
    let (id, wt) = (session.id.clone(), worktree(&repo, &session));
    let reviewed = changeset(&mut client, &id).await;
    let params = ship_params(&id, &reviewed.head, ShipStrategy::MergeLocal);

    // A branch name is for pushing only.
    let mut named = params.clone();
    named.branch = Some("elsewhere".into());
    expect_api_error(ship(&mut client, &named).await, ErrorKind::ValidationFailed);
    // A file withheld from review is named, and ships only when the human says so.
    let withheld = expect_api_error(
        ship(&mut client, &params).await,
        ErrorKind::ValidationFailed,
    );
    assert!(withheld.detail().contains("big.txt"), "{withheld:?}");
    let mut params = params;
    params.include_withheld = Some(true);
    // Anything that changed after the review.
    std::fs::write(wt.join("a.txt"), "changed after review\n").unwrap();
    expect_api_error(ship(&mut client, &params).await, ErrorKind::Conflict);
    std::fs::write(wt.join("a.txt"), "from the agent\n").unwrap();
    // A checkout of the base branch with changes of its own isn't touched.
    std::fs::write(repo.join("README.md"), "# mine\n").unwrap();
    expect_api_error(ship(&mut client, &params).await, ErrorKind::Conflict);
    std::fs::write(repo.join("README.md"), "# demo\n").unwrap();
    let base_before = git(&repo, &["rev-parse", "main"]);
    assert_eq!(subjects(&repo, "main", 1), ["initial"]);

    let shipped = ship(&mut client, &params).await.unwrap();
    assert_eq!(shipped.git_ref, "refs/heads/main");
    // A fast-forward: main is the session branch, whose last commit holds what the agent left
    // uncommitted at the reviewed checkpoint.
    assert_ne!(git(&repo, &["rev-parse", "main"]), base_before);
    assert_eq!(
        git(&repo, &["rev-parse", "main"]),
        git(&repo, &["rev-parse", &session.branch])
    );
    assert!(subjects(&repo, "main", 1)[0].starts_with("Changes from Forge checkpoint"));
    assert_eq!(read(repo.join("a.txt")), "from the agent\n");
    assert!(repo.join("big.txt").exists());
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=no"]),
        ""
    );
    assert_eq!(state(&mut client, &id).await, SessionState::Done);
    no_hook_ran(&repo);
}

#[tokio::test]
async fn merge_local_merges_a_base_that_moved_and_names_conflicts() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    // Both sessions change a.txt, so a conflict between them is between two edits.
    std::fs::write(repo.join("a.txt"), "base\n").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "a"]);
    let script = r#"
steps:
  - write: { path: a.txt, content: "agent\n" }
  - mcp_call: { tool: git_commit, arguments: { message: "add a" }, expect: allow }
  - end_turn
"#;
    fake_profile(&repo, "fake", script);
    // The other agent also asks for conflicts to be settled by keeping both sides; merge
    // settings are the base branch's, never the session's.
    let other = script.replace(
        r#"  - write: { path: a.txt, content: "agent\n" }"#,
        r#"  - write: { path: a.txt, content: "other\n" }
  - write: { path: .gitattributes, content: "* merge=union\n" }"#,
    );
    assert_ne!(other, script);
    fake_profile(&repo, "other", &other);
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;

    // Both from the same base.
    let clean = start(&mut client, &project, "feat: clean").await;
    let clash = start_with(&mut client, &project, "feat: clash", "other").await;

    // The base moved, touching another file: a merge commit.
    std::fs::write(repo.join("c.txt"), "human\n").unwrap();
    git(&repo, &["add", "c.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "c"]);
    let head = changeset(&mut client, &clean.id).await.head;
    ship(
        &mut client,
        &ship_params(&clean.id, &head, ShipStrategy::MergeLocal),
    )
    .await
    .unwrap();
    let parents = git(&repo, &["log", "-1", "--format=%P", "main"]);
    assert_eq!(parents.split_whitespace().count(), 2, "{parents}");
    assert_eq!(read(repo.join("a.txt")), "agent\n");
    assert_eq!(read(repo.join("c.txt")), "human\n");

    // The base now has its own a.txt: the conflict is named and nothing moves.
    std::fs::write(repo.join("a.txt"), "someone else\n").unwrap();
    git(&repo, &["commit", "--quiet", "-am", "theirs"]);
    let base_before = git(&repo, &["rev-parse", "main"]);
    let head = changeset(&mut client, &clash.id).await.head;
    let err = expect_api_error(
        ship(
            &mut client,
            &ship_params(&clash.id, &head, ShipStrategy::MergeLocal),
        )
        .await,
        ErrorKind::Conflict,
    );
    assert!(err.detail().contains("a.txt"), "{err:?}");
    assert_eq!(git(&repo, &["rev-parse", "main"]), base_before);
    assert_eq!(
        git(&repo, &["status", "--porcelain", "--untracked-files=no"]),
        ""
    );
    assert_eq!(state(&mut client, &clash.id).await, SessionState::Review);
}

#[tokio::test]
async fn push_branch_pushes_the_reviewed_commit_without_force() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    let remote = git_repo(home.path(), "remote.git");
    git(&remote, &["config", "receive.denyCurrentBranch", "ignore"]);
    git(
        &repo,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repo, &["push", "--quiet", "origin", "main"]);
    // A branch on the remote the session's history doesn't contain.
    git(&repo, &["checkout", "--quiet", "-b", "taken"]);
    std::fs::write(repo.join("t.txt"), "taken\n").unwrap();
    git(&repo, &["add", "t.txt"]);
    git(&repo, &["commit", "--quiet", "-m", "taken"]);
    git(&repo, &["push", "--quiet", "origin", "taken"]);
    git(&repo, &["checkout", "--quiet", "main"]);
    install_hooks(&repo);
    let script = r#"
steps:
  - write: { path: a.txt, content: "one\n" }
  - end_turn
  - expect_prompt: "more"
  - write: { path: a.txt, content: "two\n" }
  - end_turn
"#;
    fake_profile(&repo, "fake", script);
    let daemon = daemon(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "feat: push").await;
    let id = session.id.clone();
    let first = changeset(&mut client, &id).await.head;

    let mut bad = ship_params(&id, &first, ShipStrategy::PushBranch);
    bad.branch = Some("-x".into());
    expect_api_error(ship(&mut client, &bad).await, ErrorKind::ValidationFailed);
    let mut taken = ship_params(&id, &first, ShipStrategy::PushBranch);
    taken.branch = Some("taken".into());
    expect_api_error(ship(&mut client, &taken).await, ErrorKind::Conflict);

    let pushed = ship(
        &mut client,
        &ship_params(&id, &first, ShipStrategy::PushBranch),
    )
    .await
    .unwrap();
    assert_eq!(pushed.git_ref, format!("origin/{}", session.branch));
    let tip = git(&repo, &["rev-parse", &session.branch]);
    assert_eq!(git(&remote, &["rev-parse", &session.branch]), tip);
    assert_eq!(state(&mut client, &id).await, SessionState::Review);

    // Another turn: the first checkpoint is no longer what would ship.
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: id.clone(),
            text: "more".into(),
        })
        .await
        .unwrap();
    wait_entered(&mut client, &id, SessionState::Review, 2).await;
    let stale = expect_api_error(
        ship(
            &mut client,
            &ship_params(&id, &first, ShipStrategy::PushBranch),
        )
        .await,
        ErrorKind::Conflict,
    );
    assert!(
        stale.detail().contains("isn't the session's newest"),
        "{stale:?}"
    );
    let second = changeset(&mut client, &id).await.head;
    let mut renamed = ship_params(&id, &second, ShipStrategy::PushBranch);
    renamed.branch = Some("feat/renamed".into());
    let pushed = ship(&mut client, &renamed).await.unwrap();
    assert_eq!(pushed.git_ref, "origin/feat/renamed");
    assert_eq!(git(&remote, &["show", "feat/renamed:a.txt"]), "two\n");
    // Pushing again goes on top of what's there.
    ship(
        &mut client,
        &ship_params(&id, &second, ShipStrategy::PushBranch),
    )
    .await
    .unwrap();
    assert_eq!(
        git(&remote, &["rev-parse", &session.branch]),
        git(&repo, &["rev-parse", &session.branch])
    );
    no_hook_ran(&repo);
}
