//! Terminals an agent asks for: one process group each, with bounded captured output.

use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use forge_sandbox::SessionSandbox;
use futures_util::future::BoxFuture;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::watch;

/// Output kept when the agent doesn't set a limit.
pub const DEFAULT_OUTPUT_LIMIT: usize = 1024 * 1024;

/// How long to wait for output after the command exits before killing what it left running.
const DRAIN_GRACE: Duration = Duration::from_secs(2);

/// How a terminal's command ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<String>,
}

impl ExitInfo {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Longest unfinished line kept apart for one pipe before it joins the rest as it is.
const PENDING_LIMIT: usize = 64 * 1024;

/// Which of a command's output pipes a chunk came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pipe {
    Stdout,
    Stderr,
}

/// Captured output, trimmed from the front to stay under a byte limit. Each pipe's unfinished
/// last line is kept apart until it ends, so the other pipe's lines never land inside it: a
/// token on stdout stays whole whatever stderr prints meanwhile. Nothing is held from
/// readers, who see the unfinished lines after the finished ones.
#[derive(Debug, Default)]
struct Output {
    merged: String,
    pending: [String; 2],
    truncated: bool,
    limit: usize,
}

impl Output {
    fn push(&mut self, pipe: Pipe, chunk: &str) {
        let pending = &mut self.pending[pipe as usize];
        pending.push_str(chunk);
        if let Some(end) = pending.rfind(['\n', '\r']) {
            self.merged.push_str(&pending[..=end]);
            pending.drain(..=end);
        }
        if pending.len() > PENDING_LIMIT {
            self.merged.push_str(&std::mem::take(pending));
        }
        self.trim();
    }

    /// The pipes' unfinished lines join the rest: the command is done.
    fn finish(&mut self) {
        let unfinished = self.unfinished();
        self.merged.push_str(&unfinished);
        self.pending = Default::default();
    }

    fn text(&self) -> String {
        self.merged.clone() + &self.unfinished()
    }

    /// Both pipes' unfinished lines, on lines of their own: one pipe's text glued to the
    /// other's could hide the end of a secret (`AKIA…` then `warn`).
    fn unfinished(&self) -> String {
        let parts: Vec<&str> = self
            .pending
            .iter()
            .map(String::as_str)
            .filter(|p| !p.is_empty())
            .collect();
        parts.join("\n")
    }

    fn trim(&mut self) {
        let pending: usize = self.pending.iter().map(String::len).sum();
        let total = self.merged.len() + pending;
        if total <= self.limit {
            return;
        }
        let mut cut = (total - self.limit).min(self.merged.len());
        while !self.merged.is_char_boundary(cut) {
            cut += 1;
        }
        self.merged.drain(..cut);
        self.truncated = true;
        // A limit smaller than the unfinished lines themselves.
        for pending in &mut self.pending {
            if pending.len() > self.limit {
                let mut cut = pending.len() - self.limit;
                while !pending.is_char_boundary(cut) {
                    cut += 1;
                }
                pending.drain(..cut);
            }
        }
    }
}

struct Terminal {
    pid: Option<u32>,
    output: Mutex<Output>,
    exit: watch::Receiver<Option<ExitInfo>>,
}

/// Called with each chunk of output as it arrives, and the pipe it came from.
pub type OnOutput = Arc<dyn Fn(Pipe, String) -> BoxFuture<'static, ()> + Send + Sync>;
/// Called once the command has exited and all its output was delivered, before waiters wake.
/// What it returns is added to the end of the output.
pub type OnExit = Box<dyn FnOnce(ExitInfo, String) -> BoxFuture<'static, Option<String>> + Send>;

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("starting {program}: {source}")]
    Spawn {
        program: String,
        source: std::io::Error,
    },
    #[error("no terminal {0}")]
    Unknown(String),
}

/// The terminals of one agent connection, all run in its sandbox.
pub struct Terminals {
    sandbox: Arc<dyn SessionSandbox>,
    map: Mutex<HashMap<String, Arc<Terminal>>>,
}

impl Terminals {
    pub fn new(sandbox: Arc<dyn SessionSandbox>) -> Self {
        Self {
            sandbox,
            map: Mutex::default(),
        }
    }

    /// Start `program args` in `cwd` as a new process group. Returns its terminal ID.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        &self,
        id: String,
        program: &str,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        limit: usize,
        on_output: OnOutput,
        on_exit: OnExit,
    ) -> Result<String, TerminalError> {
        let mut cmd = self.sandbox.command(program, args, cwd, env);
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let mut child = cmd.spawn().map_err(|source| TerminalError::Spawn {
            program: program.to_owned(),
            source,
        })?;
        let (exit_tx, exit_rx) = watch::channel(None);
        let terminal = Arc::new(Terminal {
            pid: child.id(),
            output: Mutex::new(Output {
                limit,
                ..Output::default()
            }),
            exit: exit_rx,
        });
        let readers = [
            child.stdout.take().map(|s| {
                (
                    Pipe::Stdout,
                    Box::new(s) as Box<dyn AsyncRead + Send + Unpin>,
                )
            }),
            child.stderr.take().map(|s| {
                (
                    Pipe::Stderr,
                    Box::new(s) as Box<dyn AsyncRead + Send + Unpin>,
                )
            }),
        ]
        .into_iter()
        .flatten()
        .map(|(pipe, stream)| {
            tokio::spawn(read_into(pipe, stream, terminal.clone(), on_output.clone()))
        })
        .collect::<Vec<_>>();

        let watcher = terminal.clone();
        tokio::spawn(async move {
            let status = child.wait().await;
            let mut drained = futures_util::future::join_all(readers);
            if tokio::time::timeout(DRAIN_GRACE, &mut drained)
                .await
                .is_err()
            {
                // Something the command started still holds its output open.
                kill_group(watcher.pid);
                drained.await;
            }
            let info = match status {
                Ok(status) => exit_info(status),
                Err(e) => {
                    tracing::warn!(error = %e, "waiting for terminal command");
                    ExitInfo {
                        code: None,
                        signal: None,
                    }
                }
            };
            let tail = {
                let mut output = lock(&watcher.output);
                output.finish();
                output.text()
            };
            if let Some(extra) = on_exit(info.clone(), tail).await {
                let mut output = lock(&watcher.output);
                output.push(Pipe::Stdout, &extra);
                output.finish();
            }
            let _ = exit_tx.send(Some(info));
        });
        lock(&self.map).insert(id.clone(), terminal);
        Ok(id)
    }

    /// Output so far, whether it was truncated, and the exit status if the command ended.
    pub fn output(&self, id: &str) -> Result<(String, bool, Option<ExitInfo>), TerminalError> {
        let terminal = self.get(id)?;
        let output = lock(&terminal.output);
        let exit = terminal.exit.borrow().clone();
        Ok((output.text(), output.truncated, exit))
    }

    /// Wait for the command to exit.
    pub async fn wait(&self, id: &str) -> Result<ExitInfo, TerminalError> {
        let mut exit = self.get(id)?.exit.clone();
        let info = exit
            .wait_for(Option::is_some)
            .await
            .map(|info| info.clone())
            .ok()
            .flatten();
        Ok(info.unwrap_or(ExitInfo {
            code: None,
            signal: None,
        }))
    }

    /// Kill the command and everything it started; the terminal stays readable.
    pub fn kill(&self, id: &str) -> Result<(), TerminalError> {
        let terminal = self.get(id)?;
        if terminal.exit.borrow().is_none() {
            kill_group(terminal.pid);
        }
        Ok(())
    }

    /// Kill the command if it is still running and forget the terminal.
    pub fn release(&self, id: &str) -> Result<(), TerminalError> {
        self.kill(id)?;
        lock(&self.map).remove(id);
        Ok(())
    }

    /// Kill every running command.
    pub fn kill_all(&self) {
        let terminals: Vec<_> = lock(&self.map).drain().map(|(_, t)| t).collect();
        for terminal in terminals {
            if terminal.exit.borrow().is_none() {
                kill_group(terminal.pid);
            }
        }
    }

    fn get(&self, id: &str) -> Result<Arc<Terminal>, TerminalError> {
        lock(&self.map)
            .get(id)
            .cloned()
            .ok_or_else(|| TerminalError::Unknown(id.to_owned()))
    }
}

async fn read_into(
    pipe: Pipe,
    mut stream: Box<dyn AsyncRead + Send + Unpin>,
    terminal: Arc<Terminal>,
    on_output: OnOutput,
) {
    let mut buf = vec![0u8; 8192];
    let mut carry = Vec::new();
    loop {
        let n = match stream.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let chunk = decode(&mut carry, &buf[..n]);
        if chunk.is_empty() {
            continue;
        }
        lock(&terminal.output).push(pipe, &chunk);
        on_output(pipe, chunk).await;
    }
    if !carry.is_empty() {
        let rest = String::from_utf8_lossy(&carry).into_owned();
        lock(&terminal.output).push(pipe, &rest);
        on_output(pipe, rest).await;
    }
}

/// Decode `bytes` after any partial character left from the previous read, keeping a partial
/// character at the end for next time.
fn decode(carry: &mut Vec<u8>, bytes: &[u8]) -> String {
    carry.extend_from_slice(bytes);
    match std::str::from_utf8(carry) {
        Ok(s) => {
            let s = s.to_owned();
            carry.clear();
            s
        }
        Err(e) if e.error_len().is_none() => {
            let valid = e.valid_up_to();
            let s = String::from_utf8_lossy(&carry[..valid]).into_owned();
            carry.drain(..valid);
            s
        }
        Err(_) => {
            let s = String::from_utf8_lossy(carry).into_owned();
            carry.clear();
            s
        }
    }
}

fn kill_group(pid: Option<u32>) {
    let Some(pid) = pid
        .and_then(|p| i32::try_from(p).ok())
        .and_then(rustix::process::Pid::from_raw)
    else {
        return;
    };
    if let Err(e) = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL) {
        tracing::debug!(error = %e, "killing terminal process group");
    }
}

fn exit_info(status: std::process::ExitStatus) -> ExitInfo {
    use std::os::unix::process::ExitStatusExt;
    ExitInfo {
        code: status.code(),
        signal: status.signal().map(signal_name),
    }
}

fn signal_name(signal: i32) -> String {
    match signal {
        1 => "SIGHUP".into(),
        2 => "SIGINT".into(),
        9 => "SIGKILL".into(),
        15 => "SIGTERM".into(),
        n => format!("signal {n}"),
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pipes_unfinished_line_is_never_split_by_the_other() {
        let mut out = Output {
            limit: 1024,
            ..Output::default()
        };
        out.push(Pipe::Stdout, "token: ghp_a1B2");
        out.push(Pipe::Stderr, "   Compiling foo\n");
        // Readers see everything at once, the unfinished line last.
        assert_eq!(out.text(), "   Compiling foo\ntoken: ghp_a1B2");
        out.push(Pipe::Stdout, "c3D4\ndone\n");
        out.push(Pipe::Stderr, "warn");
        assert_eq!(
            out.text(),
            "   Compiling foo\ntoken: ghp_a1B2c3D4\ndone\nwarn"
        );
        out.push(Pipe::Stdout, "AKIA0123456789ABCDEF");
        // Two unfinished lines are kept apart, so neither runs into the other.
        let both = "   Compiling foo\ntoken: ghp_a1B2c3D4\ndone\nAKIA0123456789ABCDEF\nwarn";
        assert_eq!(out.text(), both);
        out.finish();
        assert_eq!(out.text(), both);
        let mut small = Output {
            limit: 8,
            ..Output::default()
        };
        small.push(Pipe::Stdout, "0123456789\nabcdefghijkl");
        assert!(
            small.text().len() <= 8 && small.truncated,
            "{:?}",
            small.text()
        );
    }

    fn noop_output() -> OnOutput {
        Arc::new(|_, _| Box::pin(async {}))
    }

    #[tokio::test]
    async fn captures_output_and_exit_and_reports_before_waking() {
        let terminals = Terminals::new(Arc::new(forge_sandbox::Unsandboxed::new()));
        let chunks = Arc::new(Mutex::new(String::new()));
        let sink = chunks.clone();
        let exited = Arc::new(Mutex::new(None));
        let record = exited.clone();
        let dir = tempfile::tempdir().unwrap();
        let id = terminals
            .spawn(
                "t1".into(),
                "sh",
                &["-c".into(), "printf out; printf err >&2; exit 3".into()],
                dir.path(),
                &[],
                DEFAULT_OUTPUT_LIMIT,
                Arc::new(move |_, c| {
                    sink.lock().unwrap().push_str(&c);
                    Box::pin(async {})
                }),
                Box::new(move |info, tail| {
                    *record.lock().unwrap() = Some((info, tail));
                    Box::pin(async { Some("+note".to_owned()) })
                }),
            )
            .unwrap();
        let info = terminals.wait(&id).await.unwrap();
        assert_eq!(info.code, Some(3));
        let (output, truncated, exit) = terminals.output(&id).unwrap();
        // The pipes' unfinished lines on lines of their own, then what the exit callback
        // returns, before waiters wake.
        assert!(
            output == "out\nerr+note" || output == "err\nout+note",
            "{output:?}"
        );
        assert!(output.contains("out") && output.contains("err"));
        assert!(output.ends_with("+note"));
        assert!(!truncated);
        assert_eq!(exit, Some(info.clone()));
        assert_eq!(chunks.lock().unwrap().len(), 6);
        assert_eq!(exited.lock().unwrap().clone().unwrap().0, info);
        terminals.release(&id).unwrap();
        assert!(matches!(
            terminals.output(&id),
            Err(TerminalError::Unknown(_))
        ));
    }

    #[tokio::test]
    async fn kill_stops_the_whole_group() {
        let terminals = Terminals::new(Arc::new(forge_sandbox::Unsandboxed::new()));
        let dir = tempfile::tempdir().unwrap();
        let id = terminals
            .spawn(
                "t".into(),
                "sh",
                &["-c".into(), "sleep 30 & sleep 30".into()],
                dir.path(),
                &[],
                DEFAULT_OUTPUT_LIMIT,
                noop_output(),
                Box::new(|_, _| Box::pin(async { None })),
            )
            .unwrap();
        terminals.kill(&id).unwrap();
        let info = tokio::time::timeout(Duration::from_secs(10), terminals.wait(&id))
            .await
            .expect("killed command exits")
            .unwrap();
        assert_eq!(info.signal.as_deref(), Some("SIGKILL"));
    }

    #[tokio::test]
    async fn spawn_failures_are_errors() {
        let terminals = Terminals::new(Arc::new(forge_sandbox::Unsandboxed::new()));
        let dir = tempfile::tempdir().unwrap();
        let result = terminals.spawn(
            "t".into(),
            "/no/such/program",
            &[],
            dir.path(),
            &[],
            10,
            noop_output(),
            Box::new(|_, _| Box::pin(async { None })),
        );
        assert!(matches!(result, Err(TerminalError::Spawn { .. })));
    }

    #[test]
    fn output_is_trimmed_from_the_front_on_char_boundaries() {
        let mut out = Output {
            limit: 5,
            ..Output::default()
        };
        out.push(Pipe::Stdout, "abc");
        out.push(Pipe::Stdout, "déf");
        assert!(out.truncated);
        assert!(out.text().len() <= 5);
        assert!(out.text().ends_with("éf"));
    }

    #[test]
    fn decoding_keeps_split_characters() {
        let bytes = "é".as_bytes();
        let mut carry = Vec::new();
        assert_eq!(decode(&mut carry, &bytes[..1]), "");
        assert_eq!(decode(&mut carry, &bytes[1..]), "é");
        assert!(carry.is_empty());
    }
}
