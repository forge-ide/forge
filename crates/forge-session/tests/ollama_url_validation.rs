//! Startup-level regression for F-058 / M5 (T7 — config injection).
//!
//! The `OLLAMA_BASE_URL` env var controls where LLM traffic and tool-result
//! payloads (including `fs.read` content) are sent. These tests exercise the
//! actual forged binary entry path to make sure the validator's error
//! surfaces out to the user on stderr and aborts startup — a future refactor
//! that accidentally swallowed the error in `main.rs` would slip past a
//! pure-function-only test.

use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;

const FORGED: &str = env!("CARGO_BIN_EXE_forged");

/// Spawn `forged` with a given `OLLAMA_BASE_URL` and return (exit_status,
/// merged_stderr). Uses `--ephemeral` so no pid-file plumbing is required
/// and `--provider ollama:<model>` so the validator is actually reached
/// (the default path stays on MockProvider).
fn run_with_ollama_url(
    base_url: &str,
    allow_remote: Option<&str>,
) -> (std::process::ExitStatus, String) {
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let sock_path = dir.path().join("session.sock");

    let mut cmd = std::process::Command::new(FORGED);
    cmd.arg("--ephemeral")
        .arg("--provider")
        .arg("ollama:nonexistent-model")
        .env("FORGE_SESSION_ID", "ollama-url-validation")
        .env("FORGE_SOCKET_PATH", &sock_path)
        .env("FORGE_WORKSPACE", &workspace)
        .env("OLLAMA_BASE_URL", base_url)
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if let Some(v) = allow_remote {
        cmd.env("FORGE_ALLOW_REMOTE_OLLAMA", v);
    } else {
        cmd.env_remove("FORGE_ALLOW_REMOTE_OLLAMA");
    }

    let mut child = cmd.spawn().expect("spawn forged");

    // The validator fires synchronously on the startup path before the socket
    // is bound, so the process should exit quickly. If it doesn't (because a
    // regression silently accepted the URL), fail the test rather than hang.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().expect("try_wait") {
            let out = child.wait_with_output().expect("wait_with_output");
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            return (status, stderr);
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let out = child.wait_with_output().expect("wait_with_output");
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            panic!("forged did not exit on invalid OLLAMA_BASE_URL={base_url}; stderr={stderr}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn https_url_without_opt_in_fails_startup_with_clear_error() {
    let (status, stderr) = run_with_ollama_url("https://example.com", None);
    assert!(
        !status.success(),
        "forged must reject https OLLAMA_BASE_URL; stderr={stderr}"
    );
    // Error must reach the user and explain what failed. Match on both the
    // scheme word and the policy guidance so a lazy future refactor that
    // demotes the message to "invalid url" doesn't slip through.
    assert!(
        stderr.contains("scheme"),
        "stderr must describe the scheme policy, got: {stderr}"
    );
    assert!(
        stderr.contains("OLLAMA_BASE_URL"),
        "stderr must name the offending env var, got: {stderr}"
    );
}

#[test]
fn remote_http_without_opt_in_fails_startup() {
    let (status, stderr) = run_with_ollama_url("http://attacker.example", None);
    assert!(
        !status.success(),
        "forged must reject non-loopback OLLAMA_BASE_URL; stderr={stderr}"
    );
    assert!(
        stderr.contains("FORGE_ALLOW_REMOTE_OLLAMA"),
        "stderr must name the opt-in env var, got: {stderr}"
    );
}

#[test]
fn loopback_http_logs_resolved_url_to_stderr() {
    // Loopback URL is accepted and the resolved form is surfaced on stderr
    // (F-058 DoD: "Startup log surfaces the resolved URL"). Point at port 1
    // so the daemon gets past validation but doesn't actually need an Ollama
    // instance; the startup log line is emitted before any chat happens.
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let sock_path = dir.path().join("session.sock");

    let mut child = std::process::Command::new(FORGED)
        .arg("--ephemeral")
        .arg("--provider")
        .arg("ollama:nonexistent-model")
        .env("FORGE_SESSION_ID", "ollama-url-log")
        .env("FORGE_SOCKET_PATH", &sock_path)
        .env("FORGE_WORKSPACE", &workspace)
        .env("OLLAMA_BASE_URL", "http://127.0.0.1:1")
        .env_remove("FORGE_ALLOW_REMOTE_OLLAMA")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn forged");

    // Give forged enough time to emit the startup log before we kill it.
    std::thread::sleep(Duration::from_millis(500));
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait_with_output");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        stderr.contains("ollama base_url"),
        "startup log must surface the resolved ollama URL, got: {stderr}"
    );
    assert!(
        stderr.contains("127.0.0.1"),
        "startup log must include the resolved host, got: {stderr}"
    );
}

#[test]
fn remote_http_with_opt_in_is_accepted_and_logged_as_warn() {
    // With `FORGE_ALLOW_REMOTE_OLLAMA=1`, a non-loopback `http` URL is
    // accepted but the startup log must call out the remote endpoint so
    // operators grepping logs see why traffic is leaving the box.
    let dir = TempDir::new().unwrap();
    let workspace = dir.path().join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let sock_path = dir.path().join("session.sock");

    let mut child = std::process::Command::new(FORGED)
        .arg("--ephemeral")
        .arg("--provider")
        .arg("ollama:nonexistent-model")
        .env("FORGE_SESSION_ID", "ollama-url-remote")
        .env("FORGE_SOCKET_PATH", &sock_path)
        .env("FORGE_WORKSPACE", &workspace)
        // Loopback port so reqwest can't actually dial anything — the log
        // line fires before any network I/O.
        .env("OLLAMA_BASE_URL", "http://198.51.100.1:1")
        .env("FORGE_ALLOW_REMOTE_OLLAMA", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn forged");

    std::thread::sleep(Duration::from_millis(500));
    let _ = child.kill();
    let out = child.wait_with_output().expect("wait_with_output");
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();

    assert!(
        stderr.contains("WARN"),
        "remote-accepted startup log must be WARN-flavored, got: {stderr}"
    );
    assert!(
        stderr.contains("FORGE_ALLOW_REMOTE_OLLAMA"),
        "WARN log must name the opt-in env var, got: {stderr}"
    );
}
