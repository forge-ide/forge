//! The session manager: creates sessions, runs their turns, and keeps their state.
//!
//! Each session's state lives in the store and changes only through [`SessionManager`], one
//! change at a time per session, with a `session.state` event for each. Turns and provisioning
//! run as background tasks; the agent connection outlives individual turns and is relaunched
//! when a prompt arrives and it is gone (after a crash or a daemon restart).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use forge_acp::{AcpClient, AcpError, AgentCommand, DenyAll, EventSink, Root, Workspace};
use forge_api::events::{
    EventPayload, PromptBody, SessionStateBody, TurnEndedBody, TurnSource, TurnStartedBody,
};
use forge_api::events::{NetworkBody, Verdict};
use forge_api::methods::{BranchCollision, CollisionResolution};
use forge_api::model::{Mode, Project, SandboxTier, Session, SessionState, Usage};
use forge_api::{ApiError, ErrorData, ErrorKind, ProjectId, SessionId, Timestamp, TurnId};
use forge_events::EventLog;
use forge_git::{CreateMode, GitError, Worktrees, worktree_path};
use forge_sandbox::proxy::{
    Allowlist, Connection, Limits, Proxy, ProxyConfig, PublicDns, SecretRoute, Tls, Upstream,
};
use forge_sandbox::{Sandbox, Spec, WorktreeMount};
use futures_util::future::BoxFuture;

use crate::naming::{RepoName, decide};
use crate::state::{accepts_prompt, can_close, can_transition, is_live};
use crate::store::{SessionStore, StoreError, WorktreeRecord};

/// Why agents' permission requests are denied until a policy engine decides them.
const NO_POLICY: &str = "no policy engine is configured to approve requests";

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    /// An error for the API client.
    #[error(transparent)]
    Api(#[from] ApiError),
    /// A daemon fault, for the log.
    #[error("{0}")]
    Internal(String),
}

impl From<StoreError> for SessionError {
    fn from(e: StoreError) -> Self {
        SessionError::Internal(e.to_string())
    }
}

impl From<forge_events::LogError> for SessionError {
    fn from(e: forge_events::LogError) -> Self {
        SessionError::Internal(e.to_string())
    }
}

/// How to start a session's agent.
#[derive(Clone, Debug)]
pub struct AgentLaunch {
    pub command: AgentCommand,
    /// Paths the sandbox shows read-only besides its defaults: the agent program and the
    /// user config's extra read-only paths.
    pub read_only: Vec<PathBuf>,
    /// Hosts the session's egress proxy lets through.
    pub network: Allowlist,
    /// Secrets the proxy injects, with their values from the keyring.
    pub secrets: Vec<SecretRoute>,
}

/// Finds how to start a session's agent from its profile. The daemon implements it from
/// config.
pub trait AgentResolver: Send + Sync + 'static {
    /// Blocking; may read config files.
    fn launch(&self, session: &Session) -> Result<AgentLaunch, ApiError>;
}

/// Where sandboxes put their session-private dirs, and whose home they hide.
#[derive(Clone, Debug)]
pub struct SandboxDirs {
    /// Each session's scratch dir is `<sessions>/<session id>`.
    pub sessions: PathBuf,
    /// The user's home.
    pub home: PathBuf,
}

/// What `session.create` asks for, with the branch name already chosen and the profile and
/// sandbox tier already checked.
#[derive(Clone, Debug)]
pub struct CreateRequest {
    pub project: Project,
    pub title: String,
    pub prompt: String,
    pub agent_profile: String,
    pub mode: Mode,
    pub sandbox: SandboxTier,
    pub branch: String,
    pub on_collision: Option<CollisionResolution>,
    pub max_concurrent: u32,
}

/// Per-session runtime state.
#[derive(Default)]
struct Live {
    /// Serializes state changes for the session.
    lock: tokio::sync::Mutex<()>,
    agent: Mutex<Option<Arc<AcpClient>>>,
}

/// Owns every session's lifecycle.
pub struct SessionManager {
    store: Arc<dyn SessionStore>,
    log: Arc<EventLog>,
    git: Arc<dyn Worktrees>,
    agents: Arc<dyn AgentResolver>,
    sandbox: Arc<dyn Sandbox>,
    dirs: SandboxDirs,
    live: Mutex<HashMap<SessionId, Arc<Live>>>,
    /// Serializes creates, so two sessions never claim the same branch name.
    create_lock: tokio::sync::Mutex<()>,
}

impl SessionManager {
    pub fn new(
        store: Arc<dyn SessionStore>,
        log: Arc<EventLog>,
        git: Arc<dyn Worktrees>,
        agents: Arc<dyn AgentResolver>,
        sandbox: Arc<dyn Sandbox>,
        dirs: SandboxDirs,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            log,
            git,
            agents,
            sandbox,
            dirs,
            live: Mutex::default(),
            create_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// After a daemon restart: sessions that were mid-turn become Paused and sessions that were
    /// provisioning become Failed, each with an event saying why. Nothing resumes on its own.
    pub async fn recover(self: &Arc<Self>) -> Result<(), SessionError> {
        use SessionState::*;
        let stale = self
            .blocking(|store| store.in_states(&[Provisioning, Working, Waiting]))
            .await?;
        for session in stale {
            let to = if session.state == Provisioning {
                Failed
            } else {
                Paused
            };
            let reason = "Interrupted by daemon restart";
            self.register_log(&session.id).await?;
            self.transition(&session, to, Some(reason.into())).await?;
            tracing::info!(session_id = %session.id, ?to, "{reason}");
        }
        Ok(())
    }

    /// What already holds `branch` in each of the project's repos.
    pub async fn preview(
        &self,
        project: &Project,
        branch: &str,
    ) -> Result<Vec<BranchCollision>, SessionError> {
        let names = self.inspect_names(project, branch).await?;
        Ok(names.iter().flat_map(RepoName::collisions).collect())
    }

    /// Record a new session and start provisioning it in the background.
    pub async fn create(self: &Arc<Self>, req: CreateRequest) -> Result<Session, SessionError> {
        if req.title.trim().is_empty() {
            return Err(ApiError::validation("title must not be empty").into());
        }
        let _creating = self.create_lock.lock().await;
        let names = self.inspect_names(&req.project, &req.branch).await?;
        let modes = names
            .iter()
            .map(|n| decide(n, &req.branch, req.on_collision))
            .collect::<Result<Vec<_>, _>>()?;

        let project_id = req.project.id.clone();
        let live = self
            .blocking(move |store| {
                let sessions = store.list(&project_id, None)?;
                Ok(sessions.iter().filter(|s| is_live(s.state)).count())
            })
            .await?;
        if live >= req.max_concurrent as usize {
            return Err(ApiError::new(
                ErrorKind::InvalidState,
                format!(
                    "the project already has {live} live sessions (sessions.max_concurrent = {})",
                    req.max_concurrent
                ),
            )
            .into());
        }

        let now = Timestamp::now();
        let session = Session {
            id: SessionId::new(),
            project_id: req.project.id.clone(),
            title: req.title.trim().to_owned(),
            agent_profile: req.agent_profile.clone(),
            state: SessionState::Provisioning,
            mode: req.mode,
            sandbox: req.sandbox,
            branch: req.branch.clone(),
            created_at: now,
            updated_at: now,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
                cost_usd: None,
            },
        };
        let worktrees: Vec<WorktreeRecord> = names
            .iter()
            .map(|n| WorktreeRecord {
                repo: n.repo.clone(),
                path: n.worktree.clone(),
            })
            .collect();
        let (record, rows) = (session.clone(), worktrees.clone());
        self.blocking(move |store| store.insert(&record, &rows))
            .await?;
        self.register_log(&session.id).await?;
        drop(_creating);

        let plan: Vec<RepoPlan> = names
            .into_iter()
            .zip(modes)
            .zip(req.project.repos.iter())
            .map(|((name, mode), repo)| RepoPlan {
                repo: name.repo,
                path: name.path,
                base: repo.base_branch.clone(),
                mode,
            })
            .collect();
        let manager = self.clone();
        let (created, prompt) = (session.clone(), req.prompt);
        tokio::spawn(async move { manager.provision(created, plan, prompt).await });
        Ok(session)
    }

    pub async fn get(&self, id: &SessionId) -> Result<Session, SessionError> {
        let key = id.clone();
        self.blocking(move |store| store.get(&key))
            .await?
            .ok_or_else(|| ApiError::not_found(format!("session {id}")).into())
    }

    pub async fn list(
        &self,
        project: &ProjectId,
        states: Option<Vec<SessionState>>,
    ) -> Result<Vec<Session>, SessionError> {
        let project = project.clone();
        self.blocking(move |store| store.list(&project, states.as_deref()))
            .await
    }

    /// Start a turn with `text`; returns its ID once the turn has begun.
    pub async fn prompt(
        self: &Arc<Self>,
        id: &SessionId,
        text: String,
        source: TurnSource,
    ) -> Result<TurnId, SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        if !accepts_prompt(session.state) {
            return Err(invalid_state(&session, "take a prompt"));
        }
        self.register_log(id).await?;
        let turn = self.begin_turn(&session, text.clone(), source).await?;
        let manager = self.clone();
        let (session_id, turn_id) = (id.clone(), turn.clone());
        tokio::spawn(async move { manager.run_turn(session_id, turn_id, text).await });
        Ok(turn)
    }

    /// Ask the agent to stop the current turn.
    pub async fn cancel(&self, id: &SessionId) -> Result<(), SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        if session.state != SessionState::Working {
            return Err(invalid_state(&session, "be cancelled"));
        }
        let agent = lock(&live.agent).clone();
        let Some(agent) = agent else {
            return Err(ApiError::new(
                ErrorKind::InvalidState,
                format!("session {id}'s agent is still starting; try again"),
            )
            .retryable()
            .into());
        };
        agent
            .cancel()
            .map_err(|e| ApiError::new(ErrorKind::AgentUnavailable, e.to_string()).into())
    }

    /// Stop the agent and mark the session Done; its worktrees stay for review and cleanup.
    pub async fn close(&self, id: &SessionId, reason: &str) -> Result<(), SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        if !can_close(session.state) {
            return Err(invalid_state(&session, "close"));
        }
        let agent = lock(&live.agent).take();
        if let Some(agent) = agent {
            agent.shutdown().await;
        }
        self.register_log(id).await?;
        self.transition(&session, SessionState::Done, Some(reason.to_owned()))
            .await?;
        self.log.unregister(id);
        lock(&self.live).remove(id);
        Ok(())
    }

    /// Stop every agent, e.g. on daemon shutdown. Sessions keep their state.
    pub async fn shutdown(&self) {
        let agents: Vec<Arc<AcpClient>> = lock(&self.live)
            .values()
            .filter_map(|l| lock(&l.agent).take())
            .collect();
        for agent in agents {
            agent.shutdown().await;
        }
    }

    async fn provision(self: Arc<Self>, session: Session, plan: Vec<RepoPlan>, prompt: String) {
        let id = session.id.clone();
        let live = self.live(&id);
        let result = async {
            let _guard = live.lock.lock().await;
            if let Err(reason) = self.create_worktrees(&session, &plan).await {
                self.transition(&session, SessionState::Failed, Some(reason))
                    .await?;
                return Ok(false);
            }
            match self.launch(&session).await {
                Ok(agent) => *lock(&live.agent) = Some(agent),
                Err(reason) => {
                    self.transition(&session, SessionState::Failed, Some(reason))
                        .await?;
                    return Ok(false);
                }
            }
            self.transition(&session, SessionState::Ready, None).await?;
            Ok::<_, SessionError>(true)
        }
        .await;
        match result {
            Ok(true) if !prompt.trim().is_empty() => {
                if let Err(e) = self.prompt(&id, prompt, TurnSource::Human).await {
                    tracing::error!(session_id = %id, error = %e, "starting the first turn");
                }
            }
            Ok(_) => {}
            Err(e) => tracing::error!(session_id = %id, error = %e, "provisioning"),
        }
    }

    /// Create each repo's worktree; on failure remove the ones this call made and return why.
    async fn create_worktrees(&self, session: &Session, plan: &[RepoPlan]) -> Result<(), String> {
        let (git, branch, plan) = (self.git.clone(), session.branch.clone(), plan.to_vec());
        tokio::task::spawn_blocking(move || {
            let mut done: Vec<(&RepoPlan, PathBuf)> = Vec::new();
            for repo in &plan {
                match git.create(&repo.path, &branch, &repo.base, repo.mode) {
                    Ok(path) => done.push((repo, path)),
                    Err(e) => {
                        for (made, path) in done.into_iter().rev() {
                            if made.mode == CreateMode::Adopt {
                                continue;
                            }
                            let _ = git.remove(&made.path, &path, true);
                            if made.mode == CreateMode::New {
                                let _ = git.delete_branch(&made.path, &branch);
                            }
                        }
                        return Err(format!("creating the worktree in {}: {e}", repo.repo));
                    }
                }
            }
            Ok(())
        })
        .await
        .unwrap_or_else(|e| Err(format!("creating worktrees: {e}")))
    }

    /// Start the session's agent in its worktrees, inside its sandbox. Returns why it couldn't
    /// start, including a sandbox that failed with the backend's reason and fix.
    async fn launch(self: &Arc<Self>, session: &Session) -> Result<Arc<AcpClient>, String> {
        let (agents, store, git, sandbox, dirs, record) = (
            self.agents.clone(),
            self.store.clone(),
            self.git.clone(),
            self.sandbox.clone(),
            self.dirs.clone(),
            session.clone(),
        );
        let found = tokio::task::spawn_blocking(move || {
            let launch = agents.launch(&record).map_err(|e| e.detail().to_owned())?;
            let records = store.worktrees(&record.id).map_err(|e| e.to_string())?;
            // Unsandboxed sessions mount nothing, so their worktree links aren't checked.
            let mut worktrees = Vec::new();
            let sandboxed = record.sandbox != SandboxTier::Unsandboxed;
            for w in records.iter().filter(|_| sandboxed) {
                let repo = repo_of(&w.path, &record.branch)
                    .ok_or_else(|| format!("{} is not a session worktree", w.path.display()))?;
                let dirs = git.git_dirs(&repo, &w.path).map_err(|e| e.to_string())?;
                worktrees.push(WorktreeMount {
                    path: w.path.clone(),
                    git_common: dirs.common,
                    git_admin: dirs.admin,
                });
            }
            let roots = records
                .into_iter()
                .map(|w| Root {
                    name: w.repo,
                    path: w.path,
                })
                .collect();
            let workspace = Workspace::new(roots).map_err(|e| e.to_string())?;
            // Only sessions with secrets need TLS to the secrets' hosts.
            let upstream = if launch.secrets.is_empty() {
                None
            } else {
                let tls = Tls::shared()
                    .map_err(|e| format!("loading TLS roots for secret injection: {e}"))?;
                Some(tls as Arc<dyn Upstream>)
            };
            Ok::<_, String>((launch, worktrees, workspace, upstream))
        })
        .await
        .map_err(|e| e.to_string())??;
        let (mut launch, worktrees, workspace, upstream) = found;
        let secrets = std::mem::take(&mut launch.secrets);
        let proxy = self
            .start_proxy(session, launch.network.clone(), secrets, upstream)
            .await?;
        let record = session.clone();
        let setup = tokio::task::spawn_blocking(move || {
            let spec = Spec {
                tier: record.sandbox,
                worktrees,
                scratch: dirs.sessions.join(record.id.to_string()),
                home: dirs.home,
                read_only: launch.read_only,
                proxy: Some(proxy),
            };
            let sandbox = sandbox
                .prepare(spec)
                .map_err(|e| format!("starting the sandbox: {e}"))?;
            Ok::<_, String>((launch.command, workspace, sandbox))
        })
        .await
        .map_err(|e| e.to_string())??;
        let (command, workspace, sandbox) = setup;
        let sink = Arc::new(SessionSink {
            log: self.log.clone(),
            session: session.id.clone(),
        });
        let permissions = Arc::new(DenyAll {
            reason: NO_POLICY.into(),
        });
        AcpClient::start(command, workspace, sandbox, sink, permissions)
            .await
            .map(Arc::new)
            .map_err(|e| format!("starting the agent: {e}"))
    }

    /// Start the session's egress proxy, logging each connection as a `network` event. The
    /// sandbox keeps it running; it stops with the agent.
    async fn start_proxy(
        &self,
        session: &Session,
        allow: Allowlist,
        routes: Vec<SecretRoute>,
        upstream: Option<Arc<dyn Upstream>>,
    ) -> Result<Arc<Proxy>, String> {
        let (log, id) = (self.log.clone(), session.id.clone());
        let runtime = tokio::runtime::Handle::current();
        let record =
            move |c: Connection| {
                let (log, id) = (log.clone(), id.clone());
                runtime.spawn(async move {
                let body = NetworkBody {
                    host: c.host,
                    port: c.port,
                    verdict: if c.allowed { Verdict::Allow } else { Verdict::Deny },
                    reason: c.reason,
                    bytes_sent: c.bytes_sent,
                    bytes_received: c.bytes_received,
                    duration_ms: u64::try_from(c.duration.as_millis()).unwrap_or(u64::MAX),
                };
                if let Err(e) = log.emit(&id, None, EventPayload::Network(body)).await {
                    tracing::error!(session_id = %id, error = %e, "logging a network connection");
                }
            });
            };
        Proxy::start(ProxyConfig {
            allow,
            resolver: Arc::new(PublicDns),
            log: Arc::new(record),
            limits: Limits::default(),
            routes,
            upstream,
        })
        .await
        .map(Arc::new)
        .map_err(|e| format!("starting the egress proxy: {e}"))
    }

    /// Move to Working and record the turn's start and prompt. The caller holds the lock.
    async fn begin_turn(
        &self,
        session: &Session,
        text: String,
        source: TurnSource,
    ) -> Result<TurnId, SessionError> {
        let turn = TurnId::new();
        self.transition(session, SessionState::Working, None)
            .await?;
        self.emit(
            &session.id,
            EventPayload::TurnStarted(TurnStartedBody {
                turn_id: turn.clone(),
                source,
            }),
        )
        .await?;
        self.emit(
            &session.id,
            EventPayload::Prompt(PromptBody {
                turn_id: turn.clone(),
                text,
            }),
        )
        .await?;
        Ok(turn)
    }

    async fn run_turn(self: Arc<Self>, id: SessionId, turn: TurnId, text: String) {
        if let Err(e) = self.turn(&id, &turn, text).await {
            tracing::error!(session_id = %id, error = %e, "running a turn");
        }
    }

    async fn turn(
        self: &Arc<Self>,
        id: &SessionId,
        turn: &TurnId,
        text: String,
    ) -> Result<(), SessionError> {
        let live = self.live(id);
        let outcome = match self.agent(id, &live).await {
            Ok(agent) => agent.prompt(turn.clone(), text).await.map_err(|e| {
                if matches!(e, AcpError::Exited(_)) {
                    // The process is gone; the next prompt starts a new one.
                    lock(&live.agent).take();
                }
                e.to_string()
            }),
            Err(reason) => Err(reason),
        };
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        let stop_reason = outcome.as_deref().unwrap_or("error").to_owned();
        self.emit(
            id,
            EventPayload::TurnEnded(TurnEndedBody {
                turn_id: turn.clone(),
                stop_reason: stop_reason.clone(),
            }),
        )
        .await?;
        let (to, reason) = match outcome {
            Ok(stop) if stop == "end_turn" => (SessionState::Review, None),
            Ok(stop) => (SessionState::Ready, Some(format!("turn ended: {stop}"))),
            Err(reason) => {
                self.emit(
                    id,
                    EventPayload::Error(ErrorData {
                        kind: ErrorKind::AgentUnavailable,
                        detail: reason.clone(),
                        retryable: true,
                    }),
                )
                .await?;
                (SessionState::Paused, Some(reason))
            }
        };
        if session.state == SessionState::Working {
            self.transition(&session, to, reason).await?;
        }
        Ok(())
    }

    /// The session's running agent, starting one if there is none or it has exited.
    async fn agent(
        self: &Arc<Self>,
        id: &SessionId,
        live: &Live,
    ) -> Result<Arc<AcpClient>, String> {
        let current = lock(&live.agent).clone();
        if let Some(agent) = current
            && agent.exit_watch().borrow().is_none()
        {
            return Ok(agent);
        }
        let session = self.get(id).await.map_err(|e| e.to_string())?;
        let agent = self.launch(&session).await?;
        *lock(&live.agent) = Some(agent.clone());
        Ok(agent)
    }

    /// Change state, persist it, and record the change. The caller holds the session's lock.
    async fn transition(
        &self,
        session: &Session,
        to: SessionState,
        reason: Option<String>,
    ) -> Result<(), SessionError> {
        let from = session.state;
        if !can_transition(from, to) {
            return Err(SessionError::Internal(format!(
                "session {} cannot move from {from:?} to {to:?}",
                session.id
            )));
        }
        let id = session.id.clone();
        self.blocking(move |store| store.set_state(&id, to, Timestamp::now()))
            .await?;
        self.emit(
            &session.id,
            EventPayload::SessionState(SessionStateBody { from, to, reason }),
        )
        .await?;
        Ok(())
    }

    async fn emit(&self, id: &SessionId, payload: EventPayload) -> Result<(), SessionError> {
        self.log.emit(id, None, payload).await?;
        Ok(())
    }

    async fn register_log(&self, id: &SessionId) -> Result<(), SessionError> {
        let (log, id) = (self.log.clone(), id.clone());
        tokio::task::spawn_blocking(move || log.register(&id))
            .await
            .map_err(|e| SessionError::Internal(e.to_string()))??;
        Ok(())
    }

    /// How `branch` is used in each of the project's repos.
    async fn inspect_names(
        &self,
        project: &Project,
        branch: &str,
    ) -> Result<Vec<RepoName>, SessionError> {
        let (git, store) = (self.git.clone(), self.store.clone());
        let (project, branch) = (project.clone(), branch.to_owned());
        tokio::task::spawn_blocking(move || {
            let root = PathBuf::from(&project.root);
            let mut names = Vec::new();
            for repo in &project.repos {
                let path = git.resolve_repo(&root, &repo.path).map_err(repo_error)?;
                git.validate_branch(&path, &branch).map_err(repo_error)?;
                let used = git.inspect(&path, &branch).map_err(repo_error)?;
                let worktree = worktree_path(&path, &branch);
                let owner = store.live_owner(&worktree)?;
                names.push(RepoName {
                    repo: repo.name.clone(),
                    path,
                    worktree,
                    used,
                    owner,
                });
            }
            Ok(names)
        })
        .await
        .map_err(|e| SessionError::Internal(e.to_string()))?
    }

    fn live(&self, id: &SessionId) -> Arc<Live> {
        lock(&self.live).entry(id.clone()).or_default().clone()
    }

    async fn blocking<T, F>(&self, f: F) -> Result<T, SessionError>
    where
        T: Send + 'static,
        F: FnOnce(&dyn SessionStore) -> Result<T, StoreError> + Send + 'static,
    {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || f(store.as_ref()))
            .await
            .map_err(|e| SessionError::Internal(e.to_string()))?
            .map_err(SessionError::from)
    }
}

/// One repo's part in provisioning.
#[derive(Clone, Debug)]
struct RepoPlan {
    repo: String,
    /// The repository root.
    path: PathBuf,
    base: String,
    mode: CreateMode,
}

/// Appends a session's ACP events to the event log.
struct SessionSink {
    log: Arc<EventLog>,
    session: SessionId,
}

impl EventSink for SessionSink {
    fn emit(&self, payload: EventPayload) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Err(e) = self.log.emit(&self.session, None, payload).await {
                tracing::error!(session_id = %self.session, error = %e, "appending an agent event");
            }
        })
    }
}

fn repo_error(e: GitError) -> SessionError {
    match e {
        GitError::Command { .. } | GitError::Io { .. } => SessionError::Internal(e.to_string()),
        other => ApiError::validation(other.to_string()).into(),
    }
}

fn invalid_state(session: &Session, what: &str) -> SessionError {
    ApiError::new(
        ErrorKind::InvalidState,
        format!(
            "session {} is {:?} and can't {what}",
            session.id, session.state
        ),
    )
    .into()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The repo a session worktree belongs to: `path` is `<repo>/.worktrees/<branch>`.
fn repo_of(path: &Path, branch: &str) -> Option<PathBuf> {
    let depth = Path::new(branch).components().count();
    let repo = path.ancestors().nth(depth + 1)?.to_path_buf();
    (worktree_path(&repo, branch) == path).then_some(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_of_strips_the_worktree_suffix_only() {
        let wt = Path::new("/p/repo/.worktrees/feat/x");
        assert_eq!(repo_of(wt, "feat/x"), Some(PathBuf::from("/p/repo")));
        assert_eq!(
            repo_of(Path::new("/p/repo/.worktrees/x"), "x"),
            Some("/p/repo".into())
        );
        assert_eq!(repo_of(wt, "x"), None);
        assert_eq!(
            repo_of(Path::new("/p/repo/elsewhere/feat/x"), "feat/x"),
            None
        );
    }
}
