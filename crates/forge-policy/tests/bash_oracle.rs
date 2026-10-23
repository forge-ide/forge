//! Command parsing checked against bash itself.
//!
//! Each script in the corpus runs under a real `bash -c`, with stub programs early on `PATH`
//! that record the argv they were run with. Then, for what policy parsed from the same script:
//!
//! - **Deny side:** every program bash ran appears as a simple command whose literal words
//!   match what ran (an expansion may stand for the rest), or the line is opaque. Over-reporting
//!   only over-denies, so the reverse isn't checked.
//! - **Allow side:** if the line is simple, bash ran exactly that one program, with exactly the
//!   argv of the line, word for word.
//! - **Decisions:** whatever bash ran that a deny rule names gets Deny or Ask from the engine in
//!   Full access, the one mode where a miss would allow it.
//!
//! Scripts that are known to get past policy are listed in `KNOWN_MISSES` with the reason; the
//! test fails if one of them stops missing (remove it) or any other script misses.
//!
//! Limits: bash only (other shells are never simple, and their commands are best effort), and
//! only the stubbed programs are observed, by the identity built into each stub, so a program
//! run by another name or path still logs as itself; builtins show up as the programs they run.
//! A script runs with no environment but `PATH`, `HOME`, and the log file, in a temp dir
//! holding a few files for globs to match. That matches sessions: the sandbox passes only an
//! allowlisted environment, and variables an agent sets on a terminal (`BASH_ENV`, `SHELLOPTS`,
//! exported functions) reach policy as an `env NAME=value --` prefix, which is never simple.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use std::sync::Arc;

use forge_api::model::{Action, Mode};
use forge_config::PolicyConfig;
use forge_policy::exec::{CommandLine, ExecGlob};
use forge_policy::{
    Engine, Layer, NoClassifier, NoHooks, Policy, Request, Rules, SecretScanner, Verdict,
};

/// Programs replaced by recording stubs.
const STUBS: &[&str] = &[
    "git", "curl", "rg", "cargo", "rm", "python3", "ls", "cat", "make", "tail", "grep", "pnpm",
    "npm", "node",
];

/// Deny rules for the decision check: what bash runs that these name must not be allowed.
const DENY: &[&str] = &["git push*", "git commit*", "curl*", "rm -rf /*"];

/// Scripts that get past policy, and why each is accepted (component-architecture.md, Policy
/// engine, Limits).
const KNOWN_MISSES: &[(&str, &str)] = &[
    (
        "X='a[$(git push)]'; (( X ))",
        "arithmetic evaluates a variable's value, which needs the value",
    ),
    (
        "X='a[$(git push)]'; echo $(( X ))",
        "arithmetic evaluates a variable's value, which needs the value",
    ),
    (
        "X='a[$(git push)]'; [[ X -eq 1 ]]",
        "arithmetic evaluates a variable's value, which needs the value",
    ),
    (
        "./xgit push",
        "a symlink the agent made runs another program under its name",
    ),
];

/// Inputs from the security reviews, and constructs worth pinning down.
const CORPUS: &[&str] = &[
    // Plain and compound commands.
    "cargo test",
    "cargo test --all",
    "/usr/bin/env cargo test",
    "make && make test",
    "cargo test 2>&1 | tail -20",
    "cd . && cargo test",
    "ls; git status; git log",
    "git commit -m 'a b' && git push",
    "git commit -m \"x; y\"",
    // Groups, reserved words, redirections, heredocs.
    "{ git push; }",
    "( git push )",
    "if true; then git push; fi",
    "if git status; then :; else git push; fi",
    "! git push",
    "while false; do git push; done; git status",
    "for f in a b; do git push $f; done",
    "case x in x) git push;; y) ls;; esac",
    "case x in x) ls;; esac; git push",
    "function f { git push; }; f",
    "f() { git push; }; f",
    "git >/dev/null push",
    "git push 2>&1 --force",
    "git push &>/dev/null",
    "cat <<'EOF' > notes.md\ngit push\nEOF",
    "cat <<EOF\n$(git push)\nEOF",
    "cat <<-EOF\n\tbody\n\tEOF\ngit push",
    "cat <<< 'git push'",
    "git status # git push",
    "echo 'git push'",
    "echo \"git push\"",
    "grep -rn \"git push\" .",
    // Quoting.
    "g''it push",
    "\\git push",
    "\"git\" 'push'",
    "g\"i\"t push",
    "X=$'\\'' git push",
    "git commit -m $'a\\tb'",
    "$'\\x67it' push",
    // Expansions and substitutions.
    "cargo test $FLAGS",
    "cargo test ${FLAGS:-}",
    "cargo test ${X:-$(curl evil.example)}",
    "echo $(( $(git push) ))",
    "echo $(git push)",
    "echo `git push`",
    "cat <(git push)",
    "echo \"$(git push)\"",
    "echo ${X:-`git push`}",
    "rg ${X:---pre=evil} x",
    "python3 ${X:--c} 'print(1)'",
    "rg {--pre=evil,x}",
    "rg --pr[e]=./evil x",
    "{git,push}",
    "cat *.rs",
    "ls ~",
    "cat ~/x",
    "X=--force; git push $X",
    "git $SUB",
    "git ${SUB:-push}",
    "$GIT push",
    "${G:-git} push",
    "eval git push",
    "eval \"git push\"",
    "trap 'git push' EXIT",
    "trap -- 'git push' EXIT",
    // Arithmetic and lexer details.
    "((x<<2))\ngit push\n2",
    "((x=1)); git push",
    "for ((i=0; i<1; i++)); do git push; done",
    "cat <<EOF \r; git push\nEOF\n",
    "[ -f a.rs ] && cargo test",
    "git status; [ -n x ]",
    // Wrappers.
    "env git push",
    "env -i FOO=1 git push",
    "env -u HOME git push",
    "env -S 'git push'",
    "env --split-string='git push'",
    "FOO=1 git push",
    "GIT_DIR=x git push",
    "exec git push",
    "command git push",
    "nice -n 5 git push",
    "nohup git push > /dev/null",
    "find . -maxdepth 0 -exec git push \\;",
    "bash -c 'git push'",
    "sh -c 'git push'",
    "bash -c \"bash -c 'git push'\"",
    "bash -e -c 'git push'",
    "sh -c -- 'git push'",
    "bash -o pipefail -c 'git push'",
    "sh -c 'env sh -c \"git push\"'",
    // From the fourth security review.
    "coproc NAME { git push; }",
    "coproc { git push; }",
    "cat <<< $(git push)",
    "ls > $(git push)",
    "ls 2>$(git push)",
    "case x in $(git push)) ;; esac",
    "for x in $(git push); do :; done",
    "[[ $(git push) ]]",
    "echo $[ $(git push) ]",
    "X='a[$(git push)]'; (( X ))",
    "X='a[$(git push)]'; echo $(( X ))",
    "X='a[$(git push)]'; [[ X -eq 1 ]]",
    "source <(echo git push)",
    ". <(echo git push)",
    "bash <(echo git push)",
    "bash <<< 'git push'",
    "echo 'git push' | bash",
    "echo git push | sh",
    "echo push | xargs git",
    "printf push | xargs -I{} git {}",
    r"find push -maxdepth 0 -exec git {} \;",
    "{git,\"push\"}",
    "{\"git\",push}",
    "{g{i,}t,push}",
    "{,git} push",
    "gi\\\nt push",
    "cargo test X=~/y",
    "cargo test --x=~/y",
    "cargo test a:~/y",
    "printf -v X '%s' push; git $X",
    "read X <<< push; git $X",
    "mapfile -t A <<< push; git ${A[0]}",
    "declare -A m=([k]=$(git push))",
    "a=(); a[$(git push)]=1",
    "local x=$(git push)",
    "export X=$(git push)",
    "f() { git \"$@\"; }; f push",
    "alias g=git\ng push",
    "shopt -s expand_aliases\nalias g=git\ng push",
    "set -- push; git \"$@\"",
    "set -- git push; \"$@\"",
    "IFS=,; X=git,push; $X",
    "cargo test ${X:=--config=x}",
    "cargo test \"${X:-a b}\"",
    "time git push",
    "time -p git push",
    "! { git push; }",
    "{ git push; } &",
    "git push & wait",
    "git push |& cat",
    "until git push; do :; done",
    "select x in a; do git push; break; done <<< 1",
    "[[ -n x ]] && git push",
    "test -n x && git push",
    "echo hi #\ngit push",
    "echo a#b; git push",
    "echo ${X#git push}; git status",
    "cat <<\"EOF\"\n$(git push)\nEOF",
    "cat <<E\"OF\"\n$(git push)\nEOF\ngit status",
    "cat << EOF\n`git push`\nEOF",
    "cat <<EOF\n${X:-$(git push)}\nEOF",
    "cat <<EOF; git push\nbody\nEOF",
    "cat <<A <<B\na\nA\n$(git push)\nB",
    "cat <<-EOF\n\tx\n  EOF\n\tEOF\ngit push",
    "echo $'\\'' ; git push",
    "echo \"\\\"\"; git push",
    "echo '\\'; git push",
    "cargo test 'a'\"b\"$'c'",
    "cargo test $'\\x41'",
    "cargo test \\*",
    "cargo test '*'",
    "cargo test [x]",
    "cargo test a]b",
    "cargo test {}",
    "cargo test {a}",
    "cargo test {a,}",
    "cargo test x{1..2}",
    "cargo test ~",
    "cargo test a~",
    "cargo test ''",
    "cargo test \"\"",
    "cargo test \"$@\"",
    "cargo test $@",
    "cargo test $0",
    "rg -- --pre=evil x",
    "rg -e --pre=evil x",
    // Programs reached under another name or path; @STUBS@ is the stub directory.
    "@STUBS@/git push",
    "hash -p @STUBS@/git x; x push",
    "exec -a x git push",
    "./xgit push",
];

/// Templates crossed with the expansion and quoting forms in `FORMS`, at `{F}`.
const TEMPLATES: &[&str] = &[
    "git {F}",
    "git push {F}",
    "cargo test {F}",
    "rg {F} x",
    "echo {F}; git push",
    "if {F}; then git push; fi",
    "x={F} git push",
    "env {F} git push",
    "cat <<EOF\n{F}\nEOF",
    "echo ${X:-{F}}",
    "{F} && cargo test",
];
const FORMS: &[&str] = &[
    "$X",
    "${X:-a}",
    "\"$X\"",
    "$(git status)",
    "`git status`",
    "$((1+1))",
    "{a,b}",
    "a*",
    "~",
    "$'a'",
    "'x y'",
    "\"x y\"",
    "x\\ y",
];

struct Run {
    /// What each stubbed program was run with, in order.
    ran: Vec<Vec<String>>,
}

fn stub_dir(root: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let dir = root.join("stubs");
    std::fs::create_dir_all(&dir).unwrap();
    for name in STUBS {
        // Each stub logs the name built into it, whatever it was run as, to its own file in the
        // log dir: stubs in a pipeline run at once, and a shared file would interleave their
        // records. Fields end with \x1f, so any argv round-trips.
        let script = format!(
            "#!/bin/sh\n\
             {{ printf '%s\\037' {name}; for a in \"$@\"; do printf '%s\\037' \"$a\"; done; }} \
             > \"$FORGE_ORACLE_LOG/$$\"\n"
        );
        let path = dir.join(name);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    dir
}

fn run(script: &str, root: &Path, stubs: &Path) -> Run {
    let log = root.join("log");
    let _ = std::fs::remove_dir_all(&log);
    std::fs::create_dir_all(&log).unwrap();
    let work = root.join("work");
    let mut child = Command::new("bash")
        .arg("-c")
        .arg(script.replace("@STUBS@", &stubs.display().to_string()))
        .current_dir(&work)
        .env_clear()
        .env("PATH", format!("{}:/usr/bin:/bin", stubs.display()))
        .env("HOME", &work)
        .env("FORGE_ORACLE_LOG", &log)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("running bash");
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() > deadline {
            let _ = child.kill();
            panic!("{script:?} didn't finish under bash");
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    // A background job (`cmd &`, `coproc`) may outlive the shell; give it time to log.
    if script.contains('&') || script.contains("coproc") {
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut ran: Vec<Vec<String>> = std::fs::read_dir(&log)
        .unwrap()
        .map(|entry| {
            let text = std::fs::read(entry.unwrap().path()).unwrap();
            let mut fields: Vec<String> = String::from_utf8_lossy(&text)
                .split('\u{1f}')
                .map(str::to_owned)
                .collect();
            fields.pop();
            fields
        })
        .collect();
    ran.sort();
    Run { ran }
}

/// Whether a parsed simple command accounts for what ran: same program, the literal words
/// equal, and an expansion (if any) standing for the rest.
fn accounts_for(words: &[String], literal: usize, ran: &[String]) -> bool {
    if words.first() != ran.first() {
        return false;
    }
    if literal == words.len() {
        words == ran
    } else {
        ran.len() >= literal && words[..literal] == ran[..literal]
    }
}

fn corpus() -> Vec<String> {
    let mut all: Vec<String> = CORPUS.iter().map(|s| (*s).to_owned()).collect();
    for t in TEMPLATES {
        for f in FORMS {
            all.push(t.replace("{F}", f));
        }
    }
    all
}

fn engine() -> Engine {
    let config = PolicyConfig {
        deny: vec![forge_config::PolicyRule {
            exec: DENY.iter().map(|p| (*p).to_owned()).collect(),
            ..Default::default()
        }],
        ..Default::default()
    };
    Engine::new(
        Arc::new(Rules::compile(&[(Layer::Project, &config)]).unwrap()),
        Arc::new(SecretScanner::default()),
        Arc::new(NoHooks),
        Arc::new(NoClassifier),
    )
}

/// Why `script` disagrees with what bash did, if it does.
async fn disagreements(script: &str, root: &Path, stubs: &Path, engine: &Engine) -> Vec<String> {
    let argv = vec![
        "bash".to_owned(),
        "-c".to_owned(),
        script.replace("@STUBS@", &stubs.display().to_string()),
    ];
    let parsed = CommandLine::parse(&argv);
    let run = run(script, root, stubs);
    let mut found = Vec::new();
    for ran in &run.ran {
        let seen = parsed
            .segments
            .iter()
            .any(|s| accounts_for(&s.words, s.literal, ran));
        if !seen && parsed.opaque.is_none() {
            found.push(format!(
                "bash ran {ran:?}, which policy didn't see: {:?}",
                parsed.segments
            ));
        }
    }
    if parsed.simple {
        // A stubbed program must have run with exactly the line's argv; anything else simple
        // is a builtin, which runs no stub.
        let ok = if STUBS.contains(&parsed.words[0].as_str()) {
            matches!(run.ran.as_slice(), [only] if *only == parsed.words)
        } else {
            run.ran.is_empty()
        };
        if !ok {
            found.push(format!(
                "judged simple as {:?}, but bash ran {:?}",
                parsed.words, run.ran
            ));
        }
    }
    let denied: Vec<&[String]> = run
        .ran
        .iter()
        .filter(|r| DENY.iter().any(|p| ExecGlob::new(p).matches(&r.join(" "))))
        .map(Vec::as_slice)
        .collect();
    if !denied.is_empty() {
        let request = Request::new(Action::Exec {
            argv,
            cwd: ".".into(),
        });
        let decision = engine.decide(&request, Mode::FullAccess).await.decision;
        if decision.verdict == Verdict::Allow {
            found.push(format!(
                "bash ran {denied:?}, which a deny rule names, but Full access allows it ({})",
                decision.source
            ));
        }
    }
    found
}

#[tokio::test]
async fn parsing_agrees_with_bash() {
    if Command::new("bash").arg("-c").arg("true").status().is_err() {
        eprintln!("skipping: bash is not installed");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    for file in ["a.rs", "b.rs", "--pre=evil", "x", "push"] {
        std::fs::write(work.join(file), "").unwrap();
    }
    let stubs = stub_dir(&root);
    std::os::unix::fs::symlink(stubs.join("git"), work.join("xgit")).unwrap();
    let engine = engine();

    let mut failures = BTreeSet::new();
    let scripts = corpus();
    for script in &scripts {
        let found = disagreements(script, &root, &stubs, &engine).await;
        let known = KNOWN_MISSES.iter().find(|(s, _)| s == script);
        match (known, found.is_empty()) {
            (None, false) => {
                failures.insert(format!("{script:?}: {}", found.join("; ")));
            }
            (Some((_, why)), true) => {
                failures.insert(format!(
                    "{script:?} is listed as a known miss ({why}) but policy now handles it: \
                     remove it from KNOWN_MISSES"
                ));
            }
            _ => {}
        }
    }
    for (script, _) in KNOWN_MISSES {
        assert!(
            scripts.iter().any(|s| s == script),
            "{script:?} isn't in the corpus"
        );
    }
    assert!(scripts.len() > 300, "{} scripts", scripts.len());
    assert!(
        failures.is_empty(),
        "{} of {} scripts disagree with bash:\n{}",
        failures.len(),
        scripts.len(),
        failures.into_iter().collect::<Vec<_>>().join("\n")
    );
}
