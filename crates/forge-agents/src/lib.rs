//! `forge-agents` — agent definitions, the `.agents/*.md` loader, and the
//! runtime orchestrator.
//!
//! The loader merges workspace and user-home agent definitions; the runtime
//! orchestrator instantiates them into live [`AgentInstance`]s and forwards
//! their lifecycle on a broadcast stream. See:
//!
//! - `docs/architecture/crate-architecture.md` §3.4 for the design
//! - `docs/design/ai-patterns.md` for the UX vocabulary

mod def;
mod error;
mod orchestrator;

use std::{fs, path::Path};

pub use def::{AgentDef, Isolation};
pub use error::{Error, Result};
pub use orchestrator::{
    AgentEvent, AgentInstance, AgentScope, InstanceState, Orchestrator, SpawnContext,
};

use def::load_from_dir;

/// Load agents from `<workspace_root>/.agents/*.md`, returning an empty vec if the directory is absent.
pub fn load_workspace_agents(workspace_root: &Path) -> anyhow::Result<Vec<AgentDef>> {
    load_from_dir(&workspace_root.join(".agents")).map_err(anyhow::Error::from)
}

/// Load agents from `<user_home>/.agents/*.md`, returning an empty vec if the directory is absent.
pub fn load_user_agents(user_home: &Path) -> anyhow::Result<Vec<AgentDef>> {
    load_from_dir(&user_home.join(".agents")).map_err(anyhow::Error::from)
}

/// Load and merge user-home and workspace-local agent definitions.
///
/// User agents are loaded first; workspace agents are then layered on top so
/// that on a name collision the workspace definition replaces the user one,
/// and workspace-only agents are appended. This lets a project pin or override
/// agents without editing the user's home directory.
pub fn load_agents(workspace_root: &Path, user_home: &Path) -> anyhow::Result<Vec<AgentDef>> {
    let workspace = load_workspace_agents(workspace_root)?;
    let mut merged = load_user_agents(user_home)?;

    for ws_agent in workspace {
        match merged.iter().position(|a| a.name == ws_agent.name) {
            Some(pos) => merged[pos] = ws_agent,
            None => merged.push(ws_agent),
        }
    }
    Ok(merged)
}

/// Read `<workspace_root>/AGENTS.md` if present, returning `Ok(None)` when the file is absent.
pub fn load_agents_md(workspace_root: &Path) -> anyhow::Result<Option<String>> {
    let path = workspace_root.join("AGENTS.md");
    if path.exists() {
        Ok(Some(fs::read_to_string(path)?))
    } else {
        Ok(None)
    }
}

/// Bundle of merged agent definitions plus the optional workspace-level `AGENTS.md` preamble.
///
/// Constructed once per session via [`AgentLoader::load`] and then queried
/// through [`AgentLoader::agents`] and [`AgentLoader::agents_md`].
pub struct AgentLoader {
    agents: Vec<AgentDef>,
    agents_md: Option<String>,
}

impl AgentLoader {
    /// Load workspace + user agents and the workspace `AGENTS.md` in one pass.
    pub fn load(workspace_root: &Path, user_home: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            agents: load_agents(workspace_root, user_home)?,
            agents_md: load_agents_md(workspace_root)?,
        })
    }

    /// Borrow the merged agent definitions, ordered user-first then workspace-only appended.
    pub fn agents(&self) -> &[AgentDef] {
        &self.agents
    }

    /// Borrow the workspace `AGENTS.md` contents, or `None` if the file was absent.
    pub fn agents_md(&self) -> Option<&str> {
        self.agents_md.as_deref()
    }
}
