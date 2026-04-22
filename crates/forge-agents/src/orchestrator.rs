//! Runtime orchestration for `forge-agents` (F-133).
//!
//! Provides:
//! - [`AgentInstance`] — one live instantiation of an [`AgentDef`] with a
//!   registry-assigned [`AgentInstanceId`].
//! - [`Orchestrator`] — registers instances, emits per-instance lifecycle
//!   events on a broadcast channel, enforces runtime isolation invariants.
//! - [`AgentEvent`] — the event vocabulary consumers subscribe to via
//!   [`Orchestrator::state_stream`].
//!
//! This layer is the foundation; sub-agent spawning (F-134), AGENTS.md
//! injection (F-135), UI banners (F-136), background agents (F-137), and the
//! session-side `AgentMonitor` (F-140) all compose on top.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use forge_core::AgentInstanceId;
use tokio::sync::{broadcast, Mutex};
use tokio_stream::wrappers::BroadcastStream;

use crate::def::{AgentDef, Isolation};
use crate::error::{Error, Result};

/// Type alias so hot-path field clones stay cheap (refcount bump), matching
/// the `Arc<str>` contract on `forge_core::Event::UserMessage.text`.
pub type InitialPrompt = Arc<str>;

/// Runtime lifecycle state of an [`AgentInstance`].
///
/// Distinct from [`AgentEvent`]: states are the *current* condition;
/// `StepStarted`/`StepFinished` are per-step *transitions* reported on the
/// event stream, not separate states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceState {
    /// Instance is active and has not yet reached a terminal state.
    Running,
    /// Instance finished successfully; terminal.
    Completed,
    /// Instance ended in error with the attached reason; terminal.
    Failed {
        /// Human-readable failure reason propagated from the spawner or step executor.
        reason: String,
    },
}

/// One live instantiation of an [`AgentDef`].
#[derive(Debug, Clone)]
pub struct AgentInstance {
    /// Registry-assigned identifier unique to this instance.
    pub id: AgentInstanceId,
    /// Parsed definition this instance was spawned from.
    pub def: AgentDef,
    /// Current lifecycle state; advances monotonically toward a terminal variant.
    pub state: InstanceState,
    /// Timestamp of spawn, set by the orchestrator at registration time.
    pub started_at: DateTime<Utc>,
    /// F-137 / F-134 follow-up: the seed user message this instance was
    /// spawned with. Threaded from the spawner (either `agent.spawn`'s
    /// parent-supplied arg for sub-agents, or `BackgroundAgentRegistry::start`'s
    /// user-supplied prompt) so the future step executor can materialise it
    /// as the child's first user turn. `None` only when the spawner had no
    /// seed to supply (legacy path used by the pre-F-134 scaffolding tests).
    pub initial_prompt: Option<InitialPrompt>,
}

/// Origin marker for a spawn. User-scope agents have the `Isolation::Trusted`
/// escape hatch removed at runtime; built-in skills keep it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentScope {
    /// User-authored agent; `Isolation::Trusted` is rejected at spawn time.
    #[default]
    User,
    /// Forge-shipped built-in skill; retains the `Isolation::Trusted` escape hatch.
    BuiltIn,
}

/// Context passed to [`Orchestrator::spawn`].
///
/// `scope` selects the origin-based policy the orchestrator applies (User-scope
/// rejects `Isolation::Trusted`). `initial_prompt` is the seed user message
/// the spawner wants the child's first turn to receive; `Orchestrator::spawn`
/// copies it onto the registered [`AgentInstance`] so the future step executor
/// picks it up verbatim when constructing the child's first user turn (F-134
/// follow-up wired here as the F-137 additional mandate).
#[derive(Debug, Clone, Default)]
pub struct SpawnContext {
    /// Origin policy applied to the spawn (User rejects `Isolation::Trusted`).
    pub scope: AgentScope,
    /// Optional seed user message forwarded to the child's first turn.
    pub initial_prompt: Option<InitialPrompt>,
}

impl SpawnContext {
    /// User-scope spawn with no seed prompt. Matches the pre-F-137 shape and
    /// keeps existing call sites (sub-agent test fixtures, `rerun_replace`)
    /// compiling without edits.
    pub fn user() -> Self {
        Self {
            scope: AgentScope::User,
            initial_prompt: None,
        }
    }
    /// Built-in-scope spawn with no seed prompt; used for Forge-shipped skills.
    pub fn built_in() -> Self {
        Self {
            scope: AgentScope::BuiltIn,
            initial_prompt: None,
        }
    }

    /// Attach a seed prompt to the context so the spawner can forward the
    /// parent-tool `prompt` arg (F-134 `agent.spawn`) or the user-supplied
    /// first message (F-137 background agents) to the child's first turn.
    pub fn with_prompt(mut self, prompt: InitialPrompt) -> Self {
        self.initial_prompt = Some(prompt);
        self
    }
}

/// Lifecycle events emitted on the per-orchestrator broadcast stream.
///
/// Terminal events: `Completed`, `Failed`. Before the terminal event a given
/// instance may emit any number of `StepStarted` / `StepFinished` pairs.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    /// Instance was registered and admitted to the registry.
    Spawned {
        /// Instance the event refers to.
        id: AgentInstanceId,
        /// Event wall-clock timestamp.
        at: DateTime<Utc>,
    },
    /// A named step within the instance has started executing.
    StepStarted {
        /// Instance the event refers to.
        id: AgentInstanceId,
        /// Step label emitted by the step executor.
        step: String,
        /// Event wall-clock timestamp.
        at: DateTime<Utc>,
    },
    /// A named step within the instance has finished.
    StepFinished {
        /// Instance the event refers to.
        id: AgentInstanceId,
        /// Step label emitted by the step executor.
        step: String,
        /// Event wall-clock timestamp.
        at: DateTime<Utc>,
    },
    /// Terminal: instance completed successfully.
    Completed {
        /// Instance the event refers to.
        id: AgentInstanceId,
        /// Event wall-clock timestamp.
        at: DateTime<Utc>,
    },
    /// Terminal: instance ended in error with the attached reason.
    Failed {
        /// Instance the event refers to.
        id: AgentInstanceId,
        /// Human-readable failure reason.
        reason: String,
        /// Event wall-clock timestamp.
        at: DateTime<Utc>,
    },
}

/// Channel capacity for the broadcast stream. Matches `forge-session`'s event
/// bus (32) — enough headroom for a burst of step events without a single
/// slow consumer dropping lifecycle signals.
const EVENT_BUS_CAPACITY: usize = 64;

/// Registers [`AgentInstance`]s, drives their lifecycle, and forwards
/// per-instance events to subscribers.
pub struct Orchestrator {
    registry: Arc<Mutex<HashMap<AgentInstanceId, AgentInstance>>>,
    tx: broadcast::Sender<AgentEvent>,
}

impl Orchestrator {
    /// Build an orchestrator with an empty registry and a fresh event bus.
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(EVENT_BUS_CAPACITY);
        Self {
            registry: Arc::new(Mutex::new(HashMap::new())),
            tx,
        }
    }

    /// Instantiate an agent from its definition.
    ///
    /// Rejects `isolation: trusted` under [`AgentScope::User`] with a typed
    /// [`Error::IsolationViolation`]. This re-enforces the parse-time check
    /// for programmatically-constructed defs that bypass the parser.
    pub async fn spawn(&self, def: AgentDef, ctx: SpawnContext) -> Result<AgentInstance> {
        if ctx.scope == AgentScope::User && def.isolation == Isolation::Trusted {
            return Err(Error::IsolationViolation {
                name: def.name,
                path: None,
            });
        }

        let now = Utc::now();
        let instance = AgentInstance {
            id: AgentInstanceId::new(),
            def,
            state: InstanceState::Running,
            started_at: now,
            // F-137 additional mandate: carry the seed prompt onto the
            // registered instance so a later step-executor (or a test harness
            // like F-137's integration test) can materialise it as the
            // child's first user turn.
            initial_prompt: ctx.initial_prompt.clone(),
        };

        self.registry
            .lock()
            .await
            .insert(instance.id.clone(), instance.clone());

        // `send` fails only when no subscribers are attached; that is a
        // no-op for us — the event is still valid, just nobody was listening.
        let _ = self.tx.send(AgentEvent::Spawned {
            id: instance.id.clone(),
            at: now,
        });

        Ok(instance)
    }

    /// Drive an instance to [`InstanceState::Completed`] and emit the
    /// terminal event. No-op (returns `Ok`) if the instance is already
    /// terminal; unknown ids are a silent no-op as well — the orchestrator
    /// refuses to panic on a stale id from an earlier session.
    pub async fn stop(&self, id: &AgentInstanceId) -> Result<()> {
        let mut reg = self.registry.lock().await;
        if let Some(inst) = reg.get_mut(id) {
            if matches!(inst.state, InstanceState::Running) {
                inst.state = InstanceState::Completed;
                let now = Utc::now();
                let _ = self.tx.send(AgentEvent::Completed {
                    id: id.clone(),
                    at: now,
                });
            }
        }
        Ok(())
    }

    /// Drive an instance to [`InstanceState::Failed`] and emit the terminal
    /// event. Symmetric to [`Orchestrator::stop`]. Exposed so tests and
    /// F-134's spawner can signal failures without a real step executor.
    pub async fn fail(&self, id: &AgentInstanceId, reason: String) -> Result<()> {
        let mut reg = self.registry.lock().await;
        if let Some(inst) = reg.get_mut(id) {
            if matches!(inst.state, InstanceState::Running) {
                inst.state = InstanceState::Failed {
                    reason: reason.clone(),
                };
                let now = Utc::now();
                let _ = self.tx.send(AgentEvent::Failed {
                    id: id.clone(),
                    reason,
                    at: now,
                });
            }
        }
        Ok(())
    }

    /// Emit a `StepStarted` event against an instance. Does not mutate
    /// `InstanceState` — steps are transitions, not states.
    pub async fn record_step_started(&self, id: &AgentInstanceId, step: String) -> Result<()> {
        let _ = self.tx.send(AgentEvent::StepStarted {
            id: id.clone(),
            step,
            at: Utc::now(),
        });
        Ok(())
    }

    /// Emit a `StepFinished` event against an instance.
    pub async fn record_step_finished(&self, id: &AgentInstanceId, step: String) -> Result<()> {
        let _ = self.tx.send(AgentEvent::StepFinished {
            id: id.clone(),
            step,
            at: Utc::now(),
        });
        Ok(())
    }

    /// Snapshot of a single instance.
    pub async fn get(&self, id: &AgentInstanceId) -> Option<AgentInstance> {
        self.registry.lock().await.get(id).cloned()
    }

    /// Subscribe to the lifecycle event stream. Each subscriber gets its own
    /// receiver; late subscribers miss earlier events (bounded broadcast).
    pub fn state_stream(&self) -> BroadcastStream<AgentEvent> {
        BroadcastStream::new(self.tx.subscribe())
    }
}

impl Default for Orchestrator {
    fn default() -> Self {
        Self::new()
    }
}
