//! The evaluation order: which step decides when several could, and what hooks, the
//! classifier, and remembered decisions may and may not override.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use forge_api::model::{Action, EditKind, Mode};
use forge_config::PolicyConfig;
use forge_policy::{
    Classifier, ClassifierError, ClassifyRequest, Engine, HookAdvice, Layer, NoClassifier, NoHooks,
    Policy, PreToolHooks, Request, RuleList, Rules, SecretScanner, Source, Verdict,
};
use futures_util::future::BoxFuture;

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

fn rules(project: &str) -> Arc<Rules> {
    let config: PolicyConfig = toml::from_str(project).unwrap();
    Arc::new(Rules::compile(&[(Layer::Project, &config)]).unwrap())
}

fn engine(project: &str, hooks: Arc<dyn PreToolHooks>, classifier: Arc<dyn Classifier>) -> Engine {
    Engine::new(
        rules(project),
        Arc::new(SecretScanner::default()),
        hooks,
        classifier,
    )
}

fn plain(project: &str) -> Engine {
    engine(project, Arc::new(NoHooks), Arc::new(NoClassifier))
}

/// Hooks that give fixed advice and count how often they're asked.
struct FixedHooks {
    advice: Option<HookAdvice>,
    calls: AtomicUsize,
    seen: Mutex<Vec<Action>>,
}

impl FixedHooks {
    fn new(advice: HookAdvice) -> Arc<Self> {
        Arc::new(Self {
            advice: Some(advice),
            calls: AtomicUsize::new(0),
            seen: Mutex::default(),
        })
    }
}

impl PreToolHooks for FixedHooks {
    fn advise<'a>(&'a self, request: &'a Request, _: Mode) -> BoxFuture<'a, Option<HookAdvice>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().unwrap().push(request.action.clone());
        let advice = self.advice.clone();
        Box::pin(async move { advice })
    }
}

fn hook(verdict: Option<Verdict>) -> HookAdvice {
    HookAdvice {
        hook: "check".into(),
        verdict,
        reason: verdict.map(|_| "hook says so".into()),
        rewrite: None,
        rewritten_by: None,
        note: Some("a note".into()),
        trusted: true,
    }
}

struct FixedClassifier {
    answer: Result<Option<(Verdict, String)>, ClassifierError>,
    calls: AtomicUsize,
}

impl FixedClassifier {
    fn new(answer: Result<Option<(Verdict, String)>, ClassifierError>) -> Arc<Self> {
        Arc::new(Self {
            answer,
            calls: AtomicUsize::new(0),
        })
    }
}

impl Classifier for FixedClassifier {
    fn classify<'a>(
        &'a self,
        _: ClassifyRequest<'a>,
    ) -> BoxFuture<'a, Result<Option<(Verdict, String)>, ClassifierError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let answer = self.answer.clone();
        Box::pin(async move { answer })
    }
}

#[tokio::test]
async fn git_rule_denies_in_every_mode_with_a_pointer_to_git_commit() {
    let e = plain("[[allow]]\nexec = [\"git *\"]\n");
    for mode in [Mode::Ask, Mode::AcceptEdits, Mode::Auto, Mode::FullAccess] {
        let d = e
            .decide(&exec(&["git", "commit", "-m", "x"]), mode)
            .await
            .decision;
        assert_eq!(d.verdict, Verdict::Deny, "{mode:?}");
        assert_eq!(d.source, Source::GitSharedState);
        assert!(d.reason.contains("git_commit"), "{}", d.reason);
    }
    // Read-only forms fall through to the allow rule.
    for argv in [
        &["git", "branch"][..],
        &["git", "branch", "--list"],
        &["git", "stash", "list"],
    ] {
        let d = e.decide(&exec(argv), Mode::Ask).await.decision;
        assert_eq!(d.verdict, Verdict::Allow, "{argv:?}");
    }
}

#[tokio::test]
async fn hard_denies_and_guards_run_before_hooks() {
    let hooks = FixedHooks::new(hook(Some(Verdict::Allow)));
    let e = engine(
        "[[deny]]\nexec = [\"rm -rf /*\"]\n",
        hooks.clone(),
        Arc::new(NoClassifier),
    );
    for (request, source) in [
        (exec(&["rm", "-rf", "/"]), "project:deny[0]"),
        (edit(".env"), "guard:protected-path"),
        (edit("../x"), "guard:outside-worktree"),
        (edit("sub/.git/config"), "guard:outside-worktree"),
        (exec(&["git", "push"]), "builtin:git-shared-state"),
    ] {
        let d = e.decide(&request, Mode::FullAccess).await.decision;
        assert_eq!(d.verdict, Verdict::Deny, "{request:?}");
        assert_eq!(d.source.to_string(), source);
    }
    assert_eq!(hooks.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn secrets_ask_even_where_rules_and_mode_allow() {
    let e = plain("[[allow]]\nedit = [\"src/**\"]\n");
    let secret = format!("key = \"AKIA{}\"", "Q".repeat(16));
    let d = e
        .decide(&edit("src/a.rs").with_content(secret), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Ask);
    assert_eq!(d.source, Source::SecretScan);
    assert!(d.reason.contains("aws-access-key"));
    assert!(
        !d.reason.contains("AKIA"),
        "the reason must not echo the secret"
    );
    assert!(!d.source.rememberable());
}

#[tokio::test]
async fn hook_verdicts_decide_before_allow_rules_and_mode() {
    let allow_all = "[[allow]]\nexec = [\"npm *\"]\n";
    let e = engine(
        allow_all,
        FixedHooks::new(hook(Some(Verdict::Deny))),
        Arc::new(NoClassifier),
    );
    let o = e.decide(&exec(&["npm", "install"]), Mode::FullAccess).await;
    assert_eq!(o.decision.verdict, Verdict::Deny);
    assert_eq!(o.decision.source.to_string(), "hook:check");
    assert_eq!(o.decision.reason, "hook says so");
    assert_eq!(o.note.as_deref(), Some("a note"));

    let e = engine(
        "",
        FixedHooks::new(hook(Some(Verdict::Allow))),
        Arc::new(NoClassifier),
    );
    let d = e
        .decide(&exec(&["npm", "install"]), Mode::Ask)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Allow);

    let e = engine(
        "",
        FixedHooks::new(hook(Some(Verdict::Ask))),
        Arc::new(NoClassifier),
    );
    let d = e
        .decide(&exec(&["npm", "install"]), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Ask);
    assert!(!d.source.rememberable());
}

#[tokio::test]
async fn an_untrusted_hook_can_tighten_but_its_allow_is_no_opinion() {
    let untrusted = |verdict| HookAdvice {
        trusted: false,
        ..hook(Some(verdict))
    };
    let e = engine(
        "",
        FixedHooks::new(untrusted(Verdict::Allow)),
        Arc::new(NoClassifier),
    );
    let o = e.decide(&exec(&["npm", "install"]), Mode::Ask).await;
    assert_eq!(o.decision.verdict, Verdict::Ask);
    assert_eq!(o.decision.source.to_string(), "mode:ask");
    assert_eq!(o.note.as_deref(), Some("a note"));
    let e = engine(
        "",
        FixedHooks::new(untrusted(Verdict::Deny)),
        Arc::new(NoClassifier),
    );
    let d = e
        .decide(&exec(&["npm", "install"]), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Deny);
    assert_eq!(d.source.to_string(), "hook:check");
}

#[tokio::test]
async fn a_hook_allow_does_not_override_an_ask_rule() {
    let e = engine(
        "[[ask]]\nexec = [\"npm *\"]\n",
        FixedHooks::new(hook(Some(Verdict::Allow))),
        Arc::new(NoClassifier),
    );
    let d = e
        .decide(&exec(&["npm", "install"]), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Ask);
    assert_eq!(d.source.to_string(), "project:ask[0]");
}

#[tokio::test]
async fn a_hook_without_a_verdict_leaves_the_decision_to_rules() {
    let e = engine(
        "[[allow]]\nexec = [\"npm *\"]\n",
        FixedHooks::new(hook(None)),
        Arc::new(NoClassifier),
    );
    let o = e.decide(&exec(&["npm", "install"]), Mode::Ask).await;
    assert_eq!(o.decision.source.to_string(), "project:allow[0]");
    assert_eq!(o.note.as_deref(), Some("a note"));
}

#[tokio::test]
async fn rewrites_go_through_denies_and_guards_again() {
    let rewrite = |argv: &[&str]| HookAdvice {
        hook: "fix".into(),
        verdict: None,
        reason: None,
        rewrite: Some(argv.iter().map(|s| s.to_string()).collect()),
        rewritten_by: None,
        note: None,
        trusted: false,
    };
    let e = engine(
        "[[allow]]\nexec = [\"pnpm *\"]\n",
        FixedHooks::new(rewrite(&["pnpm", "add", "x"])),
        Arc::new(NoClassifier),
    );
    let o = e.decide(&exec(&["npm", "install", "x"]), Mode::Ask).await;
    assert_eq!(o.decision.verdict, Verdict::Allow);
    assert_eq!(
        o.action,
        Action::Exec {
            argv: vec!["pnpm".into(), "add".into(), "x".into()],
            cwd: ".".into()
        }
    );

    let e = engine(
        "",
        FixedHooks::new(rewrite(&["git", "push"])),
        Arc::new(NoClassifier),
    );
    let d = e.decide(&exec(&["ls"]), Mode::FullAccess).await.decision;
    assert_eq!(d.verdict, Verdict::Deny);
    assert_eq!(d.source, Source::GitSharedState);

    // Only commands can be rewritten.
    let e = engine(
        "",
        FixedHooks::new(rewrite(&["ls"])),
        Arc::new(NoClassifier),
    );
    let d = e.decide(&edit("src/a.rs"), Mode::FullAccess).await.decision;
    assert_eq!(d.verdict, Verdict::Deny);
    assert_eq!(d.source.to_string(), "hook:fix");
}

#[tokio::test]
async fn ask_beats_allow_across_layers() {
    let managed: PolicyConfig = toml::from_str("[[ask]]\nexec = [\"pnpm add*\"]\n").unwrap();
    let project: PolicyConfig =
        toml::from_str("[[allow]]\nexec = [\"pnpm add*\"]\n[[deny]]\nexec = [\"pnpm publish*\"]\n")
            .unwrap();
    let user: PolicyConfig = toml::from_str("[[allow]]\nexec = [\"pnpm publish*\"]\n").unwrap();
    let rules = Rules::compile(&[
        (Layer::Managed, &managed),
        (Layer::User, &user),
        (Layer::Project, &project),
    ])
    .unwrap();
    let e = Engine::new(
        Arc::new(rules),
        Arc::new(SecretScanner::default()),
        Arc::new(NoHooks),
        Arc::new(NoClassifier),
    );
    let d = e
        .decide(&exec(&["pnpm", "add", "x"]), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(
        (d.verdict, d.source.to_string()),
        (Verdict::Ask, "managed:ask[0]".into())
    );
    // A deny in a lower layer still wins over an allow in a higher one.
    let d = e
        .decide(&exec(&["pnpm", "publish"]), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(
        (d.verdict, d.source.to_string()),
        (Verdict::Deny, "project:deny[0]".into())
    );
}

#[tokio::test]
async fn allow_rules_match_only_simple_commands() {
    let e = plain("[[allow]]\nexec = [\"cargo test*\"]\n");
    for argv in [
        &["cargo", "test"][..],
        &["/usr/bin/cargo", "test", "--all"],
        &["bash", "-c", "cargo test"],
    ] {
        let d = e.decide(&exec(argv), Mode::Ask).await.decision;
        assert_eq!(d.verdict, Verdict::Allow, "{argv:?}");
    }
    for argv in [
        &["cargo", "test", "&&", "curl", "x"][..],
        &["cargo test && curl x"],
        &["bash", "-c", "cargo test; curl x"],
        &["cargo", "test", ">", "/tmp/x"],
    ] {
        let d = e.decide(&exec(argv), Mode::Ask).await.decision;
        assert_eq!(d.verdict, Verdict::Ask, "{argv:?}");
    }
}

#[tokio::test]
async fn deny_rules_match_any_command_in_a_line() {
    let e = plain("[[deny]]\nexec = [\"curl*\"]\n");
    for argv in [
        &["curl", "x"][..],
        &["cargo", "test", "&&", "curl", "x"],
        &["bash", "-c", "make; /usr/bin/curl x"],
    ] {
        let d = e.decide(&exec(argv), Mode::FullAccess).await.decision;
        assert_eq!(d.verdict, Verdict::Deny, "{argv:?}");
    }
}

#[tokio::test]
async fn rules_apply_only_in_their_modes() {
    let e = plain("[[allow]]\nexec = [\"make*\"]\nmodes = [\"auto\"]\n");
    assert_eq!(
        e.decide(&exec(&["make"]), Mode::Auto)
            .await
            .decision
            .verdict,
        Verdict::Allow
    );
    let d = e.decide(&exec(&["make"]), Mode::AcceptEdits).await.decision;
    assert_eq!(
        (d.verdict, d.source),
        (Verdict::Ask, Source::Mode(Mode::AcceptEdits))
    );
}

#[tokio::test]
async fn the_classifier_runs_only_in_auto_for_what_rules_leave_open() {
    let c = FixedClassifier::new(Ok(Some((Verdict::Deny, "risky".into()))));
    let e = engine(
        "[[allow]]\nexec = [\"ls*\"]\n",
        Arc::new(NoHooks),
        c.clone(),
    );
    let d = e.decide(&exec(&["make"]), Mode::Auto).await.decision;
    assert_eq!((d.verdict, d.source), (Verdict::Deny, Source::Classifier));
    assert_eq!(
        e.decide(&exec(&["ls"]), Mode::Auto).await.decision.verdict,
        Verdict::Allow
    );
    assert_eq!(
        e.decide(&exec(&["make"]), Mode::Ask).await.decision.verdict,
        Verdict::Ask
    );
    assert_eq!(c.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_failing_classifier_means_ask_never_allow() {
    let c = FixedClassifier::new(Err(ClassifierError("timed out".into())));
    let e = engine("", Arc::new(NoHooks), c);
    let d = e.decide(&exec(&["make"]), Mode::Auto).await.decision;
    assert_eq!((d.verdict, d.source), (Verdict::Ask, Source::Classifier));
    assert!(d.reason.contains("timed out"));
}

#[tokio::test]
async fn remembered_decisions_are_exact_and_never_beat_asks_or_guards() {
    let e = plain("[[ask]]\nexec = [\"npm publish*\"]\n");
    let install = exec(&["npm", "install", "x"]);
    e.remember(&install.action, Verdict::Allow);
    let d = e.decide(&install, Mode::Ask).await.decision;
    assert_eq!(
        (d.verdict, d.source),
        (Verdict::Allow, Source::Remembered(RuleList::Allow))
    );
    // Only that exact command.
    let d = e
        .decide(&exec(&["npm", "install", "y"]), Mode::Ask)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Ask);

    let publish = exec(&["npm", "publish"]);
    e.remember(&publish.action, Verdict::Allow);
    assert_eq!(
        e.decide(&publish, Mode::Ask).await.decision.verdict,
        Verdict::Ask
    );

    let env = edit(".env");
    e.remember(&env.action, Verdict::Allow);
    assert_eq!(
        e.decide(&env, Mode::Ask).await.decision.source,
        Source::ProtectedPath
    );

    // A remembered deny holds even in Full access, and replaces an earlier allow.
    e.remember(&install.action, Verdict::Deny);
    let d = e.decide(&install, Mode::FullAccess).await.decision;
    assert_eq!(
        (d.verdict, d.source),
        (Verdict::Deny, Source::Remembered(RuleList::Deny))
    );
    // Edits are remembered by normalized path.
    let a = edit("src/a.rs");
    e.remember(&a.action, Verdict::Deny);
    let d = e
        .decide(&edit("./src/a.rs"), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Deny);
}

#[tokio::test]
async fn network_actions_are_left_to_the_allowlist() {
    let e = plain("");
    let request = Request::new(Action::Network {
        host: "example.com".into(),
        port: 443,
    });
    let d = e.decide(&request, Mode::FullAccess).await.decision;
    assert_eq!(
        (d.verdict, d.source),
        (Verdict::Deny, Source::NetworkAllowlist)
    );
}

#[test]
fn sources_print_as_event_sources_and_only_mode_asks_are_rememberable() {
    let cases = [
        (
            Source::Rule {
                layer: Layer::Managed,
                list: RuleList::Deny,
                index: 2,
            },
            "managed:deny[2]",
            false,
        ),
        (Source::Remembered(RuleList::Allow), "session:allow", false),
        (Source::GitSharedState, "builtin:git-shared-state", false),
        (Source::ProtectedPath, "guard:protected-path", false),
        (Source::SecretScan, "guard:secret-scan", false),
        (Source::Hook("lint".into()), "hook:lint", false),
        (Source::Classifier, "classifier", true),
        (Source::Mode(Mode::AcceptEdits), "mode:accept-edits", true),
    ];
    for (source, text, rememberable) in cases {
        assert_eq!(source.to_string(), text);
        assert_eq!(source.rememberable(), rememberable, "{text}");
    }
}

#[test]
fn bad_patterns_fail_compilation_naming_the_layer_and_rule() {
    let config: PolicyConfig = toml::from_str("[[ask]]\nedit = [\"src/[a\"]\n").unwrap();
    let err = Rules::compile(&[(Layer::User, &config)]).unwrap_err();
    assert_eq!(err.layer, Layer::User);
    assert!(err.message.starts_with("ask[0]"), "{}", err.message);

    let config: PolicyConfig = toml::from_str("[[deny]]\nreason = \"x\"\n").unwrap();
    assert!(Rules::compile(&[(Layer::Project, &config)]).is_err());

    let config: PolicyConfig = toml::from_str("[protected]\npaths = [\"!x\"]\n").unwrap();
    assert!(Rules::compile(&[(Layer::Project, &config)]).is_err());
}

fn bash(script: &str) -> Request {
    exec(&["bash", "-c", script])
}

#[tokio::test]
async fn commands_forge_cannot_read_ask_even_in_full_access() {
    let e = plain("[[allow]]\nexec = [\"*\"]\n");
    for script in [
        "$GIT push",
        "${X:-git push}",
        "eval \"$CMD\"",
        "$(cat cmd)",
        "alias g=git; g push",
        "echo 'unclosed",
    ] {
        let d = e.decide(&bash(script), Mode::FullAccess).await.decision;
        assert_eq!(d.verdict, Verdict::Ask, "{script}");
        assert_eq!(d.source, Source::OpaqueCommand, "{script}");
        assert!(!d.source.rememberable());
    }
    // Expansions in arguments don't make a command unreadable, but an allow rule never
    // matches them, since an expansion can pick an option that runs code.
    let d = e
        .decide(&bash("cargo test $FLAGS"), Mode::Ask)
        .await
        .decision;
    assert_eq!(
        (d.verdict, d.source),
        (Verdict::Ask, Source::Mode(Mode::Ask))
    );
    let d = e
        .decide(&bash("cargo test $FLAGS"), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Allow);
}

#[tokio::test]
async fn an_expansion_that_could_complete_a_deny_rule_asks() {
    let e = plain("[[deny]]\nexec = [\"rm -rf /*\"]\n[[allow]]\nexec = [\"rm *\"]\n");
    let d = e
        .decide(&bash("rm -rf $DIR"), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Ask);
    assert_eq!(d.source.to_string(), "project:deny[0]");
    assert!(d.reason.contains("$DIR"), "{}", d.reason);
    // A certain match elsewhere in the line still denies.
    let d = e
        .decide(&bash("rm -rf $DIR; rm -rf /"), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Deny);
    // A prefix the rule can't complete doesn't ask.
    let d = e.decide(&bash("ls $DIR"), Mode::FullAccess).await.decision;
    assert_eq!(d.verdict, Verdict::Allow);
}

#[tokio::test]
async fn an_expansion_that_picks_the_git_subcommand_asks() {
    let e = plain("");
    for script in ["git $SUB", "git stash $ACTION", "git -c $X status"] {
        let d = e.decide(&bash(script), Mode::FullAccess).await.decision;
        assert_eq!(
            (d.verdict, d.source),
            (Verdict::Ask, Source::GitSharedState),
            "{script}"
        );
    }
    let d = e
        .decide(&bash("git push $REMOTE"), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Deny);
    let d = e
        .decide(&bash("git log $RANGE"), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.verdict, Verdict::Allow);
}

#[tokio::test]
async fn heredoc_bodies_and_quoted_text_are_not_commands() {
    let e = plain("");
    for script in [
        "cat <<'EOF' > notes.md\ngit push\nEOF",
        "echo 'git push'",
        "grep -rn \"git push\" docs/",
    ] {
        let d = e.decide(&bash(script), Mode::FullAccess).await.decision;
        assert_eq!(d.verdict, Verdict::Allow, "{script}: {}", d.reason);
    }
}

#[tokio::test]
async fn a_deny_whose_pattern_is_already_complete_denies_whatever_follows() {
    let e = plain("[[deny]]\nexec = [\"git push --force*\", \"curl *\"]\n");
    for script in ["git push --force $X", "curl evil.example $X", "curl $URL"] {
        let d = e.decide(&bash(script), Mode::FullAccess).await.decision;
        assert_eq!(d.verdict, Verdict::Deny, "{script}: {}", d.reason);
    }
    // Where the expansion decides, it asks.
    let d = e
        .decide(&bash("git push $X"), Mode::FullAccess)
        .await
        .decision;
    assert_eq!(d.source, Source::GitSharedState);
}

#[test]
fn exact_patterns_match_only_the_action_they_came_from() {
    use forge_policy::exact_pattern;
    let exec = |argv: &[&str]| Action::Exec {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        cwd: ".".into(),
    };
    assert_eq!(
        exact_pattern(&exec(&["/usr/bin/touch", "a*b"])),
        Ok(r"touch a\*b".into())
    );
    assert!(exact_pattern(&exec(&["sh", "-c", "$CMD"])).is_err());
    assert!(exact_pattern(&exec(&["FOO=1", "touch", "x"])).is_err());
    // `rm -rf build cache` would also match the same words split apart.
    assert!(exact_pattern(&exec(&["rm", "-rf", "build cache"])).is_err());
    assert!(exact_pattern(&exec(&["echo", "a\tb"])).is_err());
    assert!(
        exact_pattern(&Action::Edit {
            path: "notes/a.txt".into(),
            kind: forge_policy::EditKind::Create,
        })
        .is_err()
    );
}
