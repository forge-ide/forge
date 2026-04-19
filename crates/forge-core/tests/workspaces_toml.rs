use chrono::Utc;
use forge_core::{
    workspaces::{read_workspaces, write_workspaces, WorkspaceEntry},
    WorkspaceId,
};
use std::path::PathBuf;
use tempfile::TempDir;

#[tokio::test]
async fn workspaces_toml_round_trip_single() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("workspaces.toml");

    let entries = vec![WorkspaceEntry {
        id: WorkspaceId::new(),
        path: PathBuf::from("/home/alice/code/acme-api"),
        name: "acme-api".to_string(),
        last_opened: Utc::now().with_nanosecond(0).unwrap(),
        pinned: false,
    }];

    write_workspaces(&path, &entries).await.unwrap();
    let loaded = read_workspaces(&path).await.unwrap();

    assert_eq!(entries.len(), loaded.len());
    assert_eq!(entries[0].id, loaded[0].id);
    assert_eq!(entries[0].path, loaded[0].path);
    assert_eq!(entries[0].name, loaded[0].name);
    assert_eq!(entries[0].last_opened, loaded[0].last_opened);
    assert_eq!(entries[0].pinned, loaded[0].pinned);
}

#[tokio::test]
async fn workspaces_toml_round_trip_multiple() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("workspaces.toml");

    let entries = vec![
        WorkspaceEntry {
            id: WorkspaceId::new(),
            path: PathBuf::from("/home/alice/code/acme-api"),
            name: "acme-api".to_string(),
            last_opened: Utc::now().with_nanosecond(0).unwrap(),
            pinned: true,
        },
        WorkspaceEntry {
            id: WorkspaceId::new(),
            path: PathBuf::from("/home/alice/code/docs-v2"),
            name: "docs-v2".to_string(),
            last_opened: Utc::now().with_nanosecond(0).unwrap(),
            pinned: false,
        },
    ];

    write_workspaces(&path, &entries).await.unwrap();
    let loaded = read_workspaces(&path).await.unwrap();

    assert_eq!(2, loaded.len());
    assert_eq!(entries[0].name, loaded[0].name);
    assert_eq!(entries[1].name, loaded[1].name);
    assert_eq!(entries[0].pinned, loaded[0].pinned);
    assert_eq!(entries[1].pinned, loaded[1].pinned);
}

#[tokio::test]
async fn workspaces_toml_creates_parent_dirs() {
    let dir = TempDir::new().unwrap();
    let path = dir
        .path()
        .join(".config")
        .join("forge")
        .join("workspaces.toml");

    write_workspaces(&path, &[]).await.unwrap();
    assert!(path.exists());
}

#[tokio::test]
async fn workspaces_toml_rejects_unknown_field_on_entry() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("workspaces.toml");

    // Hand-written TOML: a valid entry plus a forward-looking field (e.g. a
    // trust-bearing `trusted` flag) that this daemon does not recognize.
    // Must error rather than silently drop the field — audit L1 (F-065).
    let toml_contents = r#"
[[workspaces]]
id = "01J9X0000000000000000WENTR"
path = "/home/alice/code/acme-api"
name = "acme-api"
last_opened = "2025-01-01T00:00:00Z"
pinned = false
trusted = true
"#;
    tokio::fs::write(&path, toml_contents).await.unwrap();

    let err = read_workspaces(&path)
        .await
        .expect_err("unknown field on WorkspaceEntry must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("trusted") || msg.contains("unknown field"),
        "expected unknown-field error, got: {msg}"
    );
}

#[tokio::test]
async fn workspaces_toml_rejects_unknown_field_on_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("workspaces.toml");

    // Top-level unknown key on WorkspacesFile (e.g. a future `version` field
    // that an older daemon should not silently ignore).
    let toml_contents = "workspaces = []\nversion = 2\n";
    tokio::fs::write(&path, toml_contents).await.unwrap();

    let err = read_workspaces(&path)
        .await
        .expect_err("unknown field on WorkspacesFile must error");
    let msg = format!("{err:#}");
    assert!(
        msg.contains("version") || msg.contains("unknown field"),
        "expected unknown-field error, got: {msg}"
    );
}

#[tokio::test]
async fn workspaces_toml_empty_list() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("workspaces.toml");

    write_workspaces(&path, &[]).await.unwrap();
    let loaded = read_workspaces(&path).await.unwrap();

    assert!(loaded.is_empty());
}

trait WithNanosecond {
    fn with_nanosecond(self, ns: u32) -> Option<Self>
    where
        Self: Sized;
}

impl WithNanosecond for chrono::DateTime<chrono::Utc> {
    fn with_nanosecond(self, ns: u32) -> Option<Self> {
        chrono::Timelike::with_nanosecond(&self, ns)
    }
}
