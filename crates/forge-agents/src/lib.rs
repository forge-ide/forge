use anyhow::{bail, Context, Result};
use gray_matter::{engine::YAML, Matter, ParsedEntity};
use serde::Deserialize;
use std::{fs, path::Path};

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

pub fn load_workspace_agents(workspace_root: &Path) -> Result<Vec<AgentDef>> {
    load_from_dir(&workspace_root.join(".agents"))
}

pub fn load_user_agents(user_home: &Path) -> Result<Vec<AgentDef>> {
    load_from_dir(&user_home.join(".agents"))
}

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

pub fn load_agents_md(workspace_root: &Path) -> Result<Option<String>> {
    let path = workspace_root.join("AGENTS.md");
    if path.exists() {
        Ok(Some(fs::read_to_string(path)?))
    } else {
        Ok(None)
    }
}

pub struct AgentLoader {
    agents: Vec<AgentDef>,
    agents_md: Option<String>,
}

impl AgentLoader {
    pub fn load(workspace_root: &Path, user_home: &Path) -> Result<Self> {
        Ok(Self {
            agents: load_agents(workspace_root, user_home)?,
            agents_md: load_agents_md(workspace_root)?,
        })
    }

    pub fn agents(&self) -> &[AgentDef] {
        &self.agents
    }

    pub fn agents_md(&self) -> Option<&str> {
        self.agents_md.as_deref()
    }
}
