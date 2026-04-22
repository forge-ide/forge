//! Agent definition types and the `.agents/*.md` loader.
//!
//! `AgentDef` is the parsed shape of one agent file; [`crate::AgentLoader`]
//! bundles the workspace + user merge plus the optional `AGENTS.md` preamble.

use anyhow::Context;
use gray_matter::{engine::YAML, Matter, ParsedEntity};
use serde::Deserialize;
use std::{fs, path::Path};

use crate::error::{Error, Result};

/// Runtime isolation level applied to a live [`AgentInstance`](crate::AgentInstance).
///
/// `Trusted` bypasses sandboxing and is reserved for built-in skills shipped
/// with Forge — user-authored agents are rejected both at parse time and at
/// [`crate::Orchestrator::spawn`] if they declare it. `Process` is the
/// default for user agents. `Container(_)` is planned for later phases; for
/// now we encode the variant as `Container` (unit) so the enum is closed and
/// future work can attach a `ContainerSpec` without a breaking rename.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Isolation {
    /// No sandbox; reserved for built-in skills shipped with Forge.
    Trusted,
    /// Sandboxed subprocess isolation (default for user agents).
    #[default]
    Process,
    /// Container-backed isolation; reserved for a later phase.
    Container,
}

/// Canonical agent definition parsed from a Markdown file with YAML frontmatter.
///
/// `name` defaults to the file stem when frontmatter is absent or omits it,
/// `body` holds the prompt content with the frontmatter stripped, and
/// `allowed_paths` scopes filesystem access for tools the agent may invoke.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentDef {
    /// Agent identifier; defaults to the source file stem when frontmatter omits it.
    pub name: String,
    /// Human-readable description surfaced in pickers and UI banners.
    pub description: Option<String>,
    /// Prompt body with the YAML frontmatter stripped.
    pub body: String,
    /// Filesystem scopes the agent's tools are permitted to access.
    pub allowed_paths: Vec<String>,
    /// Runtime isolation policy applied when this def is spawned.
    pub isolation: Isolation,
}

#[derive(Deserialize, Default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    isolation: Option<String>,
    allowed_paths: Option<Vec<String>>,
}

pub(crate) fn parse_agent_file(path: &Path) -> Result<AgentDef> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("reading {}", path.display()))
        .map_err(Error::from)?;

    let matter = Matter::<YAML>::new();
    let parsed: ParsedEntity<Frontmatter> = matter
        .parse(&raw)
        .with_context(|| format!("parsing frontmatter in {}", path.display()))
        .map_err(Error::from)?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    match parsed.data {
        Some(fm) => {
            let name = fm.name.unwrap_or_else(|| stem.clone());
            let isolation = match fm.isolation.as_deref() {
                Some("trusted") => {
                    return Err(Error::IsolationViolation {
                        name,
                        path: Some(path.to_path_buf()),
                    })
                }
                Some("container") => Isolation::Container,
                Some("process") | None => Isolation::Process,
                Some(other) => {
                    return Err(Error::Other(anyhow::anyhow!(
                        "unknown isolation level '{other}' in {}",
                        path.display()
                    )))
                }
            };
            Ok(AgentDef {
                name,
                description: fm.description,
                body: parsed.content,
                allowed_paths: fm.allowed_paths.unwrap_or_default(),
                isolation,
            })
        }
        None => Ok(AgentDef {
            name: stem,
            description: None,
            body: parsed.content,
            allowed_paths: vec![],
            isolation: Isolation::Process,
        }),
    }
}

pub(crate) fn load_from_dir(dir: &Path) -> Result<Vec<AgentDef>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut paths: Vec<_> = fs::read_dir(dir)
        .map_err(|e| Error::Other(anyhow::Error::from(e)))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    paths.sort();
    paths.iter().map(|p| parse_agent_file(p)).collect()
}
