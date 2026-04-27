//! Integration tests for `forge skill install` (F-590).
//!
//! These exercise the in-process install path end-to-end:
//!
//! 1. Resolve a local fixture skill via `LocalPathResolver`.
//! 2. Install it into a temp scope.
//! 3. Confirm F-589's `load_workspace_skills` / `load_user_skills`
//!    re-discovers it on disk.
//!
//! Shelling out to the CLI binary would also work, but skipping it keeps
//! the test fast and avoids the cargo-test → cargo-build dependency.

use std::path::PathBuf;

use forge_agents::skill_loader::{load_user_skills, load_workspace_skills};
use forge_cli::skill::{
    install_resolved, list_installed, remove_skill, LocalPathResolver, Resolver, SkillScope,
};
use tempfile::tempdir;

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample-skill")
}

#[test]
fn install_local_fixture_into_user_scope_and_rediscover() {
    let workspace = tempdir().unwrap();
    let home = tempdir().unwrap();

    let fixture = fixture_path();
    let resolver = LocalPathResolver::new(fixture, std::env::current_dir().unwrap());
    let resolved = resolver.resolve().expect("fixture must parse");

    assert_eq!(resolved.skill.id.as_str(), "sample-skill");
    assert_eq!(resolved.skill.name, "Sample Skill");

    let installed_at =
        install_resolved(&resolved, SkillScope::User, workspace.path(), home.path()).unwrap();
    assert!(installed_at.join("SKILL.md").exists());

    // F-589 discovery picks it up from the user scope.
    let user_skills = load_user_skills(home.path()).unwrap();
    assert_eq!(user_skills.len(), 1);
    assert_eq!(user_skills[0].id.as_str(), "sample-skill");
    assert_eq!(user_skills[0].version.as_deref(), Some("1.0.0"));

    // It must *not* leak into workspace scope.
    let ws_skills = load_workspace_skills(workspace.path()).unwrap();
    assert!(ws_skills.is_empty());

    // List surfaces it.
    let rows = list_installed(workspace.path(), home.path()).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "sample-skill");
    assert_eq!(rows[0].scope, SkillScope::User);

    // Remove restores the empty state.
    assert!(remove_skill(
        "sample-skill",
        SkillScope::User,
        workspace.path(),
        home.path()
    )
    .unwrap());
    assert!(load_user_skills(home.path()).unwrap().is_empty());
}

#[test]
fn install_local_fixture_into_workspace_scope() {
    let workspace = tempdir().unwrap();
    let home = tempdir().unwrap();

    let resolver = LocalPathResolver::new(fixture_path(), std::env::current_dir().unwrap());
    let resolved = resolver.resolve().unwrap();

    install_resolved(
        &resolved,
        SkillScope::Workspace,
        workspace.path(),
        home.path(),
    )
    .unwrap();

    let ws_skills = load_workspace_skills(workspace.path()).unwrap();
    assert_eq!(ws_skills.len(), 1);
    assert_eq!(ws_skills[0].id.as_str(), "sample-skill");

    // Re-installing the same skill into the same scope refuses.
    let err = install_resolved(
        &resolved,
        SkillScope::Workspace,
        workspace.path(),
        home.path(),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("already installed"),
        "expected duplicate error: {err}"
    );
}
