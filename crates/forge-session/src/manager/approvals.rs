//! The session's gate: every permission request, write, and terminal the agent asks for goes
//! through the policy engine, and an Ask waits in the approvals inbox until a human decides,
//! the turn is cancelled, or the agent stops.

use std::sync::{Arc, Weak};

use forge_acp::{EventSink, Gate, GateKind, GateRequest, GateVerdict, Unmapped};
use forge_api::events::{
    ApprovalDecidedBody, ApprovalDecision, ApprovalRequestedBody, EventPayload, PolicyDecisionBody,
    Remember, Verdict,
};
use forge_api::model::{Action, Approval, EditKind, Mode, SessionState};
use forge_api::{ApiError, ApprovalId, ErrorKind, SessionId, Timestamp};
use forge_config::{PolicyRule, RuleListName};
use forge_hooks::HookRunner;
use forge_policy::{Engine, Outcome, Policy, Request, SecretScanner, Source, exact_pattern};
use futures_util::future::BoxFuture;
use tokio::sync::oneshot;

use super::{Live, SessionError, SessionManager, SessionSink, invalid_state, lock};
use crate::redact::redact_action;
use crate::store::ApprovalOutcome;

/// Content longer than this is scanned for secrets on the blocking pool.
const LARGE_CONTENT: usize = 64 * 1024;

/// An Ask waiting in the inbox.
pub(super) struct Pending {
    /// The action as the agent asked for it, unredacted, for remembering it.
    action: Action,
    /// Whether remembering an allow could change later decisions.
    rememberable: bool,
    /// Whether the action's text holds something the secret scan flags, so it must never be
    /// written to a policy file.
    secret: bool,
    kind: GateKind,
    reply: oneshot::Sender<GateVerdict>,
}

/// Decides one session's agent requests.
pub(super) struct SessionGate {
    pub(super) manager: Weak<SessionManager>,
    pub(super) session: SessionId,
    pub(super) live: Arc<Live>,
    pub(super) engine: Arc<Engine>,
    pub(super) hooks: Arc<dyn HookRunner>,
    pub(super) scanner: Arc<SecretScanner>,
    pub(super) sink: Arc<SessionSink>,
}

impl Gate for SessionGate {
    fn finished<'a>(
        &'a self,
        kind: GateKind,
        action: &'a Action,
        success: bool,
    ) -> BoxFuture<'a, Option<String>> {
        Box::pin(async move {
            let manager = self.manager.upgrade()?;
            let mode = manager.get(&self.session).await.ok()?.mode;
            let status = if success { "completed" } else { "failed" };
            let note = self.hooks.post(action, mode, status).await?;
            if kind == GateKind::Terminal {
                return Some(note);
            }
            // A write's response has no room for text, so the note comes with the next prompt.
            queue_note(&self.live, note);
            None
        })
    }

    fn decide(&self, request: GateRequest) -> BoxFuture<'_, GateVerdict> {
        Box::pin(async move {
            let id = request.tool_call_id.clone();
            match self.evaluate(request).await {
                Ok(verdict) => verdict,
                Err(e) => {
                    tracing::error!(session_id = %self.session, tool_call_id = %id, error = %e, "deciding an agent request");
                    GateVerdict::Deny(format!("Forge couldn't decide this request: {e}"))
                }
            }
        })
    }
}

impl SessionGate {
    /// The mode the session's next decision uses.
    pub(super) async fn mode(&self) -> Mode {
        match self.manager.upgrade() {
            Some(manager) => manager
                .get(&self.session)
                .await
                .map_or(Mode::Ask, |s| s.mode),
            None => Mode::Ask,
        }
    }

    async fn evaluate(&self, request: GateRequest) -> Result<GateVerdict, SessionError> {
        let id = request.tool_call_id.clone();
        let Some(manager) = self.manager.upgrade() else {
            return Ok(GateVerdict::Deny("Forge is shutting down".into()));
        };
        let Some(action) = request.action else {
            let (verdict, reason, source) = match request.unmapped {
                // The sandbox bounds every read; Forge only keeps reads to the worktrees.
                Unmapped::Read {
                    inside_worktree: true,
                } => (
                    Verdict::Allow,
                    "reading inside the session's worktrees",
                    "read:worktree",
                ),
                Unmapped::Read {
                    inside_worktree: false,
                } => (
                    Verdict::Deny,
                    "the agent may only read inside the session's worktrees",
                    "guard:outside-worktree",
                ),
                Unmapped::Fetch => (
                    Verdict::Deny,
                    "agents can't fetch with their own tools; the egress proxy's allowlist \
                     decides the network, so fetch with a command, which goes through it",
                    "network:allowlist",
                ),
                Unmapped::Nothing => (
                    Verdict::Deny,
                    "the request describes nothing Forge can check, so it is refused",
                    "guard:unknown-action",
                ),
            };
            self.record(&id, verdict, reason, source).await;
            return Ok(match verdict {
                Verdict::Allow => GateVerdict::Allow,
                _ => GateVerdict::Deny(reason.into()),
            });
        };
        let mode = manager.get(&self.session).await?.mode;
        let checked = Request {
            action,
            content: request.content,
        };
        let large = checked
            .content
            .as_ref()
            .is_some_and(|c| c.len() > LARGE_CONTENT);
        let outcome = if large {
            // Only the scan of the content runs on the blocking pool; hooks may take minutes
            // and must not hold a blocking thread meanwhile.
            let (engine, request) = (self.engine.clone(), checked.clone());
            let early = tokio::task::spawn_blocking(move || engine.check(&request, mode))
                .await
                .map_err(|e| SessionError::Internal(e.to_string()))?;
            match early {
                Some(decision) => Outcome {
                    decision,
                    action: checked.action.clone(),
                    note: None,
                    rewritten_by: None,
                },
                None => self.engine.decide_checked(&checked, mode).await,
            }
        } else {
            self.engine.decide(&checked, mode).await
        };
        let mut decision = outcome.decision;
        let note = outcome.note;
        // A rewrite always shows in the decision, whoever decided.
        let shown = match (&outcome.rewritten_by, &checked.action, &outcome.action) {
            (Some(hook), Action::Exec { argv: from, .. }, Action::Exec { argv: to, .. }) => {
                format!(
                    "hook {hook} rewrote `{}` to `{}`; {}",
                    from.join(" "),
                    to.join(" "),
                    decision.reason
                )
            }
            _ => decision.reason.clone(),
        };
        if outcome.action != checked.action && decision.verdict != Verdict::Deny {
            // A hook rewrote the action. Only a terminal can run the rewritten command; the
            // agent's own tools can't, so it is told what to run instead.
            let refusal = match (&outcome.action, request.kind) {
                (Action::Exec { .. }, GateKind::Terminal) => None,
                (Action::Exec { argv, .. }, GateKind::Permission) => Some(format!(
                    "a hook changed the command to `{}`; run that instead",
                    argv.join(" ")
                )),
                _ => Some("a hook rewrote an action that can't be rewritten".to_owned()),
            };
            if let Some(reason) = refusal {
                let source = decision.source.to_string();
                self.record(&id, Verdict::Deny, &format!("{shown}; {reason}"), &source)
                    .await;
                return Ok(GateVerdict::Deny(with_note(reason, note)));
            }
        }
        // A human who allowed exactly this in the agent's permission request has answered
        // the ask already; the grant is good once.
        // Only an ask from the mode: the permission request didn't show the human what a guard
        // or rule would flag in the write itself (its content, for one).
        let granted = decision.verdict == Verdict::Ask
            && matches!(decision.source, Source::Mode(_))
            && matches!(request.kind, GateKind::Write | GateKind::Terminal)
            && self.take_grant(&outcome.action);
        let verdict = if granted {
            let reason = "a human allowed this in the agent's permission request";
            self.record(&id, Verdict::Allow, reason, "approval").await;
            GateVerdict::Allow
        } else {
            let source = decision.source.to_string();
            self.record(&id, decision.verdict, &shown, &source).await;
            decision.reason = shown;
            match decision.verdict {
                Verdict::Allow => GateVerdict::Allow,
                Verdict::Deny => GateVerdict::Deny(decision.reason),
                Verdict::Ask => {
                    let secret = !self.scanner.kinds(&describe(&outcome.action)).is_empty();
                    let pending = |reply| Pending {
                        action: outcome.action.clone(),
                        rememberable: decision.source.rememberable(),
                        secret,
                        kind: request.kind,
                        reply,
                    };
                    manager
                        .ask(
                            &self.live,
                            &self.session,
                            &self.scanner,
                            &decision.reason,
                            pending,
                        )
                        .await?
                }
            }
        };
        Ok(match verdict {
            GateVerdict::Deny(reason) => GateVerdict::Deny(with_note(reason, note)),
            GateVerdict::Allow => {
                // The agent hears an allowed action's hook notes with its next prompt.
                if let Some(note) = note {
                    queue_note(&self.live, note);
                }
                match outcome.action {
                    Action::Exec { argv, .. } if outcome.action != checked.action => {
                        GateVerdict::Rewrite(argv)
                    }
                    _ => GateVerdict::Allow,
                }
            }
            other => other,
        })
    }

    async fn record(&self, tool_call_id: &str, verdict: Verdict, reason: &str, source: &str) {
        self.sink
            .emit(EventPayload::PolicyDecision(PolicyDecisionBody {
                tool_call_id: tool_call_id.to_owned(),
                verdict,
                reason: reason.to_owned(),
                source: source.to_owned(),
            }))
            .await;
    }

    fn take_grant(&self, action: &Action) -> bool {
        let mut grants = lock(&self.live.grants);
        let found = grants.iter().position(|g| granted(g, action));
        found.map(|i| grants.remove(i)).is_some()
    }
}

/// Most text queued in hook notes for the next prompt; older notes are dropped first.
const NOTES_LIMIT: usize = 16 * 1024;

/// Queue a hook note for the agent's next prompt, keeping the queue under [`NOTES_LIMIT`].
fn queue_note(live: &Live, note: String) {
    const DROPPED: &str = "(earlier notes dropped)";
    let mut notes = lock(&live.notes);
    let mut dropped = notes.first().is_some_and(|n| n == DROPPED);
    if dropped {
        notes.remove(0);
    }
    notes.push(note);
    while notes.len() > 1 && notes.iter().map(String::len).sum::<usize>() > NOTES_LIMIT {
        notes.remove(0);
        dropped = true;
    }
    if dropped {
        notes.insert(0, DROPPED.to_owned());
    }
}

/// `reason`, followed by a hook's note for the agent.
fn with_note(reason: String, note: Option<String>) -> String {
    match note {
        Some(note) => format!("{reason}\n{note}"),
        None => reason,
    }
}

/// Whether a grant from a permission request covers `action`: the same argv, or the same
/// path. A permission request can't tell a create from a modify, so a modify grant covers
/// both; a delete grant covers only a delete.
fn granted(grant: &Action, action: &Action) -> bool {
    match (grant, action) {
        (Action::Exec { argv: a, .. }, Action::Exec { argv: b, .. }) => a == b,
        (Action::Edit { path: a, kind: k }, Action::Edit { path: b, kind: l }) => {
            (*k == EditKind::Delete) == (*l == EditKind::Delete)
                && forge_policy::paths::normalize(a).is_some()
                && forge_policy::paths::normalize(a) == forge_policy::paths::normalize(b)
        }
        _ => false,
    }
}

impl SessionManager {
    /// Put an Ask in the inbox, move the session to Waiting, and wait for the answer.
    async fn ask(
        &self,
        live: &Live,
        id: &SessionId,
        scanner: &SecretScanner,
        reason: &str,
        pending: impl FnOnce(oneshot::Sender<GateVerdict>) -> Pending,
    ) -> Result<GateVerdict, SessionError> {
        let (tx, rx) = oneshot::channel();
        {
            let _guard = live.lock.lock().await;
            let session = self.get(id).await?;
            if !matches!(session.state, SessionState::Working | SessionState::Waiting) {
                return Ok(GateVerdict::Deny(
                    "the agent asked outside a turn, so no one is there to decide".into(),
                ));
            }
            let pending = pending(tx);
            let approval = Approval {
                id: ApprovalId::new(),
                session_id: id.clone(),
                action: redact_action(scanner, &pending.action),
                reason: scanner.redact(reason).into_owned(),
                created_at: Timestamp::now(),
            };
            let record = approval.clone();
            self.blocking(move |store| store.insert_approval(&record))
                .await?;
            lock(&live.pending).insert(approval.id.clone(), pending);
            self.emit(
                id,
                EventPayload::ApprovalRequested(ApprovalRequestedBody {
                    approval_id: approval.id,
                    action: approval.action,
                    reason: approval.reason,
                }),
            )
            .await?;
            if session.state == SessionState::Working {
                self.transition(&session, SessionState::Waiting, None)
                    .await?;
            }
        }
        Ok(rx
            .await
            .unwrap_or_else(|_| GateVerdict::Deny("the approval ended without an answer".into())))
    }

    /// Pending approvals, oldest first, optionally only one project's.
    pub async fn approvals(
        &self,
        project: Option<forge_api::ProjectId>,
    ) -> Result<Vec<Approval>, SessionError> {
        self.blocking(move |store| store.pending_approvals(project.as_ref()))
            .await
    }

    /// A human's answer to a pending approval. Remembering it keeps it for the rest of the
    /// session, or also writes it to the project's policy file; either way it applies at once.
    pub async fn decide(
        &self,
        id: &ApprovalId,
        decision: ApprovalDecision,
        remember: Option<Remember>,
        note: Option<String>,
    ) -> Result<(), SessionError> {
        let key = id.clone();
        let record = self
            .blocking(move |store| store.approval(&key))
            .await?
            .ok_or_else(|| ApiError::not_found(format!("approval {id}")))?;
        let gone = || {
            ApiError::new(
                ErrorKind::InvalidState,
                format!("approval {id} has already ended"),
            )
        };
        if record.decided {
            return Err(gone().into());
        }
        let session_id = record.approval.session_id;
        let live = self.live(&session_id);
        let _guard = live.lock.lock().await;
        let (action, rememberable, secret, kind) = lock(&live.pending)
            .get(id)
            .map(|p| (p.action.clone(), p.rememberable, p.secret, p.kind))
            .ok_or_else(gone)?;
        let verdict = match decision {
            ApprovalDecision::Allow => Verdict::Allow,
            ApprovalDecision::Deny => Verdict::Deny,
        };
        if let Some(remember) = remember {
            if verdict == Verdict::Allow && !rememberable {
                return Err(ApiError::validation(format!(
                    "remembering this allow would change nothing, since what asked ({}) asks \
                     again whatever was allowed before; decide without remember",
                    record.approval.reason
                ))
                .into());
            }
            if remember == Remember::Project && secret {
                return Err(ApiError::validation(
                    "this looks like it contains a secret, which must not be written to the \
                     project's policy file; remember it for the session instead",
                )
                .into());
            }
            if remember == Remember::Project {
                self.remember_in_project(&session_id, &action, verdict)
                    .await?;
            }
            lock(&live.remembered).push((action.clone(), verdict));
            if let Some(policy) = lock(&live.policy).clone() {
                policy.remember(&action, verdict);
            }
        }
        let outcome = match decision {
            ApprovalDecision::Allow => ApprovalOutcome::Allow,
            ApprovalDecision::Deny => ApprovalOutcome::Deny,
        };
        let key = id.clone();
        let ended = self
            .blocking(move |store| store.end_approval(&key, outcome, remember, Timestamp::now()))
            .await?;
        if !ended {
            return Err(gone().into());
        }
        self.emit(
            &session_id,
            EventPayload::ApprovalDecided(ApprovalDecidedBody {
                approval_id: id.clone(),
                decision,
                remember,
            }),
        )
        .await?;
        let pending = lock(&live.pending).remove(id);
        self.resume_if_answered(&live, &session_id).await?;
        if verdict == Verdict::Allow && kind == GateKind::Permission {
            lock(&live.grants).push(action);
        }
        if let Some(pending) = pending {
            let _ = pending.reply.send(match verdict {
                Verdict::Allow => GateVerdict::Allow,
                _ => GateVerdict::Deny(note.unwrap_or_else(|| "a human denied it".into())),
            });
        }
        Ok(())
    }

    /// Append a rule matching exactly `action` to the project's policy file.
    async fn remember_in_project(
        &self,
        session: &SessionId,
        action: &Action,
        verdict: Verdict,
    ) -> Result<(), SessionError> {
        let pattern = exact_pattern(action).map_err(|why| {
            ApiError::validation(format!(
                "this can't be remembered for the project: {why}; remember it for the session \
                 instead"
            ))
        })?;
        let (list, done) = match verdict {
            Verdict::Allow => (RuleListName::Allow, "allowed"),
            _ => (RuleListName::Deny, "denied"),
        };
        let rule = PolicyRule {
            exec: vec![pattern],
            reason: Some(format!("{done} from the approvals inbox")),
            ..PolicyRule::default()
        };
        let (agents, session) = (self.agents.clone(), self.get(session).await?);
        tokio::task::spawn_blocking(move || agents.remember_in_project(&session, list, rule))
            .await
            .map_err(|e| SessionError::Internal(e.to_string()))??;
        Ok(())
    }

    /// End every pending approval of the session as cancelled and move it back to Working.
    /// The caller holds the session's lock.
    pub(super) async fn cancel_pending(
        &self,
        live: &Live,
        id: &SessionId,
    ) -> Result<(), SessionError> {
        let drained: Vec<(ApprovalId, Pending)> = lock(&live.pending).drain().collect();
        for (approval, _) in &drained {
            let key = approval.clone();
            self.blocking(move |store| {
                store.end_approval(&key, ApprovalOutcome::Cancelled, None, Timestamp::now())
            })
            .await?;
        }
        self.resume_if_answered(live, id).await?;
        for (_, pending) in drained {
            let _ = pending.reply.send(GateVerdict::Cancelled);
        }
        Ok(())
    }

    /// Back to Working once nothing is pending. The caller holds the session's lock.
    async fn resume_if_answered(&self, live: &Live, id: &SessionId) -> Result<(), SessionError> {
        let session = self.get(id).await?;
        if lock(&live.pending).is_empty() && session.state == SessionState::Waiting {
            self.transition(&session, SessionState::Working, None)
                .await?;
        }
        Ok(())
    }

    /// After a restart, cancel what was waiting and keep a note for each session's agent.
    pub(super) async fn cancel_stale_approvals(&self) -> Result<(), SessionError> {
        let cancelled = self
            .blocking(|store| store.cancel_pending_approvals(Timestamp::now()))
            .await?;
        let mut by_session: Vec<(SessionId, Vec<String>)> = Vec::new();
        for approval in cancelled {
            let what = describe(&approval.action);
            match by_session
                .iter_mut()
                .find(|(s, _)| *s == approval.session_id)
            {
                Some((_, list)) => list.push(what),
                None => by_session.push((approval.session_id, vec![what])),
            }
        }
        for (session, actions) in by_session {
            let note = format!(
                "Forge restarted while these actions waited for approval; they were cancelled \
                 and did not run:\n{}\nAsk again if you still need them.",
                actions
                    .iter()
                    .map(|a| format!("- {a}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
            *lock(&self.live(&session).resume_note) = Some(note);
        }
        Ok(())
    }

    /// Change the mode later decisions use. The sandbox tier is fixed for the session.
    pub async fn set_mode(
        &self,
        id: &SessionId,
        mode: forge_api::model::Mode,
    ) -> Result<forge_api::model::Session, SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        if !crate::state::is_live(session.state) {
            return Err(invalid_state(&session, "change mode"));
        }
        let key = id.clone();
        self.blocking(move |store| store.set_mode(&key, mode, Timestamp::now()))
            .await?;
        self.get(id).await
    }
}

/// A short description of an action, for the agent.
fn describe(action: &Action) -> String {
    match action {
        Action::Exec { argv, .. } => argv.join(" "),
        Action::Edit { path, .. } => format!("edit {path}"),
        Action::Network { host, port } => format!("connect to {host}:{port}"),
        Action::Mcp { server, tool, .. } => format!("{server}/{tool}"),
        Action::Git { op } => format!("git {op}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grants_cover_the_same_argv_or_path() {
        let exec = |a: &[&str]| Action::Exec {
            argv: a.iter().map(|s| s.to_string()).collect(),
            cwd: ".".into(),
        };
        let edit = |p: &str, kind| Action::Edit {
            path: p.into(),
            kind,
        };
        use forge_api::model::EditKind::*;
        assert!(granted(&exec(&["ls", "-la"]), &exec(&["ls", "-la"])));
        assert!(!granted(&exec(&["ls", "-la"]), &exec(&["ls", "-l", "-a"])));
        assert!(granted(&edit("a/./b.rs", Modify), &edit("a/b.rs", Create)));
        assert!(!granted(&edit("a/b.rs", Delete), &edit("a/b.rs", Modify)));
        assert!(!granted(&edit("a/b.rs", Modify), &edit("a/b.rs", Delete)));
        assert!(!granted(&edit("a/b.rs", Modify), &edit("a/c.rs", Modify)));
        assert!(!granted(&edit("../x", Modify), &edit("../x", Modify)));
        assert!(!granted(&exec(&["a.rs"]), &edit("a.rs", Modify)));
    }
}
