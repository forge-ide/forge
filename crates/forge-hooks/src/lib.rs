//! Hook runner.
//!
//! Forge runs a project's hooks itself (`.forge/hooks.toml`), so they work for every agent. A
//! hook is a shell command that gets JSON describing the moment on stdin and may answer with
//! JSON on stdout (interfaces-and-data.md, Hook I/O). Hooks run in the session's sandbox, and
//! a script named by a relative path comes from the project root's `.forge/hooks/`, which the
//! sandbox shows read-only, so the agent can't change what its own hooks do.
//!
//! [`ShellHooks`] runs them: `tool.pre` and `edit.pre` through the policy engine's
//! [`PreToolHooks`], `tool.post` and `edit.post` after a command or write, and `task.done`
//! when a turn ends. A hook that fails, times out, or prints something that isn't its JSON
//! answer has no opinion at `tool.post` and `edit.post`, asks at `tool.pre` and `edit.pre`,
//! and blocks at `task.done`. Hooks run where the agent can reach them, so their allow is no
//! opinion: they may deny, ask, or rewrite, never loosen.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use forge_api::SessionId;
use forge_api::events::{HookRunBody, Verdict};
use forge_api::model::{Action, Mode};
use forge_config::{HOOKS_DIR, HookActionType, HookConfig, HookEvent, HooksConfig};
use forge_policy::exec::{CommandLine, ExecGlob};
use forge_policy::paths::{PathPatterns, normalize};
use forge_policy::{HookAdvice, PreToolHooks, Request, mode_name};
use forge_sandbox::SessionSandbox;
use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Default timeout for tool and edit hooks.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// Default timeout for `task.done` hooks, which tend to run checks.
const DEFAULT_TASK_TIMEOUT: Duration = Duration::from_secs(300);
/// Most a hook may print on stdout.
const OUTPUT_LIMIT: usize = 1024 * 1024;
/// The exit code recorded for a hook that couldn't start, timed out, or was killed.
const NO_EXIT_CODE: i32 = -1;
/// Longest note or reason kept from a hook's answer.
const TEXT_LIMIT: usize = 4096;

/// A project's hooks, compiled.
#[derive(Clone, Debug, Default)]
pub struct HookSet {
    hooks: Vec<Hook>,
    /// `<root>/.forge/hooks`, when a hook runs a script from it: the sandbox shows it
    /// read-only.
    dir: Option<PathBuf>,
}

/// The environment variable that names the hooks directory, so scripts can reach their
/// siblings without paths relative to the working directory, which is the agent's worktree.
pub const HOOKS_DIR_VAR: &str = "FORGE_HOOKS_DIR";

#[derive(Clone, Debug)]
struct Hook {
    name: String,
    event: HookEvent,
    action: Option<HookActionType>,
    argv: Option<ExecGlob>,
    path: Option<PathPatterns>,
    program: String,
    /// Whether the hook runs a script from the project's hooks directory.
    script: bool,
    args: Vec<String>,
    timeout: Duration,
}

impl HookSet {
    /// Compile `config`, the hooks file of the project at `root`, finding programs named bare
    /// through the daemon's `PATH`; `home` is the user's. Blocking: checks the hooks directory
    /// and programs.
    pub fn compile(config: &HooksConfig, root: &Path, home: &Path) -> Result<Self, String> {
        Self::compile_with(config, root, home, &|name| std::env::var(name).ok())
    }

    /// [`HookSet::compile`] with the daemon's environment from `var`.
    fn compile_with(
        config: &HooksConfig,
        root: &Path,
        home: &Path,
        var: &dyn Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        let mut set = HookSet::default();
        for (i, hook) in config.hook.iter().enumerate() {
            let hook = compile(hook, root, home, var).map_err(|e| format!("hook[{i}]: {e}"))?;
            set.hooks.push(hook);
        }
        if set.hooks.iter().any(|h| h.script) {
            set.dir = Some(hooks_dir(root)?);
        }
        Ok(set)
    }

    /// The directory of the hooks' scripts, if any hook runs one: the sandbox must show it
    /// read-only. Nothing else is shown for hooks.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }
}

fn compile(
    hook: &HookConfig,
    root: &Path,
    home: &Path,
    var: &dyn Fn(&str) -> Option<String>,
) -> Result<Hook, String> {
    let (program, args) = hook.run.split_first().ok_or("run needs a program")?;
    let relative = program.contains('/') && !program.starts_with('/');
    let program = match forge_config::hooks::hook_script(program) {
        Some(script) if relative => root.join(script).display().to_string(),
        _ if relative => return Err(format!("{program:?} must be a script under {HOOKS_DIR}/")),
        _ if !program.contains('/') => {
            // Found where the agent can't put a program of the same name, and only where the
            // sandbox already looks: a hooks file never widens what the sandbox shows. Run by
            // the path found, so a program that looks at its own name still works.
            let found = find_program(program, var("PATH").as_deref())
                .ok_or_else(|| format!("{program} is not installed (not found in PATH)"))?;
            if !forge_sandbox::shows_read_only(home, &found, var) {
                return Err(format!(
                    "{program} resolves to {}, which the sandbox doesn't show; name a program \
                     under a system directory or put a script under {HOOKS_DIR}/",
                    found.display()
                ));
            }
            found.display().to_string()
        }
        _ => program.clone(),
    };
    // Arguments naming hook scripts (`sh .forge/hooks/check.sh`) come from the project root
    // too, never the worktree the hook runs in.
    let mut script = relative;
    let args = args
        .iter()
        .map(|arg| match forge_config::hooks::hook_script(arg) {
            Some(path) if !arg.starts_with('/') => {
                script = true;
                root.join(path).display().to_string()
            }
            _ => arg.clone(),
        })
        .collect();
    // A default name keeps only characters fit for a decision's source.
    let name = hook.name.clone().unwrap_or_else(|| {
        let file = Path::new(&program)
            .file_name()
            .map_or_else(|| program.clone(), |n| n.to_string_lossy().into_owned());
        file.chars()
            .take(64)
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    });
    let filter = hook.filter.clone().unwrap_or_default();
    let path = filter
        .path
        .map(|p| PathPatterns::new(&[p]).map_err(|e| e.to_string()))
        .transpose()?;
    let default = if hook.event == HookEvent::TaskDone {
        DEFAULT_TASK_TIMEOUT
    } else {
        DEFAULT_TIMEOUT
    };
    Ok(Hook {
        name,
        event: hook.event,
        action: filter.action,
        argv: filter.argv.as_deref().map(ExecGlob::new),
        path,
        program,
        script,
        args,
        timeout: hook.timeout_ms.map_or(default, Duration::from_millis),
    })
}

/// The first `program` in the absolute directories of `path`.
fn find_program(program: &str, path: Option<&str>) -> Option<PathBuf> {
    std::env::split_paths(path?)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

/// `<root>/.forge/hooks`, refusing one that is, or sits under, a symlink: mounting it in the
/// sandbox must not show anything outside the project.
fn hooks_dir(root: &Path) -> Result<PathBuf, String> {
    let mut path = root.to_path_buf();
    for part in HOOKS_DIR.split('/') {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(format!("{} must be a directory", path.display())),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(path)
}

/// Where `hook.run` events go.
pub trait HookLog: Send + Sync + 'static {
    fn ran(&self, run: HookRunBody) -> BoxFuture<'_, ()>;
}

/// Whether the work is done, as the `task.done` hooks see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TaskCheck {
    Done,
    /// A hook blocked; `reason` (with the hook's note for the agent) goes back to the agent.
    Blocked {
        hook: String,
        reason: String,
    },
}

/// A commit `git_commit` is about to make, as `commit.pre` hooks see it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommitInput {
    pub message: String,
    /// The paths it changes, relative to the repo.
    pub paths: Vec<String>,
}

/// Whether the `commit.pre` hooks let a commit go ahead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitCheck {
    Passed,
    /// A hook blocked it; `reason` (with the hook's note for the agent) goes back to the agent.
    Blocked { hook: String, reason: String },
}

/// Runs a session's hooks.
pub trait HookRunner: PreToolHooks {
    /// After a command or write finished with `status` (`completed` or `failed`): the notes
    /// `tool.post` or `edit.post` hooks left for the agent.
    fn post<'a>(
        &'a self,
        action: &'a Action,
        mode: Mode,
        status: &'a str,
    ) -> BoxFuture<'a, Option<String>>;

    /// When a turn ended with the agent reporting it done.
    fn task_done(&self, mode: Mode) -> BoxFuture<'_, TaskCheck>;

    /// Before `git_commit` records a commit. A hook that denies, asks, fails, or times out
    /// blocks it: these hooks run in the agent's sandbox, so only a pass lets it through, and
    /// nothing they say can loosen what the policy decided.
    fn commit_pre<'a>(&'a self, mode: Mode, commit: &'a CommitInput)
    -> BoxFuture<'a, CommitCheck>;
}

/// What a session's hooks need to run.
pub struct HookContext {
    pub session: SessionId,
    pub project_root: PathBuf,
    /// Where hooks run: the session's first worktree.
    pub worktree: PathBuf,
    pub sandbox: Arc<dyn SessionSandbox>,
    pub log: Arc<dyn HookLog>,
}

/// Runs shell hooks in the session's sandbox.
pub struct ShellHooks {
    set: Arc<HookSet>,
    cx: HookContext,
}

/// A hook's JSON answer. Every field is optional.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Answer {
    verdict: Option<Verdict>,
    reason: Option<String>,
    rewrite: Option<Vec<String>>,
    note_for_agent: Option<String>,
}

/// How one hook run went.
enum Ran {
    /// Exit 0 with an answer, or with empty output (no opinion).
    Answered(Option<Answer>),
    /// It couldn't start, timed out, exited non-zero, or printed something else.
    Failed(String),
}

impl ShellHooks {
    pub fn new(set: Arc<HookSet>, cx: HookContext) -> Self {
        Self { set, cx }
    }

    fn matching<'a>(
        &'a self,
        event: HookEvent,
        action: Option<&'a Action>,
    ) -> impl Iterator<Item = &'a Hook> + 'a {
        self.set
            .hooks
            .iter()
            .filter(move |h| h.event == event && action.is_none_or(|a| matches(h, a)))
    }

    /// Run one hook with `input` on stdin and record the run.
    async fn run(&self, hook: &Hook, mut input: serde_json::Value, mode: Mode) -> Ran {
        input["event"] = hook.event.name().into();
        input["sessionId"] = self.cx.session.to_string().into();
        input["projectRoot"] = self.cx.project_root.display().to_string().into();
        input["worktree"] = self.cx.worktree.display().to_string().into();
        input["mode"] = mode_name(mode).into();
        let started = Instant::now();
        let (code, ran) = self.spawn(hook, input.to_string()).await;
        let verdict = match &ran {
            Ran::Answered(Some(answer)) => answer.verdict,
            _ => None,
        };
        if let Ran::Failed(why) = &ran {
            tracing::warn!(session_id = %self.cx.session, hook = %hook.name, event = hook.event.name(), "hook failed: {why}");
        }
        self.cx
            .log
            .ran(HookRunBody {
                hook: hook.name.clone(),
                event: hook.event.name().to_owned(),
                verdict,
                duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                exit_code: code,
            })
            .await;
        ran
    }

    async fn spawn(&self, hook: &Hook, input: String) -> (i32, Ran) {
        let env: Vec<(String, String)> = self
            .set
            .dir
            .iter()
            .map(|dir| (HOOKS_DIR_VAR.to_owned(), dir.display().to_string()))
            .collect();
        let mut cmd = self
            .cx
            .sandbox
            .command(&hook.program, &hook.args, &self.cx.worktree, &env);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .process_group(0);
        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(e) => return (NO_EXIT_CODE, Ran::Failed(format!("starting it: {e}"))),
        };
        // Whatever the hook started dies with it if it times out, prints too much, or the
        // run is dropped (a cancelled turn).
        let group = GroupGuard(child.id());
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("piped stdout");
        let talk = async {
            // Written and read together: a hook may print before it reads.
            let write = async move {
                if let Some(mut stdin) = stdin {
                    // A hook that doesn't read its input may have exited already.
                    let _ = stdin.write_all(input.as_bytes()).await;
                }
            };
            let read = async {
                let mut out = Vec::new();
                (&mut stdout)
                    .take(OUTPUT_LIMIT as u64 + 1)
                    .read_to_end(&mut out)
                    .await
                    .map(|_| out)
            };
            let ((), out) = tokio::join!(write, read);
            if out.as_ref().is_ok_and(|o| o.len() > OUTPUT_LIMIT) {
                return (out, None);
            }
            (out, Some(child.wait().await))
        };
        let Ok((out, status)) = tokio::time::timeout(hook.timeout, talk).await else {
            return (
                NO_EXIT_CODE,
                Ran::Failed(format!("timed out after {} ms", hook.timeout.as_millis())),
            );
        };
        let Some(status) = status else {
            return (
                NO_EXIT_CODE,
                Ran::Failed(format!("printed more than {OUTPUT_LIMIT} bytes")),
            );
        };
        group.disarm();
        let code = match &status {
            Ok(s) => s.code().unwrap_or(NO_EXIT_CODE),
            Err(_) => NO_EXIT_CODE,
        };
        let ran = match (status, out) {
            (Err(e), _) | (_, Err(e)) => Ran::Failed(e.to_string()),
            (Ok(s), _) if !s.success() => Ran::Failed(format!("exited with {s}")),
            (Ok(_), Ok(out)) if out.iter().all(u8::is_ascii_whitespace) => Ran::Answered(None),
            (Ok(_), Ok(out)) => match serde_json::from_slice::<Answer>(&out) {
                Ok(mut answer) => {
                    for text in [&mut answer.reason, &mut answer.note_for_agent]
                        .into_iter()
                        .flatten()
                    {
                        truncate(text);
                    }
                    Ran::Answered(Some(answer))
                }
                Err(e) => Ran::Failed(format!("its output isn't a hook answer: {e}")),
            },
        };
        (code, ran)
    }
}

impl PreToolHooks for ShellHooks {
    fn advise<'a>(&'a self, request: &'a Request, mode: Mode) -> BoxFuture<'a, Option<HookAdvice>> {
        Box::pin(async move {
            let event = match request.action {
                Action::Edit { .. } => HookEvent::EditPre,
                _ => HookEvent::ToolPre,
            };
            let mut action = request.action.clone();
            let mut advice: Option<HookAdvice> = None;
            let mut notes = Vec::new();
            // Each hook sees the action as the hooks before it left it. The first deny stops
            // the chain; an ask beats an allow.
            let hooks: Vec<&Hook> = self.matching(event, None).collect();
            for hook in hooks {
                if !matches(hook, &action) {
                    continue;
                }
                let input = serde_json::json!({ "action": action });
                // A pre hook that fails asks: an agent that can crash a deny hook must not
                // get past it that way.
                let answer = match self.run(hook, input, mode).await {
                    Ran::Answered(Some(answer)) => answer,
                    Ran::Answered(None) => continue,
                    Ran::Failed(why) => Answer {
                        verdict: Some(Verdict::Ask),
                        reason: Some(format!(
                            "hook {} failed ({why}); a human decides",
                            hook.name
                        )),
                        ..Answer::default()
                    },
                };
                let advice = advice.get_or_insert_with(|| HookAdvice {
                    hook: hook.name.clone(),
                    ..HookAdvice::default()
                });
                notes.extend(answer.note_for_agent);
                if let Some(argv) = answer.rewrite {
                    match (&action, argv.is_empty()) {
                        (Action::Exec { cwd, .. }, false) => {
                            action = Action::Exec {
                                argv: argv.clone(),
                                cwd: cwd.clone(),
                            };
                            advice.rewrite = Some(argv);
                            advice.rewritten_by = Some(hook.name.clone());
                        }
                        _ => {
                            advice.hook = hook.name.clone();
                            advice.verdict = Some(Verdict::Deny);
                            advice.reason = Some(format!(
                                "hook {} returned a rewrite that isn't a command",
                                hook.name
                            ));
                            break;
                        }
                    }
                }
                let stronger = matches!(
                    (answer.verdict, advice.verdict),
                    (Some(Verdict::Deny), _)
                        | (Some(Verdict::Ask), None | Some(Verdict::Allow))
                        | (Some(Verdict::Allow), None)
                );
                if stronger {
                    advice.hook = hook.name.clone();
                    advice.verdict = answer.verdict;
                    advice.reason = answer.reason;
                }
                if advice.verdict == Some(Verdict::Deny) {
                    break;
                }
            }
            let mut advice = advice?;
            if !notes.is_empty() {
                advice.note = Some(notes.join("\n"));
            }
            Some(advice)
        })
    }
}

impl HookRunner for ShellHooks {
    fn post<'a>(
        &'a self,
        action: &'a Action,
        mode: Mode,
        status: &'a str,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let event = match action {
                Action::Edit { .. } => HookEvent::EditPost,
                _ => HookEvent::ToolPost,
            };
            let mut notes = Vec::new();
            for hook in self.matching(event, Some(action)) {
                let input = serde_json::json!({ "action": action, "status": status });
                if let Ran::Answered(Some(answer)) = self.run(hook, input, mode).await {
                    notes.extend(answer.note_for_agent);
                }
            }
            (!notes.is_empty()).then(|| notes.join("\n"))
        })
    }

    fn task_done(&self, mode: Mode) -> BoxFuture<'_, TaskCheck> {
        Box::pin(async move {
            for hook in self.matching(HookEvent::TaskDone, None) {
                let blocked = |reason: String| TaskCheck::Blocked {
                    hook: hook.name.clone(),
                    reason,
                };
                match self.run(hook, serde_json::json!({}), mode).await {
                    Ran::Failed(why) => {
                        return blocked(format!("The task.done hook {} failed: {why}.", hook.name));
                    }
                    Ran::Answered(Some(answer)) if answer.verdict == Some(Verdict::Deny) => {
                        let mut reason = answer.reason.unwrap_or_else(|| {
                            format!("The task.done hook {} says the task isn't done.", hook.name)
                        });
                        if let Some(note) = answer.note_for_agent {
                            reason.push_str("\n\n");
                            reason.push_str(&note);
                        }
                        return blocked(reason);
                    }
                    Ran::Answered(_) => {}
                }
            }
            TaskCheck::Done
        })
    }

    fn commit_pre<'a>(
        &'a self,
        mode: Mode,
        commit: &'a CommitInput,
    ) -> BoxFuture<'a, CommitCheck> {
        Box::pin(async move {
            for hook in self.matching(HookEvent::CommitPre, None) {
                let blocked = |reason: String| CommitCheck::Blocked {
                    hook: hook.name.clone(),
                    reason,
                };
                let input = serde_json::json!({ "message": commit.message, "paths": commit.paths });
                match self.run(hook, input, mode).await {
                    Ran::Failed(why) => {
                        return blocked(format!("The commit.pre hook {} failed: {why}.", hook.name));
                    }
                    Ran::Answered(Some(answer))
                        if matches!(answer.verdict, Some(Verdict::Deny) | Some(Verdict::Ask)) =>
                    {
                        let mut reason = answer.reason.unwrap_or_else(|| {
                            format!("The commit.pre hook {} blocked the commit.", hook.name)
                        });
                        if answer.verdict == Some(Verdict::Ask) {
                            reason = format!(
                                "The commit.pre hook {} asked ({reason}), and commit.pre hooks can only pass or block.",
                                hook.name
                            );
                        }
                        if let Some(note) = answer.note_for_agent {
                            reason.push_str("\n\n");
                            reason.push_str(&note);
                        }
                        return blocked(reason);
                    }
                    Ran::Answered(_) => {}
                }
            }
            CommitCheck::Passed
        })
    }
}

/// Whether `hook`'s filter lets `action` through.
fn matches(hook: &Hook, action: &Action) -> bool {
    let kind = match action {
        Action::Exec { .. } => HookActionType::Exec,
        Action::Edit { .. } => HookActionType::Edit,
        Action::Mcp { .. } => HookActionType::Mcp,
        Action::Git { .. } => HookActionType::Git,
        Action::Network { .. } => return false,
    };
    if hook.action.is_some_and(|a| a != kind) {
        return false;
    }
    if let Some(glob) = &hook.argv {
        let Action::Exec { argv, .. } = action else {
            return false;
        };
        let cmd = CommandLine::parse(argv);
        if !glob.matches(&cmd.line) && !cmd.segment_lines().any(|l| glob.matches(&l)) {
            return false;
        }
    }
    if let Some(patterns) = &hook.path {
        let Action::Edit { path, .. } = action else {
            return false;
        };
        if normalize(path).is_none_or(|p| patterns.first_match(&p).is_none()) {
            return false;
        }
    }
    true
}

/// `text` cut to at most [`TEXT_LIMIT`] bytes, marking the cut.
fn truncate(text: &mut String) {
    if text.len() > TEXT_LIMIT {
        let mut cut = TEXT_LIMIT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str(" [cut]");
    }
}

/// Kills a process group when dropped, unless disarmed.
struct GroupGuard(Option<u32>);

impl GroupGuard {
    fn disarm(mut self) {
        self.0 = None;
    }
}

impl Drop for GroupGuard {
    fn drop(&mut self) {
        kill_group(self.0);
    }
}

fn kill_group(pid: Option<u32>) {
    let Some(pid) = pid
        .and_then(|p| i32::try_from(p).ok())
        .and_then(rustix::process::Pid::from_raw)
    else {
        return;
    };
    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
}

#[cfg(test)]
mod tests;
