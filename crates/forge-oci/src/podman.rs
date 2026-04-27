//! [`PodmanRuntime`] — first concrete [`crate::ContainerRuntime`] implementation.
//!
//! Shells out to a rootless `podman` binary, structured argv only. No daemon,
//! no `sh -c` invocations, no string concatenation. Every call goes through
//! the [`CommandRunner`] indirection so unit tests can drive the argv-shaping
//! logic without a real binary.

use crate::runner::{CommandOutcome, CommandRunner, TokioCommandRunner};
use crate::{ContainerHandle, ContainerRuntime, ExecResult, ImageRef, OciError, Stats};
use async_trait::async_trait;

/// Default binary name. Resolved via `PATH`.
const PODMAN: &str = "podman";

/// `ContainerRuntime` backed by the rootless `podman` CLI.
pub struct PodmanRuntime {
    runner: Box<dyn CommandRunner>,
}

impl PodmanRuntime {
    /// Build a runtime that shells out via `tokio::process::Command`.
    pub fn new() -> Self {
        Self {
            runner: Box::new(TokioCommandRunner),
        }
    }

    /// Build a runtime backed by a custom [`CommandRunner`] — for tests.
    pub fn with_runner(runner: Box<dyn CommandRunner>) -> Self {
        Self { runner }
    }

    /// Probe the host: confirm `podman --version` works AND `podman info`
    /// reports rootless mode is available.
    ///
    /// Returns:
    /// - `Ok(())` if both probes succeed.
    /// - [`OciError::RuntimeMissing`] if the version probe failed to spawn or
    ///   exited non-zero (treat as "podman not installed").
    /// - [`OciError::RootlessUnavailable`] if `podman info` ran but rootless
    ///   mode is not reported as enabled.
    /// - [`OciError::InvalidJson`] if `podman info` JSON didn't parse.
    pub async fn detect(&self) -> Result<(), OciError> {
        let version = self
            .runner
            .run(PODMAN, &["--version"])
            .await
            .map_err(|_| OciError::RuntimeMissing(PODMAN))?;
        if !version.success() {
            return Err(OciError::RuntimeMissing(PODMAN));
        }

        let info = self
            .runner
            .run(PODMAN, &["info", "--format", "json"])
            .await
            .map_err(|source| OciError::Io {
                tool: PODMAN,
                source,
            })?;
        if !info.success() {
            return Err(OciError::RootlessUnavailable {
                runtime: PODMAN,
                reason: String::from_utf8_lossy(&info.stderr).to_string(),
            });
        }

        let parsed: serde_json::Value =
            serde_json::from_slice(&info.stdout).map_err(|source| OciError::InvalidJson {
                tool: PODMAN,
                subcommand: "info",
                source,
            })?;

        let rootless = parsed
            .get("host")
            .and_then(|h| h.get("security"))
            .and_then(|s| s.get("rootless"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if !rootless {
            return Err(OciError::RootlessUnavailable {
                runtime: PODMAN,
                reason: "podman info reports rootless=false".to_string(),
            });
        }

        Ok(())
    }

    async fn run_or_fail(&self, args: &[&str]) -> Result<CommandOutcome, OciError> {
        let outcome = self
            .runner
            .run(PODMAN, args)
            .await
            .map_err(|source| OciError::Io {
                tool: PODMAN,
                source,
            })?;
        if !outcome.success() {
            return Err(OciError::CommandFailed {
                tool: PODMAN,
                args: args.iter().map(|s| s.to_string()).collect(),
                exit_code: outcome.exit_code,
                stderr: String::from_utf8_lossy(&outcome.stderr).to_string(),
            });
        }
        Ok(outcome)
    }
}

impl Default for PodmanRuntime {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContainerRuntime for PodmanRuntime {
    async fn pull(&self, image: &ImageRef) -> Result<(), OciError> {
        let img = image.to_image_string();
        self.run_or_fail(&["pull", &img]).await?;
        Ok(())
    }

    async fn create(&self, image: &ImageRef, argv: &[String]) -> Result<ContainerHandle, OciError> {
        let img = image.to_image_string();
        // `podman create <image> [argv...]` prints the new container ID on
        // stdout. We use structured args throughout — the trailing argv slice
        // is appended without any quoting / shell interpretation.
        let mut args: Vec<&str> = vec!["create", &img];
        args.extend(argv.iter().map(String::as_str));
        let outcome = self.run_or_fail(&args).await?;
        let id = String::from_utf8_lossy(&outcome.stdout).trim().to_string();
        if id.is_empty() {
            return Err(OciError::CommandFailed {
                tool: PODMAN,
                args: args.iter().map(|s| s.to_string()).collect(),
                exit_code: outcome.exit_code,
                stderr: "podman create returned empty container id".to_string(),
            });
        }
        Ok(ContainerHandle::new(id))
    }

    async fn start(&self, handle: &ContainerHandle) -> Result<(), OciError> {
        self.run_or_fail(&["start", &handle.id]).await?;
        Ok(())
    }

    async fn exec(
        &self,
        handle: &ContainerHandle,
        argv: &[String],
    ) -> Result<ExecResult, OciError> {
        let mut args: Vec<&str> = vec!["exec", &handle.id];
        args.extend(argv.iter().map(String::as_str));
        // exec captures the inner program's stdout/stderr/exit even on a
        // non-zero exit — that's a meaningful signal, not a runtime failure.
        // So we go around `run_or_fail` here.
        let outcome = self
            .runner
            .run(PODMAN, &args)
            .await
            .map_err(|source| OciError::Io {
                tool: PODMAN,
                source,
            })?;
        Ok(ExecResult {
            exit_code: outcome.exit_code,
            stdout: String::from_utf8_lossy(&outcome.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&outcome.stderr).into_owned(),
        })
    }

    async fn stop(&self, handle: &ContainerHandle) -> Result<(), OciError> {
        self.run_or_fail(&["stop", &handle.id]).await?;
        Ok(())
    }

    async fn remove(&self, handle: &ContainerHandle) -> Result<(), OciError> {
        // -f forces removal of running containers (podman stop+rm in one step).
        self.run_or_fail(&["rm", "-f", &handle.id]).await?;
        Ok(())
    }

    async fn stats(&self, handle: &ContainerHandle) -> Result<Stats, OciError> {
        let outcome = self
            .run_or_fail(&["stats", "--no-stream", "--format", "json", &handle.id])
            .await?;
        let parsed: serde_json::Value =
            serde_json::from_slice(&outcome.stdout).map_err(|source| OciError::InvalidJson {
                tool: PODMAN,
                subcommand: "stats",
                source,
            })?;
        let entry = parsed
            .as_array()
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        Ok(Stats {
            cpu_percent: entry
                .get("cpu_percent")
                .and_then(|v| v.as_str())
                .and_then(parse_percent),
            memory_bytes: entry
                .get("mem_usage")
                .and_then(|v| v.as_str())
                .and_then(parse_size_first),
            pids: entry.get("pids").and_then(|v| match v {
                serde_json::Value::String(s) => s.parse().ok(),
                serde_json::Value::Number(n) => n.as_u64(),
                _ => None,
            }),
        })
    }
}

/// Parse a podman percentage string like `"1.35%"` into a float.
fn parse_percent(s: &str) -> Option<f64> {
    s.trim().trim_end_matches('%').trim().parse().ok()
}

/// Parse the *first* size value from a podman `mem_usage` string of the form
/// `"178.3MB / 67.31GB"` into bytes. The second number is the host total,
/// which we don't surface.
fn parse_size_first(s: &str) -> Option<u64> {
    let first = s.split('/').next()?.trim();
    let (num, unit) = split_number_unit(first)?;
    let value: f64 = num.parse().ok()?;
    let multiplier: f64 = match unit.to_ascii_uppercase().as_str() {
        "" | "B" => 1.0,
        "KB" | "K" => 1_000.0,
        "MB" | "M" => 1_000_000.0,
        "GB" | "G" => 1_000_000_000.0,
        "TB" | "T" => 1_000_000_000_000.0,
        "KIB" => 1_024.0,
        "MIB" => 1_024.0 * 1_024.0,
        "GIB" => 1_024.0 * 1_024.0 * 1_024.0,
        "TIB" => 1_024.0 * 1_024.0 * 1_024.0 * 1_024.0,
        _ => return None,
    };
    Some((value * multiplier) as u64)
}

fn split_number_unit(s: &str) -> Option<(&str, &str)> {
    let split = s.find(|c: char| c.is_ascii_alphabetic())?;
    Some((s[..split].trim(), s[split..].trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{RecordingRunner, StubResponse};

    fn rt(runner: RecordingRunner) -> PodmanRuntime {
        PodmanRuntime::with_runner(Box::new(runner))
    }

    #[tokio::test]
    async fn detect_succeeds_when_version_and_rootless_ok() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"podman version 5.0\n".to_vec()));
        runner.push(StubResponse::ok_stdout(
            br#"{"host":{"security":{"rootless":true}}}"#.to_vec(),
        ));
        let calls = runner.calls.clone();

        rt(runner).detect().await.unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].1, vec!["--version"]);
        assert_eq!(calls[1].1, vec!["info", "--format", "json"]);
    }

    #[tokio::test]
    async fn detect_reports_runtime_missing_when_version_fails() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::err(b"not found".to_vec()));

        let err = rt(runner).detect().await.unwrap_err();
        assert!(matches!(err, OciError::RuntimeMissing("podman")));
    }

    #[tokio::test]
    async fn detect_reports_rootless_unavailable_when_info_says_false() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"podman version 5.0\n".to_vec()));
        runner.push(StubResponse::ok_stdout(
            br#"{"host":{"security":{"rootless":false}}}"#.to_vec(),
        ));

        let err = rt(runner).detect().await.unwrap_err();
        assert!(matches!(err, OciError::RootlessUnavailable { .. }));
    }

    #[tokio::test]
    async fn detect_reports_invalid_json() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"podman version 5.0\n".to_vec()));
        runner.push(StubResponse::ok_stdout(b"not json".to_vec()));

        let err = rt(runner).detect().await.unwrap_err();
        assert!(matches!(err, OciError::InvalidJson { .. }));
    }

    #[tokio::test]
    async fn pull_invokes_structured_args() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"".to_vec()));
        let calls = runner.calls.clone();

        let runtime = rt(runner);
        let img = ImageRef::parse("docker.io/library/alpine:3.19").unwrap();
        runtime.pull(&img).await.unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "podman");
        assert_eq!(calls[0].1, vec!["pull", "docker.io/library/alpine:3.19"]);
    }

    #[tokio::test]
    async fn create_returns_handle_from_stdout() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"abc1234deadbeef\n".to_vec()));
        let calls = runner.calls.clone();

        let runtime = rt(runner);
        let img = ImageRef::parse("alpine:3.19").unwrap();
        let h = runtime
            .create(&img, &["echo".into(), "hi".into()])
            .await
            .unwrap();
        assert_eq!(h.id, "abc1234deadbeef");

        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].1, vec!["create", "alpine:3.19", "echo", "hi"]);
    }

    #[tokio::test]
    async fn create_errors_on_empty_id() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"".to_vec()));
        let runtime = rt(runner);
        let img = ImageRef::parse("alpine:3.19").unwrap();
        let err = runtime.create(&img, &[]).await.unwrap_err();
        assert!(matches!(err, OciError::CommandFailed { .. }));
    }

    #[tokio::test]
    async fn start_uses_handle_id() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"".to_vec()));
        let calls = runner.calls.clone();

        let runtime = rt(runner);
        runtime.start(&ContainerHandle::new("xyz")).await.unwrap();

        assert_eq!(calls.lock().unwrap()[0].1, vec!["start", "xyz"]);
    }

    #[tokio::test]
    async fn exec_captures_stdout_and_exit() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse {
            matches_args: None,
            outcome: CommandOutcome {
                exit_code: Some(0),
                stdout: b"hello\n".to_vec(),
                stderr: Vec::new(),
            },
        });
        let calls = runner.calls.clone();

        let runtime = rt(runner);
        let res = runtime
            .exec(
                &ContainerHandle::new("xyz"),
                &["echo".into(), "hello".into()],
            )
            .await
            .unwrap();
        assert_eq!(res.stdout, "hello\n");
        assert_eq!(res.exit_code, Some(0));

        assert_eq!(
            calls.lock().unwrap()[0].1,
            vec!["exec", "xyz", "echo", "hello"]
        );
    }

    #[tokio::test]
    async fn exec_surfaces_nonzero_exit_without_failing() {
        // exec'd command exit codes are signal, not runtime failure.
        let runner = RecordingRunner::new();
        runner.push(StubResponse {
            matches_args: None,
            outcome: CommandOutcome {
                exit_code: Some(2),
                stdout: Vec::new(),
                stderr: b"oops\n".to_vec(),
            },
        });

        let res = rt(runner)
            .exec(&ContainerHandle::new("xyz"), &["false".into()])
            .await
            .unwrap();
        assert_eq!(res.exit_code, Some(2));
        assert_eq!(res.stderr, "oops\n");
    }

    #[tokio::test]
    async fn stop_and_remove_use_force_flag() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"".to_vec()));
        runner.push(StubResponse::ok_stdout(b"".to_vec()));
        let calls = runner.calls.clone();

        let runtime = rt(runner);
        let h = ContainerHandle::new("xyz");
        runtime.stop(&h).await.unwrap();
        runtime.remove(&h).await.unwrap();

        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].1, vec!["stop", "xyz"]);
        assert_eq!(calls[1].1, vec!["rm", "-f", "xyz"]);
    }

    #[tokio::test]
    async fn stats_parses_podman_json() {
        let runner = RecordingRunner::new();
        let json = br#"[
            {"id":"xyz","name":"c","cpu_percent":"1.35%","mem_usage":"178.3MB / 67.31GB","pids":"4"}
        ]"#;
        runner.push(StubResponse::ok_stdout(json.to_vec()));
        let calls = runner.calls.clone();

        let s = rt(runner)
            .stats(&ContainerHandle::new("xyz"))
            .await
            .unwrap();
        assert_eq!(s.cpu_percent, Some(1.35));
        assert_eq!(s.memory_bytes, Some(178_300_000));
        assert_eq!(s.pids, Some(4));

        assert_eq!(
            calls.lock().unwrap()[0].1,
            vec!["stats", "--no-stream", "--format", "json", "xyz"]
        );
    }

    #[tokio::test]
    async fn stats_tolerates_missing_fields() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"[{\"id\":\"xyz\"}]".to_vec()));
        let s = rt(runner)
            .stats(&ContainerHandle::new("xyz"))
            .await
            .unwrap();
        assert_eq!(s.cpu_percent, None);
        assert_eq!(s.memory_bytes, None);
        assert_eq!(s.pids, None);
    }

    #[tokio::test]
    async fn stats_invalid_json_surfaces_typed_error() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::ok_stdout(b"not json".to_vec()));
        let err = rt(runner)
            .stats(&ContainerHandle::new("xyz"))
            .await
            .unwrap_err();
        assert!(matches!(err, OciError::InvalidJson { .. }));
    }

    #[tokio::test]
    async fn command_failure_surfaces_typed_error() {
        let runner = RecordingRunner::new();
        runner.push(StubResponse::err(b"image not found\n".to_vec()));
        let img = ImageRef::parse("does/not:exist").unwrap();
        let err = rt(runner).pull(&img).await.unwrap_err();
        assert!(matches!(
            err,
            OciError::CommandFailed { tool: "podman", .. }
        ));
    }

    #[test]
    fn parse_size_first_handles_bytes() {
        assert_eq!(parse_size_first("178.3MB / 67.31GB"), Some(178_300_000));
        assert_eq!(parse_size_first("2.253MB / 67.31GB"), Some(2_253_000));
        assert_eq!(parse_size_first("512B / 1GB"), Some(512));
        assert_eq!(parse_size_first("1MiB / 1GiB"), Some(1_048_576));
    }
}
