//! `git_commit`, the agent's only way to commit: Forge's MCP endpoint calls it, and the daemon
//! commits on the session branch once the policy and the `commit.pre` hooks let it.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use forge_acp::{Gate, GateKind, GateRequest, GateVerdict, Unmapped};
use forge_api::events::{EventPayload, PolicyDecisionBody, ToolCallBody, ToolResultBody, Verdict};
use forge_api::model::Action;
use forge_git::GitDirs;
use forge_git::commit::{self, CommitError};
use forge_hooks::{CommitCheck, CommitInput};
use forge_mcp::{ForgeTools, GitCommitArgs, GitCommitResult};
use futures_util::future::BoxFuture;

use super::approvals::SessionGate;
use super::lock;
use forge_acp::EventSink;

/// Where a session's commits go.
#[derive(Clone, Debug)]
pub(super) struct Target {
    /// The worktree whose changes are committed: the session's working directory.
    pub worktree: PathBuf,
    pub dirs: GitDirs,
    pub branch: String,
    /// The index commits are staged in, in a dir only the daemon writes.
    pub index: PathBuf,
}

/// The tool behind the session's MCP endpoint. The endpoint starts with the proxy, before the
/// agent and its gate exist, so the tool is filled in once they do.
#[derive(Default)]
pub(super) struct CommitTool {
    ready: Mutex<Option<(Arc<SessionGate>, Target)>>,
    /// One commit at a time.
    running: tokio::sync::Mutex<()>,
}

impl CommitTool {
    pub fn set(&self, gate: Arc<SessionGate>, target: Target) {
        *lock(&self.ready) = Some((gate, target));
    }
}

impl ForgeTools for CommitTool {
    fn git_commit(&self, args: GitCommitArgs) -> BoxFuture<'_, Result<GitCommitResult, String>> {
        Box::pin(async move {
            let _one = self.running.lock().await;
            let Some((gate, target)) = lock(&self.ready).clone() else {
                return Err("Forge's tools aren't ready yet; try again".into());
            };
            run(&gate, &target, args).await
        })
    }
}

async fn run(
    gate: &SessionGate,
    target: &Target,
    args: GitCommitArgs,
) -> Result<GitCommitResult, String> {
    let id = format!("tc_{}", ulid::Ulid::new());
    let action = Action::Git {
        op: "commit".into(),
    };
    let title = format!(
        "git_commit: {}",
        args.message.lines().next().unwrap_or_default()
    );
    gate.sink
        .emit(EventPayload::ToolCall(ToolCallBody {
            tool_call_id: id.clone(),
            action: action.clone(),
            title: title.clone(),
            source: Default::default(),
        }))
        .await;
    let fail = async |reason: String| {
        result(gate, &id, "failed", &reason).await;
        Err(reason)
    };
    let staged = {
        let (t, paths) = (target.clone(), args.paths.clone());
        tokio::task::spawn_blocking(move || {
            commit::stage(&t.dirs, &t.worktree, &t.branch, paths.as_deref(), &t.index)
        })
        .await
    };
    let staged = match staged {
        Ok(Ok(staged)) => staged,
        Ok(Err(e)) => return fail(commit_error(e)).await,
        Err(e) => return fail(format!("Forge couldn't stage the commit: {e}")).await,
    };
    // What the commit changes is held to the same guards as an edit, whatever tool wrote it.
    for path in &staged.changed {
        if let Some(d) = gate.engine.guard_path(path) {
            record(gate, &id, Verdict::Deny, &d.reason, &d.source.to_string()).await;
            return fail(d.reason).await;
        }
    }
    if staged.cut {
        let reason = "the commit is too large to scan for secrets; commit fewer paths at a time";
        record(gate, &id, Verdict::Deny, reason, "guard:secret-scan").await;
        return fail(reason.into()).await;
    }
    let request = GateRequest {
        kind: GateKind::Tool,
        tool_call_id: id.clone(),
        title,
        action: Some(action.clone()),
        content: Some(staged.added.clone()),
        unmapped: Unmapped::Nothing,
        detail: Some(changed_paths(&staged.changed)),
    };
    match gate.decide(request).await {
        GateVerdict::Allow => {}
        GateVerdict::Deny(refusal) => return fail(refusal.reason).await,
        GateVerdict::HumanDeny(note) => {
            return fail(note.unwrap_or_else(|| "a human denied it".into())).await;
        }
        GateVerdict::Cancelled => return fail("the turn was cancelled".into()).await,
        GateVerdict::Rewrite(_) => {
            return fail("a hook rewrote the commit, which can't be rewritten".into()).await;
        }
    }
    let mode = gate.mode().await;
    let input = CommitInput {
        message: args.message.clone(),
        paths: staged.changed.clone(),
    };
    if let CommitCheck::Blocked { hook, reason } = gate.hooks.commit_pre(mode, &input).await {
        record(gate, &id, Verdict::Deny, &reason, &format!("hook:{hook}")).await;
        let _ = gate.finished(GateKind::Tool, &action, false).await;
        return fail(reason).await;
    }
    // The hooks checked the worktree as it is now; commit only if that is still what was
    // approved.
    let again = {
        let (t, paths) = (target.clone(), args.paths.clone());
        tokio::task::spawn_blocking(move || {
            commit::stage(&t.dirs, &t.worktree, &t.branch, paths.as_deref(), &t.index)
        })
        .await
    };
    match again {
        Ok(Ok(now)) if now.tree == staged.tree && now.parent == staged.parent => {}
        Ok(_) | Err(_) => {
            let _ = gate.finished(GateKind::Tool, &action, false).await;
            return fail(
                "the worktree changed while the commit was being checked; call git_commit again"
                    .into(),
            )
            .await;
        }
    }
    let made = {
        let (t, message) = (target.clone(), args.message);
        tokio::task::spawn_blocking(move || {
            commit::commit(&t.dirs, &t.worktree, &t.branch, &staged, &message, &t.index)
        })
        .await
    };
    let commit = match made {
        Ok(Ok(commit)) => commit,
        Ok(Err(e)) => {
            let _ = gate.finished(GateKind::Tool, &action, false).await;
            return fail(commit_error(e)).await;
        }
        Err(e) => return fail(format!("Forge couldn't commit: {e}")).await,
    };
    let note = gate.finished(GateKind::Tool, &action, true).await;
    let mut done = format!("committed {commit} on {}", target.branch);
    if let Some(note) = note {
        done.push_str("\n\n");
        done.push_str(&note);
    }
    result(gate, &id, "completed", &done).await;
    Ok(GitCommitResult {
        commit,
        branch: target.branch.clone(),
    })
}

/// The paths a commit changes, as the human deciding sees them.
fn changed_paths(changed: &[String]) -> String {
    const SHOWN: usize = 20;
    let mut text = format!(
        "changes {}",
        changed
            .iter()
            .take(SHOWN)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    );
    if changed.len() > SHOWN {
        text.push_str(&format!(" and {} more", changed.len() - SHOWN));
    }
    text
}

fn commit_error(e: CommitError) -> String {
    match e {
        CommitError::Git(e) => format!("git failed: {e}"),
        other => other.to_string(),
    }
}

async fn record(gate: &SessionGate, id: &str, verdict: Verdict, reason: &str, source: &str) {
    gate.sink
        .emit(EventPayload::PolicyDecision(PolicyDecisionBody {
            tool_call_id: id.to_owned(),
            verdict,
            reason: reason.to_owned(),
            source: source.to_owned(),
        }))
        .await;
}

async fn result(gate: &SessionGate, id: &str, status: &str, output: &str) {
    gate.sink
        .emit(EventPayload::ToolResult(ToolResultBody {
            tool_call_id: id.to_owned(),
            status: status.to_owned(),
            output: Some(output.to_owned()),
        }))
        .await;
}
