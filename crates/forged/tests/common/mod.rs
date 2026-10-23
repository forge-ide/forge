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
pub fn temp_home() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    for sub in ["data", "run", "config"] {
        std::fs::create_dir(dir.path().join(sub)).unwrap();
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

pub fn expect_api_error(
    result: Result<impl std::fmt::Debug, ClientError>,
    kind: ErrorKind,
) -> ApiError {
    match result {
        Err(ClientError::Api(err)) if err.kind() == kind => err,
        other => panic!("expected {kind:?}, got {other:?}"),
    }
}
