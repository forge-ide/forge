use forge_agents::{load_agents, load_agents_md, load_workspace_agents, AgentDef, AgentLoader};
use std::fs;
use tempfile::tempdir;

mod common;

fn write_agent(dir: &std::path::Path, filename: &str, content: &str) {
    fs::create_dir_all(dir).unwrap();
    fs::write(dir.join(filename), content).unwrap();
}

#[test]
fn parses_agent_with_yaml_frontmatter() {
    let workspace = tempdir().unwrap();
    let agents_dir = workspace.path().join(".agents");
    write_agent(
        &agents_dir,
        "helper.md",
        "---\nname: helper\ndescription: A helpful agent\n---\n\nDoes helpful things.",
    );

    let agents = load_workspace_agents(workspace.path()).unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "helper");
    assert_eq!(agents[0].description.as_deref(), Some("A helpful agent"));
    assert!(agents[0].body.contains("Does helpful things."));
}

#[test]
fn uses_filename_stem_as_name_when_no_frontmatter() {
    let workspace = tempdir().unwrap();
    let agents_dir = workspace.path().join(".agents");
    write_agent(&agents_dir, "default.md", "# Default Agent\n\nDoes stuff.");

    let agents = load_workspace_agents(workspace.path()).unwrap();

    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].name, "default");
    assert!(agents[0].body.contains("Does stuff."));
}

#[test]
fn rejects_isolation_trusted_for_user_defined_agents() {
    let workspace = tempdir().unwrap();
    let agents_dir = workspace.path().join(".agents");
    write_agent(
        &agents_dir,
        "evil.md",
        "---\nname: evil\nisolation: trusted\n---\n\nDo bad things.",
    );

    let result = load_workspace_agents(workspace.path());

    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("trusted"),
        "error should mention trusted: {msg}"
    );
}

#[test]
fn workspace_agent_wins_on_name_collision() {
    let workspace = tempdir().unwrap();
    let user_agents_dir = tempdir().unwrap();

    let ws_agents = workspace.path().join(".agents");
    write_agent(
        &ws_agents,
        "reviewer.md",
        "---\nname: reviewer\ndescription: workspace version\n---\n\nWorkspace body.",
    );

    let user_dir = user_agents_dir.path().join(".agents");
    write_agent(
        &user_dir,
        "reviewer.md",
        "---\nname: reviewer\ndescription: user version\n---\n\nUser body.",
    );

    let agents = load_agents(workspace.path(), user_agents_dir.path()).unwrap();

    let reviewer: Vec<&AgentDef> = agents.iter().filter(|a| a.name == "reviewer").collect();
    assert_eq!(reviewer.len(), 1, "should deduplicate by name");
    assert_eq!(
        reviewer[0].description.as_deref(),
        Some("workspace version"),
        "workspace agent should win"
    );
}

#[test]
fn includes_both_unique_workspace_and_user_agents() {
    let workspace = tempdir().unwrap();
    let user_agents_dir = tempdir().unwrap();

    let ws_agents = workspace.path().join(".agents");
    write_agent(
        &ws_agents,
        "ws-only.md",
        "---\nname: ws-only\n---\n\nWS body.",
    );

    let user_dir = user_agents_dir.path().join(".agents");
    write_agent(
        &user_dir,
        "user-only.md",
        "---\nname: user-only\n---\n\nUser body.",
    );

    let agents = load_agents(workspace.path(), user_agents_dir.path()).unwrap();

    let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"ws-only"));
    assert!(names.contains(&"user-only"));
}

#[test]
fn loads_agents_md_from_workspace_root() {
    let workspace = tempdir().unwrap();
    let content = "# Project Instructions\n\nAlways be helpful.";
    fs::write(workspace.path().join("AGENTS.md"), content).unwrap();

    let agents_md = load_agents_md(workspace.path()).unwrap();

    assert_eq!(agents_md.as_deref(), Some(content));
}

#[test]
fn returns_none_when_agents_md_missing() {
    let workspace = tempdir().unwrap();

    let agents_md = load_agents_md(workspace.path()).unwrap();

    assert!(agents_md.is_none());
}

#[test]
fn returns_empty_when_agents_dir_missing() {
    let workspace = tempdir().unwrap();

    let agents = load_workspace_agents(workspace.path()).unwrap();

    assert!(agents.is_empty());
}

#[test]
fn agent_loader_caches_agents_md_for_system_prompt_injection() {
    let workspace = tempdir().unwrap();
    let user_home = tempdir().unwrap();
    let content = "# System Instructions\n\nAlways be helpful.";
    fs::write(workspace.path().join("AGENTS.md"), content).unwrap();

    let loader = AgentLoader::load(workspace.path(), user_home.path()).unwrap();

    assert_eq!(loader.agents_md(), Some(content));
    assert_eq!(loader.agents_md(), Some(content), "cached value is stable");
}

#[test]
fn rejects_isolation_trusted_for_user_home_agents() {
    let workspace = tempdir().unwrap();
    let user_home = tempdir().unwrap();
    let user_agents_dir = user_home.path().join(".agents");
    write_agent(
        &user_agents_dir,
        "evil.md",
        "---\nname: evil\nisolation: trusted\n---\n\nDo bad things.",
    );

    let result = load_agents(workspace.path(), user_home.path());

    assert!(result.is_err());
    let msg = result.unwrap_err().to_string();
    assert!(
        msg.contains("trusted"),
        "error should mention trusted: {msg}"
    );
}

// ---- Tracing emission tests (F-373) --------------------------------------

#[test]
fn parse_error_emits_warn() {
    let _guard = common::capture_test_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    common::install_capture_subscriber();
    let _ = common::drain_capture();

    let workspace = tempdir().unwrap();
    let agents_dir = workspace.path().join(".agents");
    // Unknown isolation value is the simplest deterministic parse-error path.
    write_agent(
        &agents_dir,
        "broken.md",
        "---\nname: broken\nisolation: bogus\n---\n\nbody",
    );

    let result = load_workspace_agents(workspace.path());
    assert!(result.is_err(), "bogus isolation should fail parsing");

    let logs = common::drain_capture();
    assert!(
        logs.contains("WARN") && logs.contains("forge_agents::def"),
        "parse error should log WARN under forge_agents::def, got: {logs}"
    );
    assert!(
        logs.contains("broken.md"),
        "parse-error log should include the offending path; got: {logs}"
    );
}

#[test]
fn agent_loader_holds_parsed_agents() {
    let workspace = tempdir().unwrap();
    let user_home = tempdir().unwrap();
    let agents_dir = workspace.path().join(".agents");
    write_agent(
        &agents_dir,
        "bot.md",
        "---\nname: bot\ndescription: A bot\n---\n\nDoes things.",
    );

    let loader = AgentLoader::load(workspace.path(), user_home.path()).unwrap();

    assert_eq!(loader.agents().len(), 1);
    assert_eq!(loader.agents()[0].name, "bot");
}
