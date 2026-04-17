use std::path::Path;
use tempfile::TempDir;

fn gitignore_path(root: &Path) -> std::path::PathBuf {
    root.join(".forge").join(".gitignore")
}

#[tokio::test]
async fn creates_gitignore_on_first_workspace_write() {
    let dir = TempDir::new().unwrap();
    forge_core::workspace::ensure_gitignore(dir.path())
        .await
        .unwrap();

    let gi = gitignore_path(dir.path());
    assert!(gi.exists(), ".forge/.gitignore should exist");
    let contents = tokio::fs::read_to_string(&gi).await.unwrap();
    assert_eq!(contents.trim(), "*", ".forge/.gitignore should contain *");
}

#[tokio::test]
async fn gitignore_not_overwritten_if_already_exists() {
    let dir = TempDir::new().unwrap();
    let forge_dir = dir.path().join(".forge");
    tokio::fs::create_dir_all(&forge_dir).await.unwrap();
    tokio::fs::write(forge_dir.join(".gitignore"), "custom content")
        .await
        .unwrap();

    forge_core::workspace::ensure_gitignore(dir.path())
        .await
        .unwrap();

    let contents = tokio::fs::read_to_string(forge_dir.join(".gitignore"))
        .await
        .unwrap();
    assert_eq!(
        contents, "custom content",
        "existing .gitignore must not be overwritten"
    );
}

#[tokio::test]
async fn event_log_create_triggers_gitignore() {
    use forge_core::{EventLog, SessionId};

    let dir = TempDir::new().unwrap();
    let sid = SessionId::new();
    let path = dir
        .path()
        .join(".forge")
        .join("sessions")
        .join(sid.to_string())
        .join("events.jsonl");
    let _log = EventLog::create(&path).await.unwrap();

    let gi = gitignore_path(dir.path());
    assert!(
        gi.exists(),
        "EventLog::create should produce .forge/.gitignore"
    );
    let contents = tokio::fs::read_to_string(&gi).await.unwrap();
    assert_eq!(contents.trim(), "*");
}
