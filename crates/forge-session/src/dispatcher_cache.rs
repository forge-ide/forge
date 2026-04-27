//! F-567: per-session [`ToolDispatcher`] cache keyed by an MCP tools-list
//! epoch.
//!
//! Before this cache, `run_turn` allocated a fresh `HashMap`, registered the
//! five built-in tools (`fs.read` / `fs.write` / `fs.edit` / `shell.exec` /
//! `agent.spawn`) and then locked the [`McpManager`] to walk every advertised
//! tool and box up an `McpTool` adapter — every turn, before the first
//! provider byte streamed. With M servers × T tools that's M·T allocations
//! plus a manager lock acquisition on the critical path of every turn.
//!
//! The cache holds an `Arc<ToolDispatcher>` together with the epoch the
//! dispatcher was built against. `serve_with_session` pumps the
//! `McpManager::state_stream` and bumps the epoch on every transition (any
//! state change can shift a server's `tools` vec). The first turn after a
//! transition rebuilds; subsequent turns get a single `Arc::clone` and start
//! the provider request immediately. Builtins are immutable — they only have
//! to be registered into the cached dispatcher when the MCP layer forces a
//! rebuild.
//!
//! Sessions without MCP wired up (tests, embedders that pass `mcp = None`)
//! also benefit: the cache builds the builtin-only dispatcher exactly once
//! and reuses it forever.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex as AsyncMutex;

use crate::tools::{
    AgentSpawnTool, FsEditTool, FsReadTool, FsWriteTool, McpTool, MemoryWriteTool, ShellExecTool,
    ToolDispatcher,
};
use forge_agents::MemoryStore;
use forge_mcp::McpManager;

/// F-601: per-agent memory binding consulted by the dispatcher cache.
///
/// Built once per session in `serve_with_session` only when the active
/// agent has `memory_enabled: true`. When `Some`, the cache registers a
/// [`MemoryWriteTool`] alongside the other built-ins so the agent can
/// update its own `~/.config/forge/memory/<agent>.md` file. When `None`,
/// the tool is omitted from the dispatcher entirely — an agent that has
/// not opted in cannot discover the wire name and the surface area stays
/// minimal.
#[derive(Debug, Clone)]
pub struct MemoryWriteBinding {
    /// Shared store rooted at `~/.config/forge/memory/`.
    pub store: Arc<MemoryStore>,
    /// The agent name keyed against the store — same string used to derive
    /// the `<agent>.md` filename and the `## Memory` injection.
    pub agent_id: String,
}

/// Monotonic counter for the MCP tools-list snapshot.
///
/// Bumped by the `serve_with_session` state-stream forwarder on every
/// `McpStateEvent` so any transition (a new `Healthy`, a `Failed`, a
/// `Disabled` toggle) invalidates the cached dispatcher. Sessions without an
/// `McpManager` keep the epoch at zero forever — the first build is also
/// the only build.
#[derive(Debug, Default, Clone)]
pub struct McpToolsEpoch(Arc<AtomicU64>);

impl McpToolsEpoch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn current(&self) -> u64 {
        self.0.load(Ordering::Acquire)
    }

    pub fn bump(&self) {
        self.0.fetch_add(1, Ordering::AcqRel);
    }
}

/// Snapshot inside the cache: the dispatcher together with the epoch it
/// was built against. Wrapped in an `Arc` so cache hits hand out a cheap
/// shared pointer.
struct CachedDispatcher {
    epoch: u64,
    dispatcher: Arc<ToolDispatcher>,
}

/// Cached [`ToolDispatcher`] that rebuilds only when the MCP tools-list
/// epoch advances.
///
/// Constructed once per session in `serve_with_session`; cloned (cheap) into
/// every `run_turn`. The `Mutex` guards a small swap — the dispatcher itself
/// is held behind an `Arc` so the lock is released before the dispatcher is
/// used for tool dispatch.
pub struct DispatcherCache {
    epoch: McpToolsEpoch,
    cache: AsyncMutex<Option<CachedDispatcher>>,
    /// F-601: when `Some`, every cached dispatcher exposes `memory.write`.
    memory: Option<MemoryWriteBinding>,
}

impl DispatcherCache {
    pub fn new(epoch: McpToolsEpoch) -> Arc<Self> {
        Arc::new(Self {
            epoch,
            cache: AsyncMutex::new(None),
            memory: None,
        })
    }

    /// F-601: build a cache that also registers `memory.write` against the
    /// agent identified by `binding`. Only called when the active agent
    /// has opted into cross-session memory.
    pub fn with_memory(epoch: McpToolsEpoch, binding: MemoryWriteBinding) -> Arc<Self> {
        Arc::new(Self {
            epoch,
            cache: AsyncMutex::new(None),
            memory: Some(binding),
        })
    }

    /// Return a dispatcher reflecting the current MCP tools-list snapshot.
    ///
    /// Steady state: one `Arc::clone` and we're done. After an MCP state
    /// transition: rebuild the dispatcher (builtins + every adapter from
    /// the current `mgr.list().await`), cache it tagged with the current
    /// epoch, and return.
    pub async fn get(&self, mcp: Option<&Arc<McpManager>>) -> Arc<ToolDispatcher> {
        let observed_epoch = self.epoch.current();

        let mut guard = self.cache.lock().await;
        if let Some(cached) = guard.as_ref() {
            if cached.epoch == observed_epoch {
                return Arc::clone(&cached.dispatcher);
            }
        }

        let dispatcher = build_dispatcher(mcp, self.memory.as_ref()).await;
        let dispatcher = Arc::new(dispatcher);
        *guard = Some(CachedDispatcher {
            epoch: observed_epoch,
            dispatcher: Arc::clone(&dispatcher),
        });
        dispatcher
    }
}

/// Build a fresh dispatcher: register every builtin, then every MCP-server
/// adapter exposed by `mgr.list().await`. Mirrors the previous in-line body
/// of `run_turn` exactly so dispatch behavior stays identical.
async fn build_dispatcher(
    mcp: Option<&Arc<McpManager>>,
    memory: Option<&MemoryWriteBinding>,
) -> ToolDispatcher {
    let mut dispatcher = ToolDispatcher::new();
    dispatcher
        .register(Box::new(FsReadTool))
        .expect("fs.read must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(FsWriteTool))
        .expect("fs.write must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(FsEditTool))
        .expect("fs.edit must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(ShellExecTool))
        .expect("shell.exec must register on a fresh dispatcher");
    dispatcher
        .register(Box::new(AgentSpawnTool))
        .expect("agent.spawn must register on a fresh dispatcher");

    if let Some(binding) = memory {
        dispatcher
            .register(Box::new(MemoryWriteTool::new(
                Arc::clone(&binding.store),
                binding.agent_id.clone(),
            )))
            .expect("memory.write must register on a fresh dispatcher");
    }

    if let Some(mgr) = mcp {
        for server in mgr.list().await {
            for tool in server.tools {
                if let Some(adapter) = McpTool::new(
                    tool.name.clone(),
                    tool.description,
                    tool.read_only,
                    mgr.clone(),
                ) {
                    // Silently skip duplicate registrations — a malformed
                    // tools/list response that repeats a name shouldn't
                    // fail the whole turn. The namespace guarantees
                    // cross-server collision-free names; within a single
                    // server, the MCP spec forbids duplicates.
                    let _ = dispatcher.register(Box::new(adapter));
                }
            }
        }
    }

    dispatcher
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cache_returns_same_arc_until_epoch_bumps() {
        let epoch = McpToolsEpoch::new();
        let cache = DispatcherCache::new(epoch.clone());

        let a = cache.get(None).await;
        let b = cache.get(None).await;
        assert!(
            Arc::ptr_eq(&a, &b),
            "cache hit must return the same Arc — second call rebuilt"
        );

        epoch.bump();
        let c = cache.get(None).await;
        assert!(
            !Arc::ptr_eq(&a, &c),
            "epoch bump must force a rebuild — same Arc returned after invalidation"
        );

        let d = cache.get(None).await;
        assert!(
            Arc::ptr_eq(&c, &d),
            "post-rebuild, cache must hit again at the new epoch"
        );
    }

    #[tokio::test]
    async fn cache_registers_all_builtins() {
        let cache = DispatcherCache::new(McpToolsEpoch::new());
        let dispatcher = cache.get(None).await;
        for name in [
            "fs.read",
            "fs.write",
            "fs.edit",
            "shell.exec",
            "agent.spawn",
        ] {
            assert!(
                dispatcher.get(name).is_ok(),
                "builtin {name} must be registered on a fresh cache"
            );
        }
    }

    #[tokio::test]
    async fn memory_write_is_omitted_when_binding_absent() {
        // F-601: an agent that has not opted into memory must not see the
        // tool. `DispatcherCache::new` creates a cache without a binding;
        // `memory.write` must be absent.
        let cache = DispatcherCache::new(McpToolsEpoch::new());
        let dispatcher = cache.get(None).await;
        assert!(
            dispatcher.get("memory.write").is_err(),
            "memory.write must NOT register without a binding"
        );
    }

    #[tokio::test]
    async fn memory_write_is_registered_when_binding_present() {
        // F-601: when the active agent has `memory_enabled: true`,
        // `serve_with_session` constructs the cache via `with_memory` and
        // the resulting dispatcher exposes `memory.write` alongside the
        // other built-ins.
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(MemoryStore::new(dir.path()));
        let binding = MemoryWriteBinding {
            store,
            agent_id: "scribe".to_string(),
        };
        let cache = DispatcherCache::with_memory(McpToolsEpoch::new(), binding);
        let dispatcher = cache.get(None).await;
        assert!(
            dispatcher.get("memory.write").is_ok(),
            "memory.write must register when a binding is present"
        );
        // The other built-ins must still register too.
        for name in [
            "fs.read",
            "fs.write",
            "fs.edit",
            "shell.exec",
            "agent.spawn",
        ] {
            assert!(
                dispatcher.get(name).is_ok(),
                "builtin {name} must still register alongside memory.write"
            );
        }
    }
}
