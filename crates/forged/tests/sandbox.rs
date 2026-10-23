//! Sessions inside each sandbox tier, against a real daemon driving the fake agent. The tier
//! tests need a sandbox-capable host and run with `just test-sandbox`; a tier this machine
//! can't run fails them instead of skipping.

mod common;

use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use common::expect_api_error;
use common::{
    Daemon, all_events, create_params, git, git_repo_with_commit, hello_tiers, open, sandbox_bins,
    temp_home, transitions, wait_for_state,
};
#[cfg(target_os = "linux")]
use forge_api::ErrorKind;
use forge_api::SessionId;
use forge_api::events::{Event, EventPayload, Verdict};
use forge_api::methods::*;
use forge_api::model::{SandboxTier, SessionState};

/// Edits two files and runs one command, as in the session tests, then tries to read an SSH
/// key from the real home and to reach a host off the allowlist, through terminals.
fn script(key: &Path, probe: &Path) -> String {
    format!(
        r#"
steps:
  - expect_prompt: "*greeting*"
  - plan: ["Write files", "Run a command"]
  - read: README.md
  - write: {{ path: hello.txt, content: "hello\n" }}
  - write: {{ path: src/lib.txt, content: "one\ntwo\n" }}
  - exec: ["sh", "-c", "cat hello.txt"]
  - exec: ["sh", "-c", "cat {} || echo key-hidden"]
  - exec: ["{}", "proxy-connect", "blocked.example:443"]
  - message: "Done."
  - end_turn
"#,
        key.display(),
        probe.display()
    )
}

/// A home with an SSH key, a repo, and a fake-agent profile for `tier` whose script lives in
/// the hidden home, made visible through the user config's `[sandbox] read_only`.
struct Setup {
    home: tempfile::TempDir,
    repo: PathBuf,
    key: PathBuf,
}

fn setup(tier: &str) -> Setup {
    let home = temp_home();
    let key = home.path().join(".ssh/id_test");
    std::fs::create_dir_all(key.parent().unwrap()).unwrap();
    std::fs::write(&key, "PRIVATE KEY\n").unwrap();
    let repo = git_repo_with_commit(home.path(), "demo");
    let scripts = home.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let script_path = scripts.join("fake.yaml");
    let probe = &sandbox_bins()[1];
    std::fs::write(&script_path, script(&key, probe)).unwrap();
    std::fs::create_dir_all(repo.join(".forge/agents")).unwrap();
    let opt_in = if tier == "unsandboxed" {
        "allow_unsandboxed = true\n"
    } else {
        ""
    };
    std::fs::write(
        repo.join(".forge/agents/fake.toml"),
        format!(
            "agent = \"fake\"\ncommand = [{:?}, {:?}]\nsandbox = \"{tier}\"\n{opt_in}[network]\nallow = [\"allowed.example\"]\n",
            common::fake_agent().to_str().unwrap(),
            script_path.to_str().unwrap(),
        ),
    )
    .unwrap();
    std::fs::write(repo.join(".git/info/exclude"), ".forge/\n").unwrap();
    std::fs::write(
        home.path().join("config/config.toml"),
        format!(
            "[sandbox]\nread_only = [{:?}, {:?}]\n",
            scripts.to_str().unwrap(),
            probe.to_str().unwrap()
        ),
    )
    .unwrap();
    Setup { home, repo, key }
}

async fn require_tier(client: &mut forge_api::client::Client, tier: SandboxTier) {
    let tiers = hello_tiers(client).await;
    assert!(
        tiers.contains(&tier),
        "the {tier:?} tier is unavailable on this machine (server.hello lists {tiers:?}); see the daemon log for why"
    );
}

fn terminal_output(events: &[Event]) -> String {
    events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::TerminalOutput(b) => Some(b.chunk.as_str()),
            _ => None,
        })
        .collect()
}

/// The session done-when test, inside `tier`.
async fn fake_agent_in(tier: SandboxTier) {
    sandbox_bins();
    let s = setup(forge_sandbox::tier_name(tier));
    let daemon = Daemon::start(s.home.path());
    let mut client = daemon.client().await;
    require_tier(&mut client, tier).await;
    let project = open(&mut client, &s.repo).await;
    let mut params = create_params(&project, "feat: add greeting", "add a greeting");
    params.sandbox = None;
    let session = client.call::<SessionCreate>(&params).await.unwrap();
    assert_eq!(session.sandbox, tier, "the profile's tier is used");

    wait_for_state(&mut client, &session.id, SessionState::Review).await;
    let events = all_events(&mut client, &session.id).await;
    assert_eq!(
        transitions(&events),
        vec![
            (SessionState::Provisioning, SessionState::Ready),
            (SessionState::Ready, SessionState::Working),
            (SessionState::Working, SessionState::Review),
        ]
    );
    let edited: Vec<&str> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Edit(b) => Some(b.path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(edited, ["hello.txt", "src/lib.txt"]);
    let output = terminal_output(&events);
    assert!(output.contains("hello"), "the command ran: {output:?}");
    if tier != SandboxTier::Unsandboxed {
        assert!(output.contains("key-hidden"), "{output:?}");
        assert!(
            !output.contains("PRIVATE KEY"),
            "the SSH key leaked: {output:?}"
        );
    }
    assert!(output.contains("proxy refused: HTTP/1.1 403"), "{output:?}");
    let denied = events.iter().find_map(|e| match &e.payload {
        EventPayload::Network(b) if b.host == "blocked.example" => Some(b.clone()),
        _ => None,
    });
    let denied = denied.expect("the refused connection is in the event log");
    assert_eq!((denied.port, denied.verdict), (443, Verdict::Deny));
    assert!(denied.reason.unwrap_or_default().contains("allowlist"));
    let worktree = s.repo.join(".worktrees/feat/add-greeting");
    assert_eq!(
        std::fs::read_to_string(worktree.join("hello.txt")).unwrap(),
        "hello\n"
    );
    assert!(s.key.exists());
}

/// Unsandboxed sessions get the egress proxy too; nothing forces them to use it.
#[tokio::test]
async fn fake_agent_edits_and_runs_a_command_unsandboxed() {
    fake_agent_in(SandboxTier::Unsandboxed).await;
}

#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn fake_agent_edits_and_runs_a_command_in_light() {
    fake_agent_in(SandboxTier::Light).await;
}

#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn fake_agent_edits_and_runs_a_command_in_standard() {
    fake_agent_in(SandboxTier::Standard).await;
}

/// A sandbox that can't start fails the session with the reason, and never runs the agent
/// some weaker way. Here the worktree's `.git` link was rewritten to point at another
/// worktree's git dir, as an agent could do.
#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn a_sandbox_that_cannot_start_fails_the_session() {
    sandbox_bins();
    let s = setup("standard");
    let daemon = Daemon::start(s.home.path());
    let mut client = daemon.client().await;
    let tier = *hello_tiers(&mut client)
        .await
        .iter()
        .find(|t| **t != SandboxTier::Unsandboxed)
        .expect("a sandbox tier is available");
    let project = open(&mut client, &s.repo).await;

    git(
        &s.repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/x",
            ".worktrees/feat/x",
        ],
    );
    git(
        &s.repo,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "feat/y",
            ".worktrees/feat/y",
        ],
    );
    let other = std::fs::read_to_string(s.repo.join(".worktrees/feat/y/.git")).unwrap();
    std::fs::write(s.repo.join(".worktrees/feat/x/.git"), other).unwrap();

    let mut params = create_params(&project, "x", "");
    params.sandbox = Some(tier);
    params.branch = Some("feat/x".into());
    params.on_collision = Some(CollisionResolution::AdoptWorktree);
    let session = client.call::<SessionCreate>(&params).await.unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Failed).await;
    let reason = failure_reason(&mut client, &session.id).await;
    assert!(
        reason.contains("not a link to one of the repo's worktrees"),
        "{reason}"
    );
}

async fn failure_reason(client: &mut forge_api::client::Client, id: &SessionId) -> String {
    all_events(client, id)
        .await
        .iter()
        .find_map(|e| match &e.payload {
            EventPayload::SessionState(b) if b.to == SessionState::Failed => b.reason.clone(),
            _ => None,
        })
        .unwrap_or_default()
}

/// Without bubblewrap the Standard tier is unavailable: `server.hello` leaves it out and
/// asking for it says why and how to fix it.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn standard_without_bubblewrap_is_unavailable_with_a_fix() {
    sandbox_bins();
    let s = setup("standard");
    // A PATH with git and nothing else.
    let bin = s.home.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let git_path = which("git");
    std::os::unix::fs::symlink(&git_path, bin.join("git")).unwrap();
    let daemon = Daemon::start_with(s.home.path(), |cmd| {
        cmd.env("PATH", &bin);
    });
    let mut client = daemon.client().await;
    assert!(
        !hello_tiers(&mut client)
            .await
            .contains(&SandboxTier::Standard)
    );
    let project = open(&mut client, &s.repo).await;
    let mut params = create_params(&project, "x", "go");
    params.sandbox = Some(SandboxTier::Standard);
    let err = expect_api_error(
        client.call::<SessionCreate>(&params).await,
        ErrorKind::SandboxUnavailable,
    );
    assert!(err.detail().contains("bubblewrap"), "{}", err.detail());
    assert!(err.detail().contains("install"), "{}", err.detail());
}

#[cfg(target_os = "linux")]
fn which(program: &str) -> PathBuf {
    let path = std::env::var_os("PATH").unwrap();
    std::env::split_paths(&path)
        .map(|d| d.join(program))
        .find(|p| p.is_file())
        .unwrap_or_else(|| panic!("{program} not on PATH"))
}

/// Allowlist entries are plain host names; anything else fails `session.create` at once.
#[tokio::test]
async fn network_allowlists_must_be_host_names() {
    let s = setup("unsandboxed");
    let profile = s.repo.join(".forge/agents/fake.toml");
    let text = std::fs::read_to_string(&profile).unwrap();
    std::fs::write(
        &profile,
        text.replace("\"allowed.example\"", "\"https://*.example.com:443\""),
    )
    .unwrap();
    let daemon = Daemon::start(s.home.path());
    let mut client = daemon.client().await;
    let project = open(&mut client, &s.repo).await;
    let params = create_params(&project, "x", "go");
    let err = common::expect_api_error(
        client.call::<SessionCreate>(&params).await,
        forge_api::ErrorKind::ValidationFailed,
    );
    assert!(err.detail().contains("network.allow"), "{}", err.detail());
}
