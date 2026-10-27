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
/// key from the real home and to reach a host off the allowlist, through terminals, runs a
/// command a hook denies, tries to change a hook script, writes programs named `git` and
/// `lfsx` into its worktree, and commits with `git_commit`, whose git runs confined in each
/// sandboxed tier and never runs those.
fn script(key: &Path, probe: &Path, hooks: &Path, marks: &Path) -> String {
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
  - exec: ["sh", "-c", "echo key=$ANTHROPIC_API_KEY url=$ANTHROPIC_BASE_URL"]
  - exec: {{ argv: ["rm", "hello.txt"], expect: deny }}
  - exec: ["sh", "-c", "echo x >> {}/no-rm.sh || echo hooks-read-only"]
  - exec: ["sh", "-c", "for p in lfsx git gpg; do printf '#!/bin/sh\\ntouch {}/path-mark-'$p'\\n' > $p && chmod +x $p; done"]
  - mcp_call: {{ tool: git_commit, arguments: {{ message: "add lib", paths: ["src/lib.txt"] }} }}
  - message: "Done."
  - end_turn
"#,
        key.display(),
        probe.display(),
        hooks.display(),
        marks.display()
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
    // A filter the user set in the repo's own config, which the daemon's git runs when it
    // stages `src/`: it leaves a mark in the home, which only an unconfined git can write,
    // and runs `lfsx`, which the agent writes into the worktree and which the daemon's `.` in
    // `PATH` would find there.
    let mark = home.path().join("filter-ran");
    common::git(
        &repo,
        &[
            "config",
            "filter.probe.clean",
            &format!("touch {}; lfsx </dev/null; cat", mark.display()),
        ],
    );
    std::fs::write(repo.join(".gitattributes"), "src/* filter=probe\n").unwrap();
    common::git(&repo, &["add", ".gitattributes"]);
    common::git(&repo, &["commit", "--quiet", "-m", "attributes"]);
    let _ = std::fs::remove_file(&mark);
    let scripts = home.path().join("scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let script_path = scripts.join("fake.yaml");
    let probe = &sandbox_bins()[1];
    let hooks = repo.join(".forge/hooks");
    std::fs::write(
        &script_path,
        script(&key, probe, &hooks, &repo.join(".git/objects")),
    )
    .unwrap();
    write_hooks(&repo);
    std::fs::create_dir_all(repo.join(".forge/agents")).unwrap();
    let opt_in = if tier == "unsandboxed" {
        "allow_unsandboxed = true\n"
    } else {
        ""
    };
    std::fs::write(
        repo.join(".forge/agents/fake.toml"),
        format!(
            "agent = \"fake\"\ncommand = [{:?}, {:?}]\nsandbox = \"{tier}\"\n{opt_in}env_secrets = [\"ANTHROPIC_API_KEY\"]\n[network]\nallow = [\"allowed.example\"]\n",
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

/// A `tool.pre` hook that denies `rm`, and a `tool.post` hook that tries to write to its own
/// directory and leaves a note saying whether it could.
fn write_hooks(repo: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let dir = repo.join(".forge/hooks");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        repo.join(".forge/hooks.toml"),
        "[[hook]]\nevent = \"tool.pre\"\nmatch = { argv = \"rm *\" }\nrun = [\".forge/hooks/no-rm.sh\"]\n\n\
         [[hook]]\nevent = \"tool.post\"\nrun = [\".forge/hooks/post.sh\"]\n",
    )
    .unwrap();
    let scripts = [
        (
            "no-rm.sh",
            r#"echo '{"verdict":"deny","reason":"no rm in this repo"}'"#,
        ),
        (
            "post.sh",
            r#"if touch "$(dirname "$0")/tampered" 2>/dev/null; then w=writable; else w=read-only; fi
echo "{\"noteForAgent\":\"post hook ran; its dir is $w\"}""#,
        ),
    ];
    for (name, body) in scripts {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

/// The value the fake keyring holds for `ANTHROPIC_API_KEY`; it must never reach a sandbox
/// or the event log.
const SECRET: &str = "sk-forge-test-secret";

/// Start a daemon whose keyring tool is a fake that knows only `ANTHROPIC_API_KEY`.
fn start_daemon(s: &Setup) -> Daemon {
    use std::os::unix::fs::PermissionsExt;
    let tool = s.home.path().join("fake-keyring");
    let missing = if cfg!(target_os = "macos") { 44 } else { 1 };
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in\n  *ANTHROPIC_API_KEY*) echo {SECRET} ;;\n  *) exit {missing} ;;\nesac\n"
    );
    std::fs::write(&tool, script).unwrap();
    std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
    Daemon::start_with(s.home.path(), |cmd| {
        // A relative entry first: the daemon's git must never use it.
        let path = format!(".:{}", std::env::var("PATH").unwrap_or_default());
        cmd.env("FORGE_KEYRING_TOOL", &tool)
            .env("PATH", path)
            .env("GIT_AUTHOR_NAME", "Dev")
            .env("GIT_AUTHOR_EMAIL", "dev@example.com")
            .env("GIT_COMMITTER_NAME", "Dev")
            .env("GIT_COMMITTER_EMAIL", "dev@example.com");
    })
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
    let daemon = start_daemon(&s);
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
    // The agent holds a placeholder and a route on the proxy; the real secret is nowhere.
    assert!(output.contains("key=forge-proxy-injected"), "{output:?}");
    let url = output
        .lines()
        .find_map(|l| l.split_once(" url=").map(|(_, url)| url))
        .unwrap_or_default();
    assert!(
        url.contains("/_forge/") && url.ends_with("/anthropic"),
        "{output:?}"
    );
    let logged = serde_json::to_string(&events).unwrap();
    assert!(!logged.contains(SECRET), "the secret reached the event log");
    let denied = denied.expect("the refused connection is in the event log");
    assert_eq!((denied.port, denied.verdict), (443, Verdict::Deny));
    assert!(denied.reason.unwrap_or_default().contains("allowlist"));
    // Hooks run inside the sandbox from the project root, which neither they nor the agent
    // can change.
    let decisions: Vec<String> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::PolicyDecision(b) if b.verdict == Verdict::Deny => Some(b.source.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(decisions, ["hook:no-rm.sh"]);
    let results: String = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::ToolResult(b) => b.output.clone(),
            _ => None,
        })
        .collect();
    assert!(results.contains("post hook ran"), "{results}");
    let hooks = s.repo.join(".forge/hooks");
    if tier != SandboxTier::Unsandboxed {
        assert!(results.contains("its dir is read-only"), "{results}");
        assert!(!results.contains("writable"), "{results}");
        assert!(output.contains("hooks-read-only"), "{output:?}");
        assert!(!hooks.join("tampered").exists());
        assert!(
            !std::fs::read_to_string(hooks.join("no-rm.sh"))
                .unwrap()
                .contains("\nx\n")
        );
    }
    let worktree = s.repo.join(".worktrees/feat/add-greeting");
    assert_eq!(
        std::fs::read_to_string(worktree.join("hello.txt")).unwrap(),
        "hello\n"
    );
    assert_eq!(
        common::git(&s.repo, &["log", "-1", "--format=%s", "feat/add-greeting"]).trim(),
        "add lib"
    );
    // The turn was checkpointed, with the daemon's git confined in the sandboxed tiers.
    assert!(
        events
            .iter()
            .any(|e| matches!(&e.payload, EventPayload::Checkpoint(_))),
        "no checkpoint"
    );
    let errors: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.payload {
            EventPayload::Error(d) => Some(d.detail.clone()),
            _ => None,
        })
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    // The daemon's git never ran the agent's program, whatever the tier.
    for program in ["lfsx", "git"] {
        let mark = s.repo.join(format!(".git/objects/path-mark-{program}"));
        assert!(!mark.exists(), "the daemon ran the agent's {program}");
    }
    // The daemon's git staged it confined in a sandboxed tier; unconfined, for the control.
    assert_eq!(
        s.home.path().join("filter-ran").exists(),
        tier == SandboxTier::Unsandboxed,
        "the daemon's git ran confined only in sandboxed tiers"
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

/// A hook that times out inside Standard leaves nothing running: killing it reaches what it
/// started inside bubblewrap's namespace too.
#[cfg(target_os = "linux")]
#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn a_hook_that_times_out_in_standard_leaves_nothing_running() {
    use std::os::unix::fs::PermissionsExt;
    let s = setup("standard");
    let marker = format!("sleep 8{}", std::process::id());
    std::fs::write(
        s.repo.join(".forge/hooks.toml"),
        "[[hook]]\nevent = \"tool.pre\"\nrun = [\".forge/hooks/slow.sh\"]\ntimeout_ms = 1000\n",
    )
    .unwrap();
    let slow = s.repo.join(".forge/hooks/slow.sh");
    std::fs::write(&slow, format!("#!/bin/sh\n{marker} &\n{marker}\n")).unwrap();
    std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        s.home.path().join("scripts/fake.yaml"),
        "steps:\n  - exec: { argv: [\"true\"], expect: deny }\n  - end_turn\n",
    )
    .unwrap();
    let daemon = start_daemon(&s);
    let mut client = daemon.client().await;
    require_tier(&mut client, SandboxTier::Standard).await;
    let project = open(&mut client, &s.repo).await;
    let mut params = create_params(&project, "x", "go");
    params.sandbox = None;
    let session = client.call::<SessionCreate>(&params).await.unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Waiting).await;
    // The timed-out hook made the command ask; by now it and its children are gone.
    let running = std::fs::read_dir("/proc").unwrap().flatten().any(|p| {
        std::fs::read(p.path().join("cmdline")).is_ok_and(|c| {
            String::from_utf8_lossy(&c)
                .replace('\0', " ")
                .contains(&marker)
        })
    });
    assert!(!running, "the timed-out hook left {marker:?} running");
    let pending = client
        .call::<ApprovalList>(&ApprovalListParams { project_id: None })
        .await
        .unwrap();
    assert!(
        pending[0].reason.contains("timed out"),
        "{}",
        pending[0].reason
    );
}

/// A sandbox that can't start fails the session with the reason, and never runs the agent
/// some weaker way. Here the worktree's `.git` link was rewritten to point at another
/// worktree's git dir, as an agent could do.
#[tokio::test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
async fn a_sandbox_that_cannot_start_fails_the_session() {
    sandbox_bins();
    let s = setup("standard");
    let daemon = start_daemon(&s);
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
    let daemon = start_daemon(&s);
    let mut client = daemon.client().await;
    let project = open(&mut client, &s.repo).await;
    let params = create_params(&project, "x", "go");
    let err = common::expect_api_error(
        client.call::<SessionCreate>(&params).await,
        forge_api::ErrorKind::ValidationFailed,
    );
    assert!(err.detail().contains("network.allow"), "{}", err.detail());
}

/// A secret missing from the keyring fails the session with the command that stores it.
#[tokio::test]
async fn a_missing_secret_fails_the_session_with_a_fix() {
    let s = setup("unsandboxed");
    let profile = s.repo.join(".forge/agents/fake.toml");
    let text = std::fs::read_to_string(&profile).unwrap();
    std::fs::write(
        &profile,
        text.replace("ANTHROPIC_API_KEY", "OPENAI_API_KEY"),
    )
    .unwrap();
    let daemon = start_daemon(&s);
    let mut client = daemon.client().await;
    let project = open(&mut client, &s.repo).await;
    let session = client
        .call::<SessionCreate>(&create_params(&project, "x", ""))
        .await
        .unwrap();
    wait_for_state(&mut client, &session.id, SessionState::Failed).await;
    let reason = failure_reason(&mut client, &session.id).await;
    let fix = if cfg!(target_os = "macos") {
        "security add-generic-password -s forge -a OPENAI_API_KEY -w"
    } else {
        "secret-tool store --label='Forge OPENAI_API_KEY' service forge account OPENAI_API_KEY"
    };
    assert!(reason.contains(fix), "{reason}");
}

/// Only secrets Forge knows how to inject may be named.
#[tokio::test]
async fn unknown_secrets_are_refused() {
    let s = setup("unsandboxed");
    let profile = s.repo.join(".forge/agents/fake.toml");
    let text = std::fs::read_to_string(&profile).unwrap();
    std::fs::write(
        &profile,
        text.replace("ANTHROPIC_API_KEY", "AWS_SECRET_ACCESS_KEY"),
    )
    .unwrap();
    let daemon = start_daemon(&s);
    let mut client = daemon.client().await;
    let project = open(&mut client, &s.repo).await;
    let err = common::expect_api_error(
        client
            .call::<SessionCreate>(&create_params(&project, "x", "go"))
            .await,
        forge_api::ErrorKind::ValidationFailed,
    );
    assert!(
        err.detail().contains("AWS_SECRET_ACCESS_KEY"),
        "{}",
        err.detail()
    );
    assert!(
        err.detail().contains("ANTHROPIC_API_KEY"),
        "{}",
        err.detail()
    );
}
