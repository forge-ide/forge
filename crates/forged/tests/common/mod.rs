//! Starts a real `forged` binary against a temp dir.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use forge_api::client::{Client, ClientError};
use forge_api::{ApiError, ErrorKind};
use tempfile::TempDir;

pub struct Daemon {
    child: Child,
    pub home: PathBuf,
    pub socket: PathBuf,
}

/// A temp dir laid out as a fake `$HOME` with Forge's directories overridden inside it.
/// They are private (0700), as forged requires.
pub fn temp_home() -> TempDir {
    use std::os::unix::fs::DirBuilderExt;
    let dir = tempfile::tempdir().unwrap();
    for sub in ["data", "run", "config"] {
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(dir.path().join(sub))
            .unwrap();
    }
    dir
}

pub fn forged_command(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_forged"));
    cmd.env("HOME", home)
        .env("FORGE_DATA_DIR", home.join("data"))
        .env("FORGE_RUNTIME_DIR", home.join("run"))
        .env("FORGE_CONFIG_DIR", home.join("config"))
        .env("RUST_LOG", "forged=debug")
        .stdin(Stdio::null());
    cmd
}

/// Run a forged that is expected to exit on its own; kill it and panic if it is still running
/// after 10 s.
pub fn expect_exit(home: &Path) -> std::process::Output {
    let mut child = forged_command(home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "forged kept running; stderr: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    child.wait_with_output().unwrap()
}

impl Daemon {
    pub fn start(home: &Path) -> Self {
        let child = forged_command(home).spawn().unwrap();
        let mut daemon = Daemon {
            child,
            home: home.to_path_buf(),
            socket: home.join("run/forged.sock"),
        };
        daemon.wait_ready();
        daemon
    }

    fn wait_ready(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                panic!("forged exited early: {status}");
            }
            if std::os::unix::net::UnixStream::connect(&self.socket).is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("forged did not start listening within 10s");
    }

    pub async fn client(&self) -> Client {
        Client::connect(&self.socket).await.unwrap()
    }

    /// Ask the daemon to shut down cleanly and wait for it.
    pub fn stop(mut self) {
        let status = Command::new("kill")
            .args(["-TERM", &self.child.id().to_string()])
            .status()
            .unwrap();
        assert!(status.success());
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "forged exited with {status}");
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("forged did not stop within 10s");
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A fresh git repo on branch `main` with no commits.
pub fn git_repo(parent: &Path, name: &str) -> PathBuf {
    let path = parent.join(name);
    std::fs::create_dir_all(&path).unwrap();
    let status = Command::new("git")
        .args(["init", "--quiet", "--initial-branch=main"])
        .arg(&path)
        .env("HOME", parent)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success());
    // Tests compare against canonical paths; macOS temp dirs sit behind a symlink.
    path.canonicalize().unwrap()
}

/// Run git in `dir` with a fixed identity and no user or system config; panics on failure.
pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// A git repo on `main` with one commit, so worktrees can branch from it.
pub fn git_repo_with_commit(parent: &Path, name: &str) -> PathBuf {
    let path = git_repo(parent, name);
    std::fs::write(path.join("README.md"), "# demo\n").unwrap();
    git(&path, &["add", "README.md"]);
    git(&path, &["commit", "--quiet", "-m", "initial"]);
    path
}

/// The fake agent binary, built on first use. Cargo only exposes a package's own binaries to
/// its tests, so build it next to `forged` with the same profile.
pub fn fake_agent() -> &'static Path {
    static BIN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    BIN.get_or_init(|| {
        let forged = Path::new(env!("CARGO_BIN_EXE_forged"));
        let mut cmd = Command::new(env!("CARGO"));
        cmd.args(["build", "--quiet", "-p", "forge-fake-agent"]);
        if forged.parent().and_then(|p| p.file_name()) == Some("release".as_ref()) {
            cmd.arg("--release");
        }
        assert!(cmd.status().unwrap().success(), "building forge-fake-agent");
        let bin = forged.with_file_name("forge-fake-agent");
        assert!(bin.exists(), "{} missing after build", bin.display());
        bin
    })
}

/// Write `.forge/agents/<id>.toml` running the fake agent on `script` (YAML text), unsandboxed.
pub fn fake_profile(repo: &Path, id: &str, script: &str) {
    let dir = repo.join(".forge/agents");
    std::fs::create_dir_all(&dir).unwrap();
    let script_path = dir.join(format!("{id}.yaml"));
    std::fs::write(&script_path, script).unwrap();
    let profile = format!(
        "agent = \"fake\"\ncommand = [{:?}, {:?}]\nsandbox = \"unsandboxed\"\nallow_unsandboxed = true\n",
        fake_agent().to_str().unwrap(),
        script_path.to_str().unwrap(),
    );
    std::fs::write(dir.join(format!("{id}.toml")), profile).unwrap();
    // Keep the test's own files out of the repo's status checks.
    let exclude = repo.join(".git/info/exclude");
    if exclude.parent().unwrap().is_dir() {
        let mut text = std::fs::read_to_string(&exclude).unwrap_or_default();
        if !text.contains(".forge/agents/") {
            text.push_str(".forge/agents/\n");
            std::fs::write(&exclude, text).unwrap();
        }
    }
}

impl Daemon {
    /// Kill the daemon without letting it clean up, as a crash would.
    pub fn kill(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn expect_api_error(
    result: Result<impl std::fmt::Debug, ClientError>,
    kind: ErrorKind,
) -> ApiError {
    match result {
        Err(ClientError::Api(err)) if err.kind() == kind => err,
        other => panic!("expected {kind:?}, got {other:?}"),
    }
}
