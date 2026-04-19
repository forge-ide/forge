use anyhow::{bail, Context, Result};
use gray_matter::{engine::YAML, Matter, ParsedEntity};
use serde::Deserialize;
use std::{fs, path::Path};

/// Canonical agent definition parsed from a Markdown file with YAML frontmatter.
///
/// `name` defaults to the file stem when frontmatter is absent or omits it,
/// `body` holds the prompt content with the frontmatter stripped, and
/// `allowed_paths` scopes filesystem access for tools the agent may invoke.
#[derive(Debug, Clone, PartialEq)]
pub struct AgentDef {
    pub name: String,
    pub description: Option<String>,
    pub body: String,
    pub allowed_paths: Vec<String>,
}

#[derive(Deserialize, Default)]
struct Frontmatter {
    name: Option<String>,
    description: Option<String>,
    isolation: Option<String>,
    allowed_paths: Option<Vec<String>>,
}

fn parse_agent_file(path: &Path) -> Result<AgentDef> {
    let raw = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    let matter = Matter::<YAML>::new();
    let parsed: ParsedEntity<Frontmatter> = matter
        .parse(&raw)
        .with_context(|| format!("parsing frontmatter in {}", path.display()))?;

    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown")
        .to_string();

    match parsed.data {
        Some(fm) => {
            if fm.isolation.as_deref() == Some("trusted") {
                bail!(
                    "isolation: trusted is not allowed for user-defined agents ({})",
                    path.display()
                );
            }
            Ok(AgentDef {
                name: fm.name.unwrap_or(stem),
                description: fm.description,
                body: parsed.content,
                allowed_paths: fm.allowed_paths.unwrap_or_default(),
            })
        }
        None => Ok(AgentDef {
            name: stem,
            description: None,
            body: parsed.content,
            allowed_paths: vec![],
        }),
    }
}

fn load_from_dir(dir: &Path) -> Result<Vec<AgentDef>> {
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut paths: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    paths.sort();
    paths.iter().map(|p| parse_agent_file(p)).collect()
}

/// Load agents from `<workspace_root>/.agents/*.md`, returning an empty vec if the directory is absent.
pub fn load_workspace_agents(workspace_root: &Path) -> Result<Vec<AgentDef>> {
    load_from_dir(&workspace_root.join(".agents"))
}

/// Load agents from `<user_home>/.agents/*.md`, returning an empty vec if the directory is absent.
pub fn load_user_agents(user_home: &Path) -> Result<Vec<AgentDef>> {
    load_from_dir(&user_home.join(".agents"))
}

/// Load and merge user-home and workspace-local agent definitions.
///
/// User agents are loaded first; workspace agents are then layered on top so
/// that on a name collision the workspace definition replaces the user one,
/// and workspace-only agents are appended. This lets a project pin or override
/// agents without editing the user's home directory.
pub fn load_agents(workspace_root: &Path, user_home: &Path) -> Result<Vec<AgentDef>> {
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
pub fn load_agents_md(workspace_root: &Path) -> Result<Option<String>> {
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
    pub fn load(workspace_root: &Path, user_home: &Path) -> Result<Self> {
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
