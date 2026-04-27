use forge_agents::{load_skills, load_user_skills, load_workspace_skills, parse_skill_file};
use std::fs;
use tempfile::tempdir;

fn write_skill(scope_root: &std::path::Path, id: &str, body: &str) {
    let dir = scope_root.join(".skills").join(id);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("SKILL.md"), body).unwrap();
}

#[test]
fn parses_skill_with_yaml_frontmatter() {
    let workspace = tempdir().unwrap();
    write_skill(
        workspace.path(),
        "planner",
        "---\nname: Planner\nversion: 1.2.0\ndescription: Plans things\ntools:\n  - shell\n  - fs.read\n---\n\nPrompt body for planner.",
    );

    let skills = load_workspace_skills(workspace.path()).unwrap();

    assert_eq!(skills.len(), 1);
    let s = &skills[0];
    assert_eq!(s.id.as_str(), "planner");
    assert_eq!(s.name, "Planner");
    assert_eq!(s.version.as_deref(), Some("1.2.0"));
    assert_eq!(s.description.as_deref(), Some("Plans things"));
    assert_eq!(s.tools, vec!["shell".to_string(), "fs.read".to_string()]);
    assert!(s.prompt.contains("Prompt body for planner."));
    assert!(s.source_path.ends_with("SKILL.md"));
}

#[test]
fn defaults_name_to_folder_when_frontmatter_missing() {
    let workspace = tempdir().unwrap();
    write_skill(workspace.path(), "default-skill", "# Just a body");

    let skills = load_workspace_skills(workspace.path()).unwrap();

    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id.as_str(), "default-skill");
    assert_eq!(skills[0].name, "default-skill");
    assert!(skills[0].prompt.contains("Just a body"));
}

#[test]
fn ignores_unknown_frontmatter_fields() {
    let workspace = tempdir().unwrap();
    write_skill(
        workspace.path(),
        "future",
        "---\nname: Future\nfuture_field: something\n---\n\nBody.",
    );

    let skills = load_workspace_skills(workspace.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "Future");
}

#[test]
fn skips_folders_without_skill_md() {
    let workspace = tempdir().unwrap();
    let empty = workspace.path().join(".skills").join("empty");
    fs::create_dir_all(&empty).unwrap();
    write_skill(workspace.path(), "real", "---\nname: Real\n---\nbody");

    let skills = load_workspace_skills(workspace.path()).unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id.as_str(), "real");
}

#[test]
fn returns_empty_when_skills_dir_missing() {
    let workspace = tempdir().unwrap();
    let skills = load_workspace_skills(workspace.path()).unwrap();
    assert!(skills.is_empty());
}

#[test]
fn workspace_shadows_user_on_id_collision() {
    let workspace = tempdir().unwrap();
    let user = tempdir().unwrap();

    write_skill(
        workspace.path(),
        "shared",
        "---\nname: Shared\ndescription: workspace version\n---\nws body",
    );
    write_skill(
        user.path(),
        "shared",
        "---\nname: Shared\ndescription: user version\n---\nuser body",
    );

    let skills = load_skills(workspace.path(), user.path()).unwrap();

    let shared: Vec<_> = skills
        .iter()
        .filter(|s| s.id.as_str() == "shared")
        .collect();
    assert_eq!(shared.len(), 1, "should deduplicate by id");
    assert_eq!(shared[0].description.as_deref(), Some("workspace version"));
}

#[test]
fn merges_unique_workspace_and_user_skills() {
    let workspace = tempdir().unwrap();
    let user = tempdir().unwrap();

    write_skill(workspace.path(), "ws-only", "---\nname: ws\n---\nws");
    write_skill(user.path(), "user-only", "---\nname: user\n---\nuser");

    let skills = load_skills(workspace.path(), user.path()).unwrap();

    let ids: Vec<_> = skills.iter().map(|s| s.id.as_str()).collect();
    assert!(ids.contains(&"ws-only"));
    assert!(ids.contains(&"user-only"));
}

#[test]
fn output_is_sorted_by_id_for_determinism() {
    let workspace = tempdir().unwrap();
    // Write in non-alphabetical order; loader must still emit sorted.
    for id in ["zeta", "alpha", "mu"] {
        write_skill(
            workspace.path(),
            id,
            &format!("---\nname: {id}\n---\nbody {id}"),
        );
    }

    let skills = load_workspace_skills(workspace.path()).unwrap();
    let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["alpha", "mu", "zeta"]);
}

#[test]
fn merged_output_is_sorted_by_id() {
    let workspace = tempdir().unwrap();
    let user = tempdir().unwrap();

    write_skill(workspace.path(), "zeta", "---\nname: z\n---\nbody");
    write_skill(user.path(), "alpha", "---\nname: a\n---\nbody");
    write_skill(user.path(), "mu", "---\nname: m\n---\nbody");

    let skills = load_skills(workspace.path(), user.path()).unwrap();
    let ids: Vec<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["alpha", "mu", "zeta"]);
}

#[test]
fn returns_empty_when_user_dir_missing() {
    let user = tempdir().unwrap();
    let skills = load_user_skills(user.path()).unwrap();
    assert!(skills.is_empty());
}

#[test]
fn parse_skill_file_directly() {
    let workspace = tempdir().unwrap();
    write_skill(workspace.path(), "direct", "---\nname: Direct\n---\nbody");
    let path = workspace
        .path()
        .join(".skills")
        .join("direct")
        .join("SKILL.md");

    let skill = parse_skill_file(&path).unwrap();
    assert_eq!(skill.id.as_str(), "direct");
    assert_eq!(skill.name, "Direct");
    assert_eq!(skill.source_path, path);
}

#[test]
fn rejects_skill_folder_with_invalid_id() {
    // Folder name beginning with `.` is not a valid SkillId. Skip the
    // load_from_scope path entirely (read_dir wouldn't list it as a regular
    // entry) — exercise parse_skill_file directly to confirm validation.
    let dir = tempdir().unwrap();
    let bad = dir.path().join(".dotted");
    fs::create_dir_all(&bad).unwrap();
    let path = bad.join("SKILL.md");
    fs::write(&path, "body").unwrap();

    let err = parse_skill_file(&path).unwrap_err();
    assert!(
        err.to_string().contains("invalid skill id"),
        "should reject leading-dot folder names, got: {err}"
    );
}
