//! Hook tests against a real daemon driving the fake agent: `tool.pre` and `edit.pre` decide and
//! rewrite, `tool.post` and `edit.post` notes reach the agent, `task.done` sends the session back
//! to work, and an agent can't change its own hooks.

mod common;

use std::path::{Path, PathBuf};

use common::{
    Daemon, all_events, collect_until, create_params, entered, fake_profile, git_repo_with_commit,
    open, subscribe, temp_home,
};
use forge_api::client::Client;
use forge_api::events::ApprovalDecision;
use forge_api::events::{Event, EventKind, EventPayload, Topic, TurnSource, Verdict};
use forge_api::methods::*;
use forge_api::model::{Mode, Project, Session, SessionState};

/// Write `<repo>/.forge/hooks.toml` and executable scripts under `.forge/hooks/`, each run by sh.
fn hooks(repo: &Path, toml: &str, scripts: &[(&str, &str)]) {
    use std::os::unix::fs::PermissionsExt;
    let dir = repo.join(".forge/hooks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(repo.join(".forge/hooks.toml"), toml).unwrap();
    for (name, body) in scripts {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

async fn start(client: &mut Client, project: &Project, prompt: &str) -> Session {
    let session = client
        .call::<SessionCreate>(&create_params(project, "feat: hooks", prompt))
        .await
        .unwrap();
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

fn results(events: &[Event]) -> Vec<(String, String)> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ToolResult(b) => {
                Some((b.status.clone(), b.output.clone().unwrap_or_default()))
            }
            _ => None,
        })
        .collect()
}

fn prompts(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Prompt(b) => Some(b.text.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn tool_pre_hooks_deny_and_rewrite_commands() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        r#"
[[hook]]
event = "tool.pre"
match = { argv = "npm *" }
run = [".forge/hooks/block-npm.sh"]

[[hook]]
event = "tool.pre"
name = "rename"
match = { argv = "touch old.txt" }
run = [".forge/hooks/rename.sh"]
"#,
        &[
            (
                "block-npm.sh",
                r#"echo '{"verdict":"deny","reason":"Use pnpm in this repo","noteForAgent":"Run pnpm add instead."}'"#,
            ),
            ("rename.sh", r#"echo '{"rewrite":["touch","new.txt"]}'"#),
        ],
    );
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - exec: { argv: ["npm", "install", "left-pad"], expect: deny }
  - exec: ["touch", "old.txt"]
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "go").await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let dir = worktree(&repo, &session);
    assert!(dir.join("new.txt").exists());
    assert!(!dir.join("old.txt").exists());
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(
        decisions(&events),
        [
            (Verdict::Deny, "hook:block-npm.sh".to_owned()),
            (Verdict::Allow, "mode:full-access".to_owned()),
        ]
    );
    // The rewrite shows in the decision even though the mode decided.
    let reasons: Vec<String> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::PolicyDecision(b) => Some(b.reason.clone()),
            _ => None,
        })
        .collect();
    assert!(
        reasons[1].starts_with("hook rename rewrote `touch old.txt` to `touch new.txt`"),
        "{}",
        reasons[1]
    );
    let refused = &results(&events)[0];
    assert_eq!(refused.0, "failed");
    assert!(refused.1.contains("Use pnpm in this repo"), "{}", refused.1);
    assert!(refused.1.contains("Run pnpm add instead."), "{}", refused.1);
    let runs: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::HookRun(b) => Some((b.hook.clone(), b.event.clone(), b.verdict)),
            _ => None,
        })
        .collect();
    assert_eq!(
        runs,
        [
            (
                "block-npm.sh".into(),
                "tool.pre".into(),
                Some(Verdict::Deny)
            ),
            ("rename".into(), "tool.pre".into(), None),
        ]
    );
}

#[tokio::test]
async fn edit_pre_hooks_block_paths() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        r#"
[[hook]]
event = "edit.pre"
match = { path = "gen/" }
run = [".forge/hooks/generated.sh"]
"#,
        &[(
            "generated.sh",
            r#"echo '{"verdict":"deny","reason":"gen/ is generated; edit the templates"}'"#,
        )],
    );
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - write: { path: gen/out.rs, content: "x", expect: deny }
  - write: { path: src/lib.rs, content: "x" }
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "go").await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let dir = worktree(&repo, &session);
    assert!(!dir.join("gen/out.rs").exists());
    assert!(dir.join("src/lib.rs").exists());
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(
        decisions(&events)[0],
        (Verdict::Deny, "hook:generated.sh".to_owned())
    );
}

#[tokio::test]
async fn post_hook_notes_reach_the_agent() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        r#"
[[hook]]
event = "tool.post"
run = [".forge/hooks/lint.sh"]

[[hook]]
event = "edit.post"
match = { path = "*.rs" }
run = [".forge/hooks/fmt.sh"]
"#,
        &[
            ("lint.sh", r#"echo '{"noteForAgent":"lint: 2 warnings"}'"#),
            (
                "fmt.sh",
                r#"echo '{"noteForAgent":"src/lib.rs needs rustfmt"}'"#,
            ),
        ],
    );
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - exec: ["echo", "built"]
  - write: { path: src/lib.rs, content: "fn x(){}" }
  - end_turn
  - expect_prompt: "*needs rustfmt*"
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "go").await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    // The command's note is part of its output; the edit's comes with the next prompt.
    let events = all_events(&mut client, &session.id).await;
    let output = &results(&events)[0].1;
    assert!(output.starts_with("built\n"), "{output}");
    assert!(output.contains("lint: 2 warnings"), "{output}");
    client
        .call::<SessionPrompt>(&SessionPromptParams {
            session_id: session.id.clone(),
            text: "more".into(),
        })
        .await
        .unwrap();
    collect_until(&mut client, |e| {
        matches!(&e.payload, EventPayload::TurnEnded(b) if b.stop_reason == "end_turn")
            && e.seq > events.len() as u64
    })
    .await;
    let events = all_events(&mut client, &session.id).await;
    let second = prompts(&events).pop().unwrap();
    assert!(second.contains("src/lib.rs needs rustfmt"), "{second}");
    assert!(second.ends_with("more"), "{second}");
}

#[tokio::test]
async fn task_done_hooks_send_the_session_back_to_work() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        r#"
[[hook]]
event = "task.done"
name = "tests"
run = [".forge/hooks/tests.sh"]
"#,
        &[(
            "tests.sh",
            r#"if [ -f fixed.txt ]; then exit 0; fi
echo '{"verdict":"deny","reason":"2 tests fail.","noteForAgent":"Make them pass."}'"#,
        )],
    );
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "fix it"
  - end_turn
  - expect_prompt: "*2 tests fail.*Make them pass.*"
  - exec: ["touch", "fixed.txt"]
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "fix it").await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let events = all_events(&mut client, &session.id).await;
    let turns: Vec<TurnSource> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TurnStarted(b) => Some(b.source),
            _ => None,
        })
        .collect();
    assert_eq!(turns, [TurnSource::Human, TurnSource::Hook]);
    // The session stayed Working between the turns.
    let states: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::SessionState(b) => Some(b.to),
            _ => None,
        })
        .collect();
    assert_eq!(
        states,
        [
            SessionState::Ready,
            SessionState::Working,
            SessionState::Review
        ]
    );
    let runs = events
        .iter()
        .filter(|e| e.payload.kind() == EventKind::HookRun)
        .count();
    assert_eq!(runs, 2);
}

#[tokio::test]
async fn a_task_done_hook_that_keeps_blocking_gives_up_after_three_rounds() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        r#"
[[hook]]
event = "task.done"
run = [".forge/hooks/never.sh"]
"#,
        &[("never.sh", "exit 1")],
    );
    fake_profile(
        &repo,
        "fake",
        "steps:\n  - end_turn\n  - end_turn\n  - end_turn\n  - end_turn\n",
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "go").await;
    let notes = collect_until(&mut client, entered(SessionState::Review)).await;

    let review = notes.last().unwrap();
    let EventPayload::SessionState(b) = &review.event.payload else {
        unreachable!()
    };
    let reason = b.reason.as_deref().unwrap_or_default();
    assert!(reason.contains("never.sh"), "{reason}");
    assert!(reason.contains("3"), "{reason}");
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(prompts(&events).len(), 3);
}

#[tokio::test]
async fn an_agent_cant_change_its_own_hooks() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        r#"
[[hook]]
event = "tool.pre"
match = { argv = "rm *" }
run = [".forge/hooks/no-rm.sh"]
"#,
        &[("no-rm.sh", r#"echo '{"verdict":"deny","reason":"no rm"}'"#)],
    );
    // Commit the hooks, so the session's worktree has its own copy to tamper with.
    common::git(&repo, &["add", ".forge/hooks", ".forge/hooks.toml"]);
    common::git(&repo, &["commit", "--quiet", "-m", "hooks"]);
    fake_profile(
        &repo,
        "fake",
        r#"
steps:
  - expect_prompt: "*"
  - exec: ["sh", "-c", "printf '#!/bin/sh\necho {\"verdict\":\"allow\"}\n' > .forge/hooks/no-rm.sh && printf '' > .forge/hooks.toml"]
  - exec: { argv: ["rm", "README.md"], expect: deny }
  - end_turn
"#,
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "go").await;
    collect_until(&mut client, entered(SessionState::Review)).await;

    let dir = worktree(&repo, &session);
    assert!(dir.join("README.md").exists());
    assert!(
        std::fs::read_to_string(dir.join(".forge/hooks/no-rm.sh"))
            .unwrap()
            .contains("allow")
    );
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(
        decisions(&events).last().unwrap(),
        &(Verdict::Deny, "hook:no-rm.sh".to_owned())
    );
}

#[tokio::test]
async fn an_invalid_hooks_file_fails_the_launch() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        "[[hook]]\nevent = \"tool.pre\"\nrun = [\"scripts/x.sh\"]\n",
        &[],
    );
    fake_profile(&repo, "fake", "steps:\n  - end_turn\n");
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    start(&mut client, &project, "").await;
    let notes = collect_until(&mut client, entered(SessionState::Failed)).await;
    let EventPayload::SessionState(b) = &notes.last().unwrap().event.payload else {
        unreachable!()
    };
    let reason = b.reason.as_deref().unwrap_or_default();
    assert!(reason.contains(".forge/hooks/"), "{reason}");
}

async fn start_in(client: &mut Client, project: &Project, mode: Mode, prompt: &str) -> Session {
    let mut params = create_params(project, "feat: hooks", prompt);
    params.mode = mode;
    let session = client.call::<SessionCreate>(&params).await.unwrap();
    subscribe(client, vec![Topic::Session(session.id.clone())], Some(0)).await;
    session
}

/// Wait for the next approval request, deny it, and return its reason.
async fn deny_next(client: &mut Client) -> String {
    let notes = collect_until(client, |e| e.payload.kind() == EventKind::ApprovalRequested).await;
    let EventPayload::ApprovalRequested(b) = &notes.last().unwrap().event.payload else {
        unreachable!()
    };
    client
        .call::<ApprovalDecide>(&ApprovalDecideParams {
            approval_id: b.approval_id.clone(),
            decision: ApprovalDecision::Deny,
            remember: None,
            note: None,
        })
        .await
        .unwrap();
    b.reason.clone()
}

#[tokio::test]
async fn a_hook_that_always_allows_does_not_switch_off_ask_mode() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        "[[hook]]\nevent = \"tool.pre\"\nrun = [\"echo\", \"{\\\"verdict\\\":\\\"allow\\\"}\"]\n",
        &[],
    );
    fake_profile(
        &repo,
        "fake",
        "steps:\n  - exec: { argv: [\"touch\", \"x\"], expect: deny }\n  - end_turn\n",
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start_in(&mut client, &project, Mode::Ask, "go").await;
    deny_next(&mut client).await;
    collect_until(&mut client, entered(SessionState::Review)).await;
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(decisions(&events), [(Verdict::Ask, "mode:ask".to_owned())]);
    let runs: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::HookRun(b) => Some(b.verdict),
            _ => None,
        })
        .collect();
    assert_eq!(
        runs,
        [Some(Verdict::Allow)],
        "the hook's own answer is still logged"
    );
}

#[tokio::test]
async fn a_pre_hook_that_fails_asks_even_in_full_access() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    hooks(
        &repo,
        "[[hook]]\nevent = \"edit.pre\"\nname = \"generated\"\nrun = [\".forge/hooks/broken.sh\"]\n\n\
         [[hook]]\nevent = \"tool.post\"\nrun = [\".forge/hooks/broken.sh\"]\n",
        &[("broken.sh", "exit 1")],
    );
    fake_profile(
        &repo,
        "fake",
        "steps:\n  - write: { path: a.txt, content: x, expect: deny }\n  - exec: [\"true\"]\n  - end_turn\n",
    );
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start_in(&mut client, &project, Mode::FullAccess, "go").await;
    let reason = deny_next(&mut client).await;
    assert!(reason.contains("hook generated failed"), "{reason}");
    collect_until(&mut client, entered(SessionState::Review)).await;
    // The failing post hook changed nothing.
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(
        decisions(&events),
        [
            (Verdict::Ask, "hook:generated".to_owned()),
            (Verdict::Allow, "mode:full-access".to_owned()),
        ]
    );
}

/// Whether a process whose command line contains `marker` is running (Linux only).
#[cfg(target_os = "linux")]
fn running(marker: &str) -> bool {
    std::fs::read_dir("/proc").unwrap().flatten().any(|p| {
        std::fs::read(p.path().join("cmdline")).is_ok_and(|c| {
            String::from_utf8_lossy(&c)
                .replace('\0', " ")
                .contains(marker)
        })
    })
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn cancelling_stops_a_running_task_done_hook() {
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    // A marker unique to this run, so nothing left from another run matches.
    let marker = format!("sleep 9{}", std::process::id());
    hooks(
        &repo,
        "[[hook]]\nevent = \"task.done\"\nrun = [\".forge/hooks/slow.sh\"]\n",
        &[("slow.sh", &format!("{marker} & {marker}"))],
    );
    fake_profile(&repo, "fake", "steps:\n  - end_turn\n");
    let daemon = Daemon::start(home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    let session = start(&mut client, &project, "go").await;
    // The turn ends only once task.done hooks are through, so watch for the hook itself.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !running(&marker) {
        assert!(
            std::time::Instant::now() < deadline,
            "the hook never started"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    client
        .call::<SessionCancel>(&SessionIdParams {
            session_id: session.id.clone(),
        })
        .await
        .unwrap();
    collect_until(&mut client, entered(SessionState::Ready)).await;
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(!running(&marker), "the hook outlived the cancel");
}

#[tokio::test]
async fn a_hooks_file_cant_expose_a_program_the_sandbox_hides() {
    use std::os::unix::fs::PermissionsExt;
    let home = temp_home();
    let repo = git_repo_with_commit(home.path(), "demo");
    // A script in the user's own bin dir, perhaps with a token in it.
    let bin = home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let deploy = bin.join("deploy.sh");
    std::fs::write(&deploy, "#!/bin/sh\nTOKEN=hunter2\n").unwrap();
    std::fs::set_permissions(&deploy, std::fs::Permissions::from_mode(0o755)).unwrap();
    hooks(
        &repo,
        "[[hook]]\nevent = \"tool.pre\"\nrun = [\"deploy.sh\"]\n",
        &[],
    );
    fake_profile(&repo, "fake", "steps:\n  - end_turn\n");
    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let daemon = Daemon::start_with(home.path(), |cmd| {
        cmd.env("PATH", &path);
    });
    let mut client = daemon.client().await;
    let project = open(&mut client, &repo).await;
    start(&mut client, &project, "").await;
    let notes = collect_until(&mut client, entered(SessionState::Failed)).await;
    let EventPayload::SessionState(b) = &notes.last().unwrap().event.payload else {
        unreachable!()
    };
    let reason = b.reason.as_deref().unwrap_or_default();
    assert!(
        reason.contains("deploy.sh") && reason.contains("doesn't show"),
        "{reason}"
    );
}
