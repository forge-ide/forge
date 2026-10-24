use std::sync::Mutex;

use forge_api::model::EditKind;

use super::*;

#[derive(Default)]
struct Runs(Mutex<Vec<HookRunBody>>);

impl HookLog for Runs {
    fn ran(&self, run: HookRunBody) -> BoxFuture<'_, ()> {
        self.0.lock().unwrap().push(run);
        Box::pin(async {})
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    runs: Arc<Runs>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("root/.forge/hooks")).unwrap();
        std::fs::create_dir_all(dir.path().join("wt")).unwrap();
        Self {
            dir,
            runs: Arc::default(),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("root")
    }

    /// A stand-in for the user's home.
    fn home(&self) -> PathBuf {
        self.dir.path().join("home")
    }

    /// Write an executable hook script that runs `body` under sh.
    fn script(&self, name: &str, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.root().join(".forge/hooks").join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn hooks(&self, toml: &str) -> ShellHooks {
        let config: HooksConfig = toml::from_str(toml).unwrap();
        assert!(config.validate().is_empty(), "{:?}", config.validate());
        let set = HookSet::compile(&config, &self.root(), &self.home()).unwrap();
        ShellHooks::new(
            Arc::new(set),
            HookContext {
                session: SessionId::new(),
                project_root: self.root(),
                worktree: self.dir.path().join("wt"),
                sandbox: Arc::new(forge_sandbox::Unsandboxed::new()),
                log: self.runs.clone(),
            },
        )
    }

    fn runs(&self) -> Vec<(String, Option<Verdict>, i32)> {
        self.runs
            .0
            .lock()
            .unwrap()
            .iter()
            .map(|r| (r.hook.clone(), r.verdict, r.exit_code))
            .collect()
    }
}

fn exec(argv: &[&str]) -> Request {
    Request::new(Action::Exec {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: ".".into(),
    })
}

fn edit(path: &str) -> Request {
    Request::new(Action::Edit {
        path: path.into(),
        kind: EditKind::Modify,
    })
}

#[tokio::test]
async fn hooks_get_the_spec_input_and_their_answer_is_advice() {
    let fx = Fixture::new();
    fx.script(
        "block-npm.sh",
        r#"cat > "$0.input"; echo '{"verdict":"deny","reason":"Use pnpm","noteForAgent":"Run pnpm add."}'"#,
    );
    let hooks = fx.hooks(
        r#"
[[hook]]
event = "tool.pre"
match = { action = "exec", argv = "npm install*" }
run = [".forge/hooks/block-npm.sh"]
"#,
    );
    assert_eq!(
        hooks.advise(&exec(&["pnpm", "add", "x"]), Mode::Ask).await,
        None
    );
    let advice = hooks
        .advise(
            &exec(&["sh", "-c", "cd x && npm install left-pad"]),
            Mode::Auto,
        )
        .await
        .unwrap();
    assert_eq!(advice.hook, "block-npm.sh");
    assert_eq!(advice.verdict, Some(Verdict::Deny));
    assert_eq!(advice.reason.as_deref(), Some("Use pnpm"));
    assert_eq!(advice.note.as_deref(), Some("Run pnpm add."));

    let input: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fx.root().join(".forge/hooks/block-npm.sh.input")).unwrap(),
    )
    .unwrap();
    assert_eq!(input["event"], "tool.pre");
    assert_eq!(input["mode"], "auto");
    assert_eq!(input["action"]["type"], "exec");
    assert_eq!(input["action"]["argv"][2], "cd x && npm install left-pad");
    assert_eq!(
        input["worktree"],
        fx.dir.path().join("wt").display().to_string()
    );
    assert!(input["sessionId"].as_str().unwrap().starts_with("ses_"));
    assert_eq!(fx.runs(), [("block-npm.sh".into(), Some(Verdict::Deny), 0)]);
}

#[tokio::test]
async fn later_hooks_see_rewrites_and_the_first_deny_stops_the_chain() {
    let fx = Fixture::new();
    fx.script(
        "rewrite.sh",
        r#"echo '{"rewrite":["pnpm","install"],"verdict":"allow"}'"#,
    );
    fx.script(
        "ask.sh",
        r#"echo '{"verdict":"ask","reason":"installs need a human"}'"#,
    );
    fx.script("deny.sh", r#"echo '{"verdict":"deny"}'"#);
    fx.script("never.sh", "touch \"$0.ran\"");
    let hooks = fx.hooks(
        r#"
[[hook]]
event = "tool.pre"
match = { argv = "npm *" }
run = [".forge/hooks/rewrite.sh"]

[[hook]]
event = "tool.pre"
match = { argv = "pnpm *" }
run = [".forge/hooks/ask.sh"]

[[hook]]
event = "tool.pre"
match = { argv = "npm *" }
run = [".forge/hooks/never.sh"]
"#,
    );
    let advice = hooks
        .advise(&exec(&["npm", "install"]), Mode::Ask)
        .await
        .unwrap();
    assert_eq!(advice.rewrite, Some(vec!["pnpm".into(), "install".into()]));
    // The ask hook matched the rewritten command and beat the rewriter's allow; the last hook
    // no longer matched.
    assert_eq!(advice.verdict, Some(Verdict::Ask));
    assert_eq!(advice.hook, "ask.sh");
    assert!(!fx.root().join(".forge/hooks/never.sh.ran").exists());

    let hooks = fx.hooks(
        r#"
[[hook]]
event = "tool.pre"
run = [".forge/hooks/deny.sh"]

[[hook]]
event = "tool.pre"
run = [".forge/hooks/never.sh"]
"#,
    );
    let advice = hooks.advise(&exec(&["ls"]), Mode::Ask).await.unwrap();
    assert_eq!(advice.verdict, Some(Verdict::Deny));
    assert!(!fx.root().join(".forge/hooks/never.sh.ran").exists());
}

#[tokio::test]
async fn failed_pre_hooks_ask_post_hooks_stay_quiet_and_task_done_blocks() {
    let fx = Fixture::new();
    fx.script("fail.sh", r#"echo '{"noteForAgent":"x"}'; exit 3"#);
    fx.script("junk.sh", "echo not json");
    fx.script("slow.sh", "sleep 5");
    fx.script("quiet.sh", "true");
    let config = r#"
[[hook]]
event = "EVENT"
run = [".forge/hooks/fail.sh"]

[[hook]]
event = "EVENT"
run = [".forge/hooks/junk.sh"]

[[hook]]
event = "EVENT"
run = [".forge/hooks/slow.sh"]
timeout_ms = 200

[[hook]]
event = "EVENT"
run = [".forge/hooks/quiet.sh"]
"#;
    let hooks = fx.hooks(&config.replace("EVENT", "tool.pre"));
    let started = Instant::now();
    let advice = hooks
        .advise(&exec(&["ls"]), Mode::FullAccess)
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(3));
    assert_eq!(advice.verdict, Some(Verdict::Ask));
    assert_eq!(advice.hook, "fail.sh");
    assert!(advice.reason.unwrap().contains("failed"));
    assert_eq!(
        fx.runs(),
        [
            ("fail.sh".into(), None, 3),
            ("junk.sh".into(), None, 0),
            ("slow.sh".into(), None, NO_EXIT_CODE),
            ("quiet.sh".into(), None, 0),
        ]
    );

    let hooks = fx.hooks(&config.replace("EVENT", "tool.post"));
    assert_eq!(
        hooks
            .post(&exec(&["ls"]).action, Mode::Ask, "completed")
            .await,
        None
    );

    let hooks = fx.hooks(&config.replace("EVENT", "task.done"));
    match hooks.task_done(Mode::Ask).await {
        TaskCheck::Blocked { hook, reason } => {
            assert_eq!(hook, "fail.sh");
            assert!(reason.contains("exited"), "{reason}");
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn sandboxed_hooks_are_untrusted_so_their_allow_counts_for_nothing() {
    let fx = Fixture::new();
    fx.script("allow.sh", r#"echo '{"verdict":"allow"}'"#);
    let hooks = fx.hooks("[[hook]]\nevent = \"tool.pre\"\nrun = [\".forge/hooks/allow.sh\"]\n");
    let advice = hooks.advise(&exec(&["ls"]), Mode::Ask).await.unwrap();
    assert_eq!(advice.verdict, Some(Verdict::Allow));
    assert!(!advice.trusted);
}

#[tokio::test]
async fn script_arguments_and_siblings_come_from_the_project_root() {
    let fx = Fixture::new();
    fx.script("lib.sh", r#"echo '{"verdict":"deny","reason":"root lib"}'"#);
    fx.script("main.sh", r#". "$FORGE_HOOKS_DIR/lib.sh""#);
    // The worktree's copies, which the agent controls, say otherwise.
    let wt = fx.dir.path().join("wt/.forge/hooks");
    std::fs::create_dir_all(&wt).unwrap();
    for name in ["lib.sh", "main.sh"] {
        std::fs::write(
            wt.join(name),
            "echo '{\"verdict\":\"deny\",\"reason\":\"worktree\"}'\n",
        )
        .unwrap();
    }
    for run in [
        r#"["sh", ".forge/hooks/lib.sh"]"#,
        r#"["sh", "./.forge/hooks/main.sh"]"#,
    ] {
        let hooks = fx.hooks(&format!("[[hook]]\nevent = \"tool.pre\"\nrun = {run}\n"));
        let advice = hooks.advise(&exec(&["ls"]), Mode::Ask).await.unwrap();
        assert_eq!(advice.reason.as_deref(), Some("root lib"), "{run}");
    }
}

#[tokio::test]
async fn too_much_output_fails_at_once_and_big_input_does_not_deadlock() {
    let fx = Fixture::new();
    fx.script("flood.sh", "head -c 3000000 /dev/zero | tr '\\0' x");
    fx.script(
        "chatty.sh",
        r#"head -c 200000 /dev/zero | tr '\0' ' '; cat > /dev/null; echo '{"verdict":"deny"}'"#,
    );
    let hooks = fx.hooks(
        "[[hook]]\nevent = \"tool.pre\"\nrun = [\".forge/hooks/flood.sh\"]\ntimeout_ms = 20000\n",
    );
    let started = Instant::now();
    let advice = hooks.advise(&exec(&["ls"]), Mode::Ask).await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(advice.reason.unwrap().contains("more than"));

    let hooks = fx.hooks(
        "[[hook]]\nevent = \"tool.pre\"\nrun = [\".forge/hooks/chatty.sh\"]\ntimeout_ms = 20000\n",
    );
    let big = "a".repeat(300_000);
    let started = Instant::now();
    let advice = hooks
        .advise(&exec(&["echo", &big]), Mode::Ask)
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(advice.verdict, Some(Verdict::Deny));
}

#[tokio::test]
async fn long_notes_and_reasons_are_cut() {
    let fx = Fixture::new();
    fx.script(
        "long.sh",
        r#"n=$(head -c 10000 /dev/zero | tr '\0' n); echo "{\"verdict\":\"deny\",\"reason\":\"$n\",\"noteForAgent\":\"$n\"}""#,
    );
    let hooks = fx.hooks("[[hook]]\nevent = \"tool.pre\"\nrun = [\".forge/hooks/long.sh\"]\n");
    let advice = hooks.advise(&exec(&["ls"]), Mode::Ask).await.unwrap();
    assert!(advice.reason.unwrap().len() <= TEXT_LIMIT + 10);
    assert!(advice.note.unwrap().ends_with("[cut]"));
}

#[tokio::test]
async fn task_done_blocks_with_the_hooks_reason_and_note() {
    let fx = Fixture::new();
    fx.script("ok.sh", "true");
    fx.script(
        "tests.sh",
        r#"echo '{"verdict":"deny","reason":"2 tests fail.","noteForAgent":"Run cargo test."}'"#,
    );
    let hooks = fx.hooks(
        r#"
[[hook]]
event = "task.done"
run = [".forge/hooks/ok.sh"]
"#,
    );
    assert_eq!(hooks.task_done(Mode::Auto).await, TaskCheck::Done);
    let hooks = fx.hooks(
        r#"
[[hook]]
event = "task.done"
name = "tests"
run = [".forge/hooks/tests.sh"]
"#,
    );
    assert_eq!(
        hooks.task_done(Mode::Auto).await,
        TaskCheck::Blocked {
            hook: "tests".into(),
            reason: "2 tests fail.\n\nRun cargo test.".into(),
        }
    );
}

#[tokio::test]
async fn commit_pre_hooks_see_the_commit_and_block_unless_they_pass() {
    let fx = Fixture::new();
    // Passes only when it sees the commit's message and paths.
    fx.script(
        "check.sh",
        r#"input=$(cat); case "$input" in *'"message":"add f"'*'"paths":["src/f.rs"]'*) ;; *) echo '{"verdict":"deny","reason":"wrong input"}';; esac"#,
    );
    fx.script(
        "tests.sh",
        r#"echo '{"verdict":"deny","reason":"tests fail","noteForAgent":"fix them"}'"#,
    );
    fx.script("asks.sh", r#"echo '{"verdict":"ask","reason":"maybe"}'"#);
    fx.script("broken.sh", "exit 3");
    fx.script("slow.sh", "sleep 5");
    let commit = CommitInput {
        message: "add f".into(),
        paths: vec!["src/f.rs".into()],
    };
    let hooks = |name: &str| {
        fx.hooks(&format!(
            "[[hook]]\nevent = \"commit.pre\"\nname = \"{name}\"\nrun = [\".forge/hooks/{name}.sh\"]\ntimeout_ms = 300\n"
        ))
    };
    assert_eq!(
        hooks("check").commit_pre(Mode::Auto, &commit).await,
        CommitCheck::Passed
    );
    let blocked = |name: &str, reason: &str| CommitCheck::Blocked {
        hook: name.into(),
        reason: reason.into(),
    };
    assert_eq!(
        hooks("tests").commit_pre(Mode::Auto, &commit).await,
        blocked("tests", "tests fail\n\nfix them")
    );
    for name in ["asks", "broken", "slow"] {
        match hooks(name).commit_pre(Mode::Auto, &commit).await {
            CommitCheck::Blocked { hook, reason } => {
                assert_eq!(hook, name);
                assert!(reason.contains(name), "{reason}");
            }
            CommitCheck::Passed => panic!("{name} passed"),
        }
    }
    // No commit.pre hooks: nothing to block.
    let none = fx.hooks("");
    assert_eq!(
        none.commit_pre(Mode::Auto, &commit).await,
        CommitCheck::Passed
    );
}

#[tokio::test]
async fn post_hooks_leave_notes_and_edit_hooks_match_paths() {
    let fx = Fixture::new();
    fx.script(
        "status.sh",
        r#"input=$(cat); case "$input" in *'"status":"failed"'*) echo '{"noteForAgent":"it failed"}';; esac"#,
    );
    fx.script(
        "gen.sh",
        r#"echo '{"verdict":"deny","reason":"generated"}'"#,
    );
    let hooks = fx.hooks(
        r#"
[[hook]]
event = "tool.post"
run = [".forge/hooks/status.sh"]

[[hook]]
event = "edit.pre"
match = { path = "gen/**" }
run = [".forge/hooks/gen.sh"]

[[hook]]
event = "edit.post"
match = { path = "*.rs" }
run = [".forge/hooks/status.sh"]
"#,
    );
    let ls = exec(&["ls"]).action;
    assert_eq!(hooks.post(&ls, Mode::Ask, "completed").await, None);
    assert_eq!(
        hooks.post(&ls, Mode::Ask, "failed").await.as_deref(),
        Some("it failed")
    );
    // Edits run edit hooks, never tool hooks.
    assert_eq!(hooks.advise(&edit("src/a.rs"), Mode::Ask).await, None);
    let advice = hooks
        .advise(&edit("./gen/x/y.rs"), Mode::Ask)
        .await
        .unwrap();
    assert_eq!(advice.verdict, Some(Verdict::Deny));
    assert_eq!(
        hooks
            .post(&edit("src/a.rs").action, Mode::Ask, "failed")
            .await
            .as_deref(),
        Some("it failed")
    );
    assert_eq!(
        hooks.post(&edit("a.txt").action, Mode::Ask, "failed").await,
        None
    );
}

#[test]
fn scripts_come_from_the_project_root_and_its_hooks_dir_may_not_be_a_link() {
    let fx = Fixture::new();
    let config: HooksConfig =
        toml::from_str("[[hook]]\nevent = \"tool.pre\"\nrun = [\"./.forge/hooks/a.sh\", \"x\"]\n")
            .unwrap();
    let set = HookSet::compile(&config, &fx.root(), &fx.home()).unwrap();
    assert_eq!(set.dir(), Some(fx.root().join(".forge/hooks").as_path()));
    assert_eq!(
        set.hooks[0].program,
        fx.root().join(".forge/hooks/a.sh").display().to_string()
    );
    let bare: HooksConfig =
        toml::from_str("[[hook]]\nevent = \"tool.pre\"\nrun = [\"sh\"]\n").unwrap();
    assert_eq!(
        HookSet::compile(&bare, &fx.root(), &fx.home())
            .unwrap()
            .dir(),
        None
    );
    let missing: HooksConfig =
        toml::from_str("[[hook]]\nevent = \"tool.pre\"\nrun = [\"forge-no-such-program\"]\n")
            .unwrap();
    assert!(HookSet::compile(&missing, &fx.root(), &fx.home()).is_err());

    std::fs::remove_dir(fx.root().join(".forge/hooks")).unwrap();
    std::os::unix::fs::symlink(fx.dir.path(), fx.root().join(".forge/hooks")).unwrap();
    assert!(HookSet::compile(&config, &fx.root(), &fx.home()).is_err());
}

#[test]
fn bare_programs_must_already_be_inside_what_the_sandbox_shows() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fx = Fixture::new();
    let home = fx.home();
    let bin = home.join("bin");
    let cargo_bin = home.join(".cargo/bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&cargo_bin).unwrap();
    let exe = |path: &Path| {
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    };
    // A script in the user's ~/bin, which the sandbox hides: naming it must not expose it.
    exe(&bin.join("deploy.sh"));
    // A toolchain program the sandbox shows, and a link there leading back into the home.
    exe(&cargo_bin.join("cargo"));
    std::fs::write(home.join("secret"), "token").unwrap();
    symlink(home.join("secret"), cargo_bin.join("leak")).unwrap();
    // A link to a system program keeps its own name, for programs that look at it.
    let sh = std::fs::canonicalize("/bin/sh").unwrap();
    symlink(&sh, cargo_bin.join("multi")).unwrap();
    let path = std::env::join_paths([&bin, &cargo_bin]).unwrap();
    let compile = |program: &str| {
        let config: HooksConfig = toml::from_str(&format!(
            "[[hook]]\nevent = \"tool.pre\"\nrun = [{program:?}]\n"
        ))
        .unwrap();
        let path = path.to_str().unwrap().to_owned();
        let var = move |name: &str| (name == "PATH").then(|| path.clone());
        HookSet::compile_with(&config, &fx.root(), &home, &var)
    };
    let refused = compile("deploy.sh").unwrap_err();
    assert!(refused.contains("doesn't show"), "{refused}");
    assert!(compile("leak").is_err());
    assert_eq!(
        compile("cargo").unwrap().hooks[0].program,
        cargo_bin.join("cargo").display().to_string()
    );
    let multi = compile("multi").unwrap();
    assert_eq!(
        multi.hooks[0].program,
        cargo_bin.join("multi").display().to_string()
    );
    assert_eq!(multi.dir(), None, "nothing is shown for a bare program");
}
