//! Policy golden tests: every action in `golden/actions.yaml`, in every mode, under every rule
//! set in `golden/rulesets/`, with the verdict and the source that decided. One snapshot per
//! rule set; review changes with `cargo insta review` and explain each in the PR.

use std::path::Path;
use std::sync::Arc;

use forge_api::model::{Action, EditKind, Mode};
use forge_config::PolicyConfig;
use forge_policy::{
    Engine, Layer, NoClassifier, NoHooks, Policy, Request, Rules, SecretScanner, Verdict,
    mode_name, verdict_name,
};
use serde::Deserialize;

const MODES: [Mode; 4] = [Mode::Ask, Mode::AcceptEdits, Mode::Auto, Mode::FullAccess];

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum Fixture {
    Edit {
        kind: EditKind,
        path: String,
        #[serde(default)]
        secret: Option<String>,
    },
    Exec {
        argv: Vec<String>,
        /// Replaces the word `SECRET` in `argv`.
        #[serde(default)]
        secret: Option<String>,
    },
    Mcp {
        server: String,
        tool: String,
        args: serde_json::Value,
    },
    Git {
        op: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RuleSet {
    managed: Option<PolicyConfig>,
    user: Option<PolicyConfig>,
    project: Option<PolicyConfig>,
    #[serde(default)]
    session: Vec<Remembered>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Remembered {
    verdict: String,
    action: Action,
}

/// A fake secret of `kind`, assembled here so no credential-shaped text is checked in.
fn fake_secret(kind: &str) -> String {
    let (prefix, body, n) = match kind {
        "aws-access-key" => ("AKIA", 'Q', 16),
        "anthropic-key" => ("sk-ant-api03-", 'x', 40),
        "private-key" => ("-----BEGIN RSA PRIVATE KEY-----\n", 'A', 64),
        other => panic!("no fake secret for {other}"),
    };
    format!("{prefix}{}", body.to_string().repeat(n))
}

impl Fixture {
    fn request(&self) -> Request {
        match self {
            Fixture::Edit { kind, path, secret } => {
                let request = Request::new(Action::Edit {
                    path: path.clone(),
                    kind: *kind,
                });
                match secret {
                    Some(kind) => {
                        request.with_content(format!("let k = \"{}\";\n", fake_secret(kind)))
                    }
                    None => request.with_content("fn ok() {}\n"),
                }
            }
            Fixture::Exec { argv, secret } => {
                let argv = argv
                    .iter()
                    .map(|a| match secret {
                        Some(kind) => a.replace("SECRET", &fake_secret(kind)),
                        None => a.clone(),
                    })
                    .collect();
                Request::new(Action::Exec {
                    argv,
                    cwd: ".".into(),
                })
            }
            Fixture::Mcp { server, tool, args } => Request::new(Action::Mcp {
                server: server.clone(),
                tool: tool.clone(),
                args: args.clone(),
            }),
            Fixture::Git { op } => Request::new(Action::Git { op: op.clone() }),
        }
    }

    fn label(&self) -> String {
        match self {
            Fixture::Edit { kind, path, secret } => {
                let kind = format!("{kind:?}").to_lowercase();
                match secret {
                    Some(s) => format!("edit {kind} {path} (content: {s})"),
                    None => format!("edit {kind} {path}"),
                }
            }
            Fixture::Exec { argv, secret } => {
                let words: Vec<String> = argv
                    .iter()
                    .map(|a| {
                        if a.contains(' ') || a.is_empty() {
                            format!("{a:?}")
                        } else {
                            a.clone()
                        }
                    })
                    .collect();
                match secret {
                    Some(s) => format!("exec {} (SECRET: {s})", words.join(" ")),
                    None => format!("exec {}", words.join(" ")),
                }
            }
            Fixture::Mcp { server, tool, .. } => format!("mcp {server}.{tool}"),
            Fixture::Git { op } => format!("git-op {op}"),
        }
    }
}

fn engine(set: &RuleSet) -> Engine {
    let mut layers = Vec::new();
    for (layer, config) in [
        (Layer::Managed, &set.managed),
        (Layer::User, &set.user),
        (Layer::Project, &set.project),
    ] {
        if let Some(config) = config {
            layers.push((layer, config));
        }
    }
    let rules = Rules::compile(&layers).expect("rule set compiles");
    let engine = Engine::new(
        Arc::new(rules),
        Arc::new(SecretScanner::default()),
        Arc::new(NoHooks),
        Arc::new(NoClassifier),
    );
    for r in &set.session {
        let verdict = match r.verdict.as_str() {
            "allow" => Verdict::Allow,
            "deny" => Verdict::Deny,
            other => panic!("remembered verdict {other:?}"),
        };
        engine.remember(&r.action, verdict);
    }
    engine
}

fn golden_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

#[tokio::test]
async fn policy_decisions_match_the_golden_tables() {
    let dir = golden_dir();
    let actions: Vec<Fixture> =
        serde_norway::from_str(&std::fs::read_to_string(dir.join("actions.yaml")).unwrap())
            .unwrap();
    let mut sets: Vec<_> = std::fs::read_dir(dir.join("rulesets"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    sets.sort();
    assert!(sets.len() >= 3, "rule sets: {sets:?}");
    let mut decisions = 0;
    for path in sets {
        let name = path.file_stem().unwrap().to_string_lossy().into_owned();
        let set: RuleSet = toml::from_str(&std::fs::read_to_string(&path).unwrap())
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let engine = engine(&set);
        let mut out = String::new();
        for action in &actions {
            out.push_str(&action.label());
            out.push('\n');
            let request = action.request();
            for mode in MODES {
                let outcome = engine.decide(&request, mode).await;
                decisions += 1;
                out.push_str(&format!(
                    "  {:<13}{:<6}{}\n",
                    mode_name(mode),
                    verdict_name(outcome.decision.verdict),
                    outcome.decision.source
                ));
            }
        }
        insta::assert_snapshot!(format!("golden_{name}"), out);
    }
    assert!(decisions >= 200, "{decisions} decisions");
}
