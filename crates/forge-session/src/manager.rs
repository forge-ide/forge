//! The session manager: creates sessions, runs their turns, and keeps their state.
//!
//! Each session's state lives in the store and changes only through [`SessionManager`], one
//! change at a time per session, with a `session.state` event for each. Turns and provisioning
//! run as background tasks; the agent connection outlives individual turns and is relaunched
//! when a prompt arrives and it is gone (after a crash or a daemon restart).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, MutexGuard};

use forge_acp::logins::LoginKind;
use forge_acp::shim::{self, HomeConfig, ShimContext};
use forge_acp::{AcpClient, AcpError, AgentCommand, EventSink, Pipe, Root, Workspace, adapters};
use forge_api::events::{
    EventPayload, PromptBody, SessionStateBody, TurnEndedBody, TurnSource, TurnStartedBody,
};
use forge_api::events::{HookRunBody, NetworkBody, TerminalOutputBody, Verdict};
use forge_api::methods::{BranchCollision, CollisionResolution};
use forge_api::model::{Action, Mode, Project, SandboxTier, Session, SessionState, Usage};
use forge_api::{
    ApiError, ApprovalId, ErrorData, ErrorKind, ProjectId, SessionId, Timestamp, TurnId,
};
use forge_config::{PolicyRule, RuleListName};
use forge_events::EventLog;
use forge_git::{CreateMode, GitError, Launch, Worktrees, worktree_path};
use forge_hooks::{HookContext, HookLog, HookRunner, HookSet, ShellHooks, TaskCheck};
use forge_policy::{Engine, NoClassifier, Policy, Rules, SecretScanner};
use forge_sandbox::proxy::{
    Allowlist, Connection, Limits, Proxy, ProxyConfig, PublicDns, SecretRoute, Tls, Upstream,
};
use forge_sandbox::{Sandbox, Spec, WorktreeMount};
use futures_util::future::BoxFuture;

use crate::naming::{RepoName, decide};
use crate::redact::{Holdback, SecretRedactor};
use crate::state::{accepts_prompt, can_close, can_transition, is_live};
use crate::store::{SessionStore, StoreError, WorktreeRecord};

mod approvals;
mod commit;

use approvals::{Pending, SessionGate};
use commit::CommitTool;

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
    /// The registry agent it runs, whose shim prepares it; `None` for any other agent.
    pub agent: Option<String>,
    /// Paths the sandbox shows read-only besides its defaults: the agent program and the
    /// user config's extra read-only paths.
    pub read_only: Vec<PathBuf>,
    /// Hosts the session's egress proxy lets through.
    pub network: Allowlist,
    /// Secrets the proxy injects, with their values from the keyring or the agent's login.
    pub secrets: Vec<SecretRoute>,
    /// The agent's own sign-in the proxy carries, when it isn't an API key.
    pub login: Option<LoginKind>,
    /// What the agent reads from the user's own config; its links' targets are among
    /// `read_only`.
    pub home: HomeConfig,
    /// The managed, user, and project policy layers, compiled.
    pub rules: Arc<Rules>,
    /// The project's hooks, compiled.
    pub hooks: Arc<HookSet>,
    /// The project root, for hooks' input.
    pub project_root: PathBuf,
}

/// Finds how to start a session's agent from its profile and the policy files, and records
/// decisions remembered for the project. The daemon implements it from config.
pub trait AgentResolver: Send + Sync + 'static {
    /// Blocking; may read config files.
    fn launch(&self, session: &Session) -> Result<AgentLaunch, ApiError>;
    /// The values of the secrets the session's proxy injects, so the event log can redact
    /// them before the agent is launched (after a daemon restart). Blocking.
    fn secret_values(&self, session: &Session) -> Result<Vec<String>, ApiError>;
    /// Append `rule` to the session's project policy file. Blocking.
    fn remember_in_project(
        &self,
        session: &Session,
        list: RuleListName,
        rule: PolicyRule,
    ) -> Result<(), ApiError>;
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
    /// The running agent's policy engine, rebuilt from the policy files at each launch.
    policy: Mutex<Option<Arc<Engine>>>,
    /// Decisions remembered for the session, replayed into each new engine.
    remembered: Mutex<Vec<(Action, forge_api::events::Verdict)>>,
    /// Asks waiting in the inbox.
    pending: Mutex<HashMap<ApprovalId, Pending>>,
    /// Actions a human allowed in a permission request, each good for one matching write or
    /// terminal in the same turn.
    grants: Mutex<Vec<Action>>,
    /// For the agent with the next prompt: approvals a daemon restart cancelled.
    resume_note: Mutex<Option<String>>,
    /// The running agent's hooks.
    hooks: Mutex<Option<Arc<dyn HookRunner>>>,
    /// The running agent's event sink, whose held-back stream text is flushed when a turn
    /// ends.
    sink: Mutex<Option<Arc<SessionSink>>>,
    /// Hook notes for the agent's next prompt, from writes and allowed actions, whose
    /// responses have no room for them.
    notes: Mutex<Vec<String>>,
    /// `task.done` blocks in a row since the human last prompted.
    hook_rounds: Mutex<u32>,
    /// Set by `session.cancel`, so a turn that ends while `task.done` hooks run doesn't start
    /// another.
    cancel_requested: std::sync::atomic::AtomicBool,
    /// Wakes a turn waiting on `task.done` hooks when `session.cancel` arrives, which stops
    /// them.
    cancel_hooks: tokio::sync::Notify,
    /// Instructions for the running agent's first prompt, for agents that take them no other
    /// way.
    preamble: Mutex<Option<&'static str>>,
}

/// `task.done` blocks in a row before the session goes to Review anyway.
const MAX_HOOK_ROUNDS: u32 = 3;

/// Owns every session's lifecycle.
pub struct SessionManager {
    store: Arc<dyn SessionStore>,
    log: Arc<EventLog>,
    git: Arc<dyn Worktrees>,
    agents: Arc<dyn AgentResolver>,
    sandbox: Arc<dyn Sandbox>,
    dirs: SandboxDirs,
    /// The event log's redactor, told each session's secrets at launch.
    redactor: Arc<SecretRedactor>,
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
        redactor: Arc<SecretRedactor>,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            log,
            git,
            agents,
            sandbox,
            dirs,
            redactor,
            live: Mutex::default(),
            create_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// After a daemon restart: sessions that were mid-turn become Paused and sessions that were
    /// provisioning become Failed, each with an event saying why, and pending approvals are
    /// cancelled, which the agent hears with its next prompt. Nothing resumes on its own.
    pub async fn recover(self: &Arc<Self>) -> Result<(), SessionError> {
        use SessionState::*;
        self.cancel_stale_approvals().await?;
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
        self.know_secrets(&session).await?;
        *lock(&live.hook_rounds) = 0;
        live.cancel_requested.store(false, Ordering::SeqCst);
        let text = agent_text(&live, text);
        let turn = self.begin_turn(&session, text.clone(), source).await?;
        let manager = self.clone();
        let (session_id, turn_id) = (id.clone(), turn.clone());
        tokio::spawn(async move { manager.run_turn(session_id, turn_id, text).await });
        Ok(turn)
    }

    /// Make sure the log redacts the session's injected secrets before anything the human
    /// types is recorded: after a daemon restart, the agent and its scanner aren't there yet.
    /// If they can't be resolved the prompt fails, as the launch after it would.
    async fn know_secrets(&self, session: &Session) -> Result<(), SessionError> {
        if self.redactor.has(&session.id) {
            return Ok(());
        }
        let (agents, record) = (self.agents.clone(), session.clone());
        let values = tokio::task::spawn_blocking(move || agents.secret_values(&record))
            .await
            .map_err(|e| SessionError::Internal(e.to_string()))??;
        self.redactor
            .set(&session.id, Arc::new(SecretScanner::new(values)));
        Ok(())
    }

    /// Ask the agent to stop the current turn; anything waiting for approval is cancelled.
    pub async fn cancel(&self, id: &SessionId) -> Result<(), SessionError> {
        let live = self.live(id);
        let _guard = live.lock.lock().await;
        let session = self.get(id).await?;
        if !matches!(session.state, SessionState::Working | SessionState::Waiting) {
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
        live.cancel_requested.store(true, Ordering::SeqCst);
        live.cancel_hooks.notify_waiters();
        agent
            .cancel()
            .map_err(|e| ApiError::new(ErrorKind::AgentUnavailable, e.to_string()))?;
        self.cancel_pending(&live, id).await
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
        let sink = lock(&live.sink).take();
        if let Some(sink) = sink {
            sink.release_all().await;
        }
        self.transition(&session, SessionState::Done, Some(reason.to_owned()))
            .await?;
        // The copies of the user's agent settings don't outlive the session.
        let scratch = self.dirs.sessions.join(id.to_string());
        let cleared = tokio::task::spawn_blocking(move || {
            shim::clear_home(&scratch.join("home"))?;
            match std::fs::remove_dir_all(scratch.join("shim")) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            }
        })
        .await;
        if let Ok(Err(e)) = cleared {
            tracing::warn!(session_id = %id, error = %e, "clearing the agent's home config");
        }
        // The session's secret values go only once nothing more can be recorded.
        self.log.unregister(id);
        self.redactor.remove(id);
        lock(&self.live).remove(id);
        Ok(())
    }

    /// Stop every agent, e.g. on daemon shutdown, recording what their streams held back.
    /// Sessions keep their state.
    pub async fn shutdown(&self) {
        let lives: Vec<Arc<Live>> = lock(&self.live).values().cloned().collect();
        for live in lives {
            let agent = lock(&live.agent).take();
            if let Some(agent) = agent {
                agent.shutdown().await;
            }
            let sink = lock(&live.sink).take();
            if let Some(sink) = sink {
                sink.release_all().await;
            }
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
        // The index commits are staged in sits in the session's scratch dir, which only the
        // daemon writes (the sandbox gets only its home and temp dirs).
        let index = forge_git::commit::index_path(&self.dirs.sessions.join(session.id.to_string()));
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
            // Commits go on the branch from the working directory's worktree.
            let first = workspace.cwd().to_path_buf();
            let repo = repo_of(&first, &record.branch)
                .ok_or_else(|| format!("{} is not a session worktree", first.display()))?;
            let target = commit::Target {
                dirs: git.git_dirs(&repo, &first).map_err(|e| e.to_string())?,
                worktree: first,
                branch: record.branch.clone(),
                index,
            };
            // Only sessions with secrets need TLS to the secrets' hosts.
            let upstream = if launch.secrets.is_empty() {
                None
            } else {
                let tls = Tls::shared()
                    .map_err(|e| format!("loading TLS roots for secret injection: {e}"))?;
                Some(tls as Arc<dyn Upstream>)
            };
            Ok::<_, String>((launch, worktrees, workspace, upstream, target))
        })
        .await
        .map_err(|e| e.to_string())??;
        let (mut launch, worktrees, workspace, upstream, mut target) = found;
        let secrets = std::mem::take(&mut launch.secrets);
        let (rules, hook_set) = (launch.rules.clone(), launch.hooks.clone());
        let project_root = launch.project_root.clone();
        // A login's value as it is now; one its CLI refreshes later never enters the sandbox.
        let sources: Vec<_> = secrets.iter().map(|s| s.value.clone()).collect();
        let mut values: Vec<String> = tokio::task::spawn_blocking(move || {
            sources
                .iter()
                .filter_map(|source| source.current().ok())
                .map(|value| value.expose().to_owned())
                .collect()
        })
        .await
        .map_err(|e| e.to_string())?;
        // Forge's MCP endpoint is served on the proxy; its tool is filled in once the gate
        // exists, below.
        let tools = Arc::new(CommitTool::default());
        let proxy = self
            .start_proxy(
                session,
                launch.network.clone(),
                secrets,
                upstream,
                tools.clone(),
            )
            .await?;
        // Commands and edits holding an injected secret's value ask, and the value never
        // reaches the log; nor does the proxy's token, which agents log in their config.
        values.push(proxy.token().to_owned());
        values.extend(launch.home.secrets.iter().cloned());
        let scanner = Arc::new(SecretScanner::new(values));
        target.dirs.launch = self.git_launch(session, &target.dirs, &scanner).await;
        let record = session.clone();
        let manifest = launch.agent.as_deref().and_then(adapters::manifest);
        let repo_git = (
            target.dirs.clone(),
            target.worktree.clone(),
            target.index.clone(),
            target.branch.clone(),
        );
        let setup = tokio::task::spawn_blocking(move || {
            // The agent's repo config as the branch committed it, read before anything runs.
            let (git_dirs, worktree, index, branch) = repo_git;
            let committed = manifest
                .map(|m| shim::repo_config(&m.id))
                .unwrap_or_default()
                .iter()
                .map(|path| {
                    forge_git::commit::committed_file(&git_dirs, &worktree, &index, &branch, path)
                        .map(|c| ((*path).to_owned(), c))
                        .map_err(|e| format!("reading {path} from the branch: {e}"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let scratch = dirs.sessions.join(record.id.to_string());
            // A fresh dir for the shim's files each launch, shown read-only: the agent can't
            // change what its own settings say, or plant anything the daemon would follow.
            let shim_dir = scratch.join("shim");
            match std::fs::remove_dir_all(&shim_dir) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(format!("clearing the agent's shim dir: {e}")),
            }
            std::fs::create_dir_all(&scratch)
                .and_then(|()| std::fs::create_dir(&shim_dir))
                .map_err(|e| format!("creating the agent's shim dir: {e}"))?;
            // Agents' own config in the sandbox home starts afresh each launch, from the user's.
            // Nothing runs in the session now; removing a directory never follows a symlink
            // inside it.
            let home = scratch.join("home");
            shim::clear_home(&home).map_err(|e| e.to_string())?;
            let mut read_only = launch.read_only;
            read_only.push(shim_dir.clone());
            // The session's own copy of the agent's config, inside the read-only shim dir.
            let staged = shim::stage(&launch.home, &shim_dir, &worktree)
                .map_err(|e| format!("copying the agent's login into the session: {e}"))?;
            read_only.extend(staged.read_only);
            let spec = Spec {
                tier: record.sandbox,
                worktrees,
                scratch,
                home: dirs.home,
                read_only,
                read_write: staged.read_write,
                home_locked: shim::home_locked(manifest),
                proxy: Some(proxy.clone()),
            };
            let sandbox = sandbox
                .prepare(spec)
                .map_err(|e| format!("starting the sandbox: {e}"))?;
            // The sandbox has made its home by now.
            let seen_as = sandbox
                .var("HOME")
                .map_or_else(|| home.clone(), PathBuf::from);
            shim::install_home(&launch.home, &home, &seen_as)
                .map_err(|e| format!("setting up the agent's config: {e}"))?;
            let var = |name: &str| sandbox.var(name);
            let mcp_url = sandbox.var("HTTP_PROXY").map(|url| proxy.local_url(&url));
            let prepared = shim::prepare(
                manifest,
                &ShimContext {
                    dir: &shim_dir,
                    mcp_url,
                    var: &var,
                    home: &launch.home,
                    login: launch.login,
                    worktree: &worktree,
                    committed: &committed,
                },
            )
            .map_err(|e| e.to_string())?;
            let mut command = launch.command;
            command.argv.extend(prepared.args);
            command.env.extend(prepared.env);
            Ok::<_, String>((command, prepared.setup, workspace, sandbox))
        })
        .await
        .map_err(|e| e.to_string())??;
        let (command, setup, workspace, sandbox) = setup;
        // A relaunch: what the old agent's streams held is recorded under the scanner it was
        // held with, before the new one takes over.
        let live = self.live(&session.id);
        let old = lock(&live.sink).take();
        if let Some(old) = old {
            old.release_all().await;
        }
        self.redactor.set(&session.id, scanner.clone());
        let sink = Arc::new(SessionSink {
            log: self.log.clone(),
            session: session.id.clone(),
            holdback: Holdback::new(scanner.clone()),
        });
        let hooks: Arc<dyn HookRunner> = Arc::new(ShellHooks::new(
            hook_set,
            HookContext {
                session: session.id.clone(),
                project_root,
                worktree: workspace.cwd().to_path_buf(),
                sandbox: sandbox.clone(),
                log: sink.clone(),
            },
        ));
        let engine = Arc::new(Engine::new(
            rules,
            scanner.clone(),
            hooks.clone(),
            Arc::new(NoClassifier),
        ));
        *lock(&live.hooks) = Some(hooks.clone());
        *lock(&live.sink) = Some(sink.clone());
        for (action, verdict) in lock(&live.remembered).iter() {
            engine.remember(action, *verdict);
        }
        *lock(&live.policy) = Some(engine.clone());
        let gate = Arc::new(SessionGate {
            manager: Arc::downgrade(self),
            session: session.id.clone(),
            live: live.clone(),
            engine,
            hooks,
            scanner,
            sink: sink.clone(),
        });
        tools.set(gate.clone(), target);
        *lock(&live.preamble) = shim::first_prompt_preamble(manifest);
        let agent = manifest.map(|m| m.id.clone());
        AcpClient::start(command, setup, workspace, sandbox, sink, gate)
            .await
            .map(Arc::new)
            .map_err(|e| match (e, agent) {
                (AcpError::AuthRequired { methods }, Some(agent)) => {
                    let ways: Vec<&str> = methods.iter().map(|m| m.name.as_str()).collect();
                    format!(
                        "the agent isn't signed in; sign in with `forge agent sign-in {agent}` \
                         ({})",
                        ways.join(", ")
                    )
                }
                (e, _) => format!("starting the agent: {e}"),
            })
    }

    /// Start the session's egress proxy, logging each connection as a `network` event. The
    /// sandbox keeps it running; it stops with the agent.
    async fn start_proxy(
        &self,
        session: &Session,
        allow: Allowlist,
        routes: Vec<SecretRoute>,
        upstream: Option<Arc<dyn Upstream>>,
        tools: Arc<CommitTool>,
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
            local: Some(Arc::new(forge_mcp::ForgeServer::new(tools))),
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
        self.transition(session, SessionState::Working, None)
            .await?;
        self.record_turn(&session.id, text, source).await
    }

    /// Record a turn's start and prompt. The caller holds the lock.
    async fn record_turn(
        &self,
        id: &SessionId,
        text: String,
        source: TurnSource,
    ) -> Result<TurnId, SessionError> {
        let turn = TurnId::new();
        self.emit(
            id,
            EventPayload::TurnStarted(TurnStartedBody {
                turn_id: turn.clone(),
                source,
            }),
        )
        .await?;
        self.emit(
            id,
            EventPayload::Prompt(PromptBody {
                turn_id: turn.clone(),
                text,
            }),
        )
        .await?;
        Ok(turn)
    }

    /// Boxed, since a turn a `task.done` hook blocked starts the next one.
    fn run_turn(
        self: Arc<Self>,
        id: SessionId,
        turn: TurnId,
        text: String,
    ) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            if let Err(e) = self.turn(&id, &turn, text).await {
                tracing::error!(session_id = %id, error = %e, "running a turn");
            }
        })
    }

    async fn turn(
        self: &Arc<Self>,
        id: &SessionId,
        turn: &TurnId,
        text: String,
    ) -> Result<(), SessionError> {
        let live = self.live(id);
        let outcome = match self.agent(id, &live).await {
            Ok(agent) => {
                let text = match lock(&live.preamble).take() {
                    Some(preamble) => format!("{preamble}\n\n{text}"),
                    None => text,
                };
                agent.prompt(turn.clone(), text).await.map_err(|e| {
                    if matches!(e, AcpError::Exited(_)) {
                        // The process is gone; the next prompt starts a new one.
                        lock(&live.agent).take();
                    }
                    e.to_string()
                })
            }
            Err(reason) => Err(reason),
        };
        // task.done hooks may take minutes, so they run before the lock is taken.
        // session.cancel stops them; dropping the run kills the hook.
        let hooks = lock(&live.hooks).clone();
        let mut interrupted = false;
        let check = match (&outcome, hooks) {
            (Ok(stop), Some(hooks)) if stop == "end_turn" => {
                let mode = self.get(id).await?.mode;
                let cancelled = live.cancel_hooks.notified();
                tokio::pin!(cancelled);
                cancelled.as_mut().enable();
                if live.cancel_requested.load(Ordering::SeqCst) {
                    interrupted = true;
                    None
                } else {
                    tokio::select! {
                        check = hooks.task_done(mode) => Some(check),
                        () = cancelled => {
                            interrupted = true;
                            None
                        }
                    }
                }
            }
            _ => None,
        };
        let _guard = live.lock.lock().await;
        // Whatever still waits can't be answered once the turn is over.
        self.cancel_pending(&live, id).await?;
        lock(&live.grants).clear();
        let session = self.get(id).await?;
        let sink = lock(&live.sink).clone();
        if let Some(sink) = sink {
            sink.end_turn().await;
        }
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
            Ok(stop) if stop == "end_turn" && interrupted => {
                live.cancel_requested.store(false, Ordering::SeqCst);
                (
                    SessionState::Ready,
                    Some("turn ended: cancelled".to_owned()),
                )
            }
            Ok(stop) if stop == "end_turn" => match check {
                Some(TaskCheck::Blocked { hook, reason })
                    if session.state == SessionState::Working =>
                {
                    if live.cancel_requested.swap(false, Ordering::SeqCst) {
                        (
                            SessionState::Ready,
                            Some("turn ended: cancelled".to_owned()),
                        )
                    } else {
                        let rounds = {
                            let mut rounds = lock(&live.hook_rounds);
                            *rounds += 1;
                            *rounds
                        };
                        if rounds < MAX_HOOK_ROUNDS {
                            // Back to work, staying in Working, with what the hook said.
                            let text = agent_text(&live, reason);
                            let next = self.record_turn(id, text.clone(), TurnSource::Hook).await?;
                            let (manager, id) = (self.clone(), id.clone());
                            tokio::spawn(manager.run_turn(id, next, text));
                            return Ok(());
                        }
                        *lock(&live.hook_rounds) = 0;
                        (
                            SessionState::Review,
                            Some(format!(
                                "the task.done hook {hook} still blocked after {rounds} rounds: {reason}"
                            )),
                        )
                    }
                }
                _ => (SessionState::Review, None),
            },
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

    /// How the session's git commands that open its worktree run (see [`git_confinement`]).
    /// Confined git reads a snapshot of the user's git config, written to the session's
    /// daemon-owned dir, without values the session's scanner flags as secrets.
    async fn git_launch(
        &self,
        session: &Session,
        dirs: &forge_git::GitDirs,
        scanner: &Arc<SecretScanner>,
    ) -> Launch {
        let confine = match git_confinement(session.sandbox, self.sandbox.as_ref()) {
            Ok(None) => {
                return match forge_sandbox::PlainGit::new() {
                    Ok(plain) => Launch::Plain(Arc::new(plain)),
                    Err(why) => Launch::Refused(why.to_string()),
                };
            }
            Ok(Some(confine)) => confine,
            Err(why) => return Launch::Refused(why),
        };
        let config = self
            .dirs
            .sessions
            .join(session.id.to_string())
            .join("git-config");
        let home = self.dirs.home.clone();
        let xdg = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        let (dirs, scanner, id) = (dirs.clone(), scanner.clone(), session.id.clone());
        let built = tokio::task::spawn_blocking(move || {
            let looks_secret = |value: &str| !scanner.scan(value).is_empty();
            let snap = forge_git::confine::snapshot(&dirs, &home, &xdg, &looks_secret)?;
            for key in &snap.dropped {
                tracing::warn!(session_id = %id, key, "left a git config setting that looks like it holds a secret out of the daemon's git");
            }
            forge_git::confine::write_snapshot(&snap, &config)?;
            Ok::<_, GitError>(forge_git::confine::Confined {
                confine,
                config,
                read: snap.read,
            })
        })
        .await;
        match built {
            Ok(Ok(confined)) => Launch::Confined(confined),
            Ok(Err(e)) => Launch::Refused(format!("reading the user's git config: {e}")),
            Err(e) => Launch::Refused(e.to_string()),
        }
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

/// `text` for the agent, after the notes it should hear first: approvals a restart cancelled,
/// and hook notes from writes and allowed actions.
fn agent_text(live: &Live, text: String) -> String {
    let mut parts = Vec::new();
    parts.extend(lock(&live.resume_note).take());
    let notes = std::mem::take(&mut *lock(&live.notes));
    if !notes.is_empty() {
        parts.push(format!(
            "Notes from the project's hooks:\n{}",
            notes.join("\n")
        ));
    }
    parts.push(text);
    parts.join("\n\n")
}

/// Appends a session's ACP events to the event log, which redacts them, holding back the end
/// of streamed chunks that might continue a secret in the next one.
struct SessionSink {
    log: Arc<EventLog>,
    session: SessionId,
    holdback: Holdback,
}

impl SessionSink {
    /// Record what messages held back; the turn is over. Terminals keep theirs until they end.
    async fn end_turn(&self) {
        for payload in self.holdback.end_turn() {
            self.append(payload).await;
        }
    }

    /// Record everything held back; the agent stopped.
    async fn release_all(&self) {
        for payload in self.holdback.release_all() {
            self.append(payload).await;
        }
    }

    async fn append(&self, payload: EventPayload) {
        if let Err(e) = self.log.emit(&self.session, None, payload).await {
            tracing::error!(session_id = %self.session, error = %e, "appending an agent event");
        }
    }
}

impl HookLog for SessionSink {
    fn ran(&self, run: HookRunBody) -> BoxFuture<'_, ()> {
        EventSink::emit(self, EventPayload::HookRun(run))
    }
}

impl EventSink for SessionSink {
    fn emit(&self, payload: EventPayload) -> BoxFuture<'_, ()> {
        let ready = self.holdback.take(payload);
        Box::pin(async move {
            for payload in ready {
                self.append(payload).await;
            }
        })
    }

    fn terminal_output(&self, pipe: Pipe, body: TerminalOutputBody) -> BoxFuture<'_, ()> {
        let ready = self.holdback.take_output(pipe, body);
        Box::pin(async move {
            for payload in ready {
                self.append(payload).await;
            }
        })
    }
}

/// The confinement for the session's git commands that open its worktree: `None` when the
/// session itself runs unsandboxed, whose agent can reach whatever the daemon's git could;
/// an error when this machine can't confine git, which then doesn't run at all.
fn git_confinement(
    tier: SandboxTier,
    sandbox: &dyn Sandbox,
) -> Result<Option<Arc<forge_sandbox::GitConfinement>>, String> {
    if tier == SandboxTier::Unsandboxed {
        return Ok(None);
    }
    sandbox
        .git_confinement()
        .map(Some)
        .map_err(|why| format!("this machine can't confine it ({why})"))
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

    /// A sandbox that can't confine git.
    struct NoConfinement;

    impl Sandbox for NoConfinement {
        fn tiers(&self) -> Vec<forge_sandbox::TierStatus> {
            vec![]
        }
        fn prepare(
            &self,
            _: forge_sandbox::Spec,
        ) -> Result<Arc<dyn forge_sandbox::SessionSandbox>, forge_sandbox::SandboxError> {
            unreachable!()
        }
        fn git_confinement(
            &self,
        ) -> Result<Arc<forge_sandbox::GitConfinement>, forge_sandbox::Unavailable> {
            Err(forge_sandbox::Unavailable {
                reason: "no Landlock".into(),
                fix: "boot with landlock".into(),
            })
        }
    }

    #[test]
    fn git_runs_confined_or_not_at_all_unless_the_session_is_unsandboxed() {
        assert!(matches!(
            git_confinement(SandboxTier::Unsandboxed, &NoConfinement),
            Ok(None)
        ));
        for tier in [SandboxTier::Light, SandboxTier::Standard] {
            match git_confinement(tier, &NoConfinement) {
                Err(why) => assert!(why.contains("no Landlock"), "{why}"),
                other => panic!("{other:?}"),
            }
        }
    }

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
