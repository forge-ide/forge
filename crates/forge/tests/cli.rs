//! Runs the `forge` binary against an in-process daemon in a temp dir.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

use forge_api::model::Project;
use forge_config::Paths;
use tempfile::TempDir;
use tokio::sync::oneshot;

struct Env {
    home: TempDir,
    paths: Paths,
    stop: Option<oneshot::Sender<()>>,
    daemon: Option<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

impl Env {
    fn new() -> Self {
        let home = tempfile::tempdir().unwrap();
        let paths = Paths {
            data_dir: home.path().join("data"),
            runtime_dir: home.path().join("run"),
            config_dir: home.path().join("config"),
            managed_policy: home.path().join("managed.toml"),
        };
        Self {
            home,
            paths,
            stop: None,
            daemon: None,
        }
    }

    async fn start_daemon(&mut self) {
        let (tx, rx) = oneshot::channel();
        let paths = self.paths.clone();
        self.daemon = Some(tokio::spawn(forged::run(paths, async {
            let _ = rx.await;
        })));
        self.stop = Some(tx);
        let deadline = Instant::now() + Duration::from_secs(10);
        while std::os::unix::net::UnixStream::connect(self.paths.socket()).is_err() {
            assert!(Instant::now() < deadline, "daemon did not start");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn stop_daemon(&mut self) {
        let _ = self.stop.take().unwrap().send(());
        self.daemon.take().unwrap().await.unwrap().unwrap();
    }

    /// Run `forge` with `args` in `cwd`; the process is blocking, so run it off the runtime.
    async fn forge(&self, cwd: &Path, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_forge"));
        cmd.args(args)
            .current_dir(cwd)
            .env("HOME", self.home.path())
            .env("FORGE_DATA_DIR", &self.paths.data_dir)
            .env("FORGE_RUNTIME_DIR", &self.paths.runtime_dir)
            .env("FORGE_CONFIG_DIR", &self.paths.config_dir)
            .env("FORGE_MANAGED_POLICY", &self.paths.managed_policy);
        tokio::task::spawn_blocking(move || cmd.output().unwrap())
            .await
            .unwrap()
    }

    fn git_repo(&self, name: &str) -> PathBuf {
        let path = self.home.path().join(name);
        let status = Command::new("git")
            .args(["init", "--quiet", "--initial-branch=main"])
            .arg(&path)
            .env("HOME", self.home.path())
            .status()
            .unwrap();
        assert!(status.success());
        path.canonicalize().unwrap()
    }
}

fn stdout_json<T: serde::de::DeserializeOwned>(output: &Output) -> T {
    assert!(
        output.status.success(),
        "forge failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn project_open_dot_returns_the_project() {
    let mut env = Env::new();
    env.start_daemon().await;
    let repo = env.git_repo("demo");

    let project: Project = stdout_json(&env.forge(&repo, &["project", "open", "."]).await);
    assert_eq!(project.name, "demo");
    assert_eq!(project.root, repo.to_str().unwrap());
    assert!(repo.join("forge.toml").exists());

    let listed: Vec<Project> = stdout_json(&env.forge(&repo, &["project", "list"]).await);
    assert_eq!(listed, vec![project.clone()]);
    let got: Project = stdout_json(
        &env.forge(&repo, &["project", "get", project.id.as_str()])
            .await,
    );
    assert_eq!(got, project);

    env.stop_daemon().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn errors_go_to_stderr_with_failure_status() {
    let mut env = Env::new();
    env.start_daemon().await;
    let plain = env.home.path().join("plain");
    std::fs::create_dir(&plain).unwrap();

    let output = env.forge(&plain, &["project", "open"]).await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ValidationFailed"), "{stderr}");
    assert!(output.stdout.is_empty());

    let output = env.forge(&plain, &["project", "get", "not-an-id"]).await;
    assert!(!output.status.success());

    env.stop_daemon().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn says_how_to_start_the_daemon_when_it_is_not_running() {
    let env = Env::new();
    let output = env.forge(env.home.path(), &["status"]).await;
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("forged"), "{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn status_prints_hello() {
    let mut env = Env::new();
    env.start_daemon().await;
    let hello: serde_json::Value = stdout_json(&env.forge(env.home.path(), &["status"]).await);
    assert_eq!(hello["apiVersion"], forge_api::API_VERSION);
    env.stop_daemon().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn lists_agents_and_approvals_and_checks_wire_names() {
    let mut env = Env::new();
    env.start_daemon().await;
    let agents: Vec<forge_api::model::AgentProfile> =
        stdout_json(&env.forge(env.home.path(), &["agent", "list"]).await);
    let ids: Vec<_> = agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["antigravity", "claude", "codex", "gemini"]);
    assert!(agents.iter().all(|a| a.supported_version.is_some()));

    let pending: Vec<serde_json::Value> =
        stdout_json(&env.forge(env.home.path(), &["approval", "list"]).await);
    assert!(pending.is_empty());

    // Modes and decisions take their wire names, and anything else fails before connecting.
    let project = forge_api::ProjectId::new().to_string();
    let output = env
        .forge(
            env.home.path(),
            &[
                "session",
                "create",
                "--project",
                &project,
                "--agent",
                "claude",
                "--title",
                "t",
                "--mode",
                "yolo",
            ],
        )
        .await;
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("yolo"));
    let output = env
        .forge(env.home.path(), &["approval", "decide", "apr_x", "maybe"])
        .await;
    assert!(!output.status.success());
    env.stop_daemon().await;
}
