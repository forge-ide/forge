use forge_core::{EventLog, SessionId};
use std::io::{BufRead, BufReader};
use tempfile::TempDir;

fn session_path(dir: &TempDir, id: &SessionId) -> std::path::PathBuf {
    dir.path().join(".forge").join("sessions").join(id.to_string()).join("events.jsonl")
}

#[tokio::test]
async fn creates_file_with_schema_header() {
    let dir = TempDir::new().unwrap();
    let id = SessionId::new();
    let path = session_path(&dir, &id);

    let _log = EventLog::create(&path).await.unwrap();

    let file = std::fs::File::open(&path).unwrap();
    let mut lines = BufReader::new(file).lines();
    let first = lines.next().expect("file should have at least one line").unwrap();
    assert_eq!(first, r#"{"schema_version":1}"#);
}

#[tokio::test]
async fn creates_parent_directories() {
    let dir = TempDir::new().unwrap();
    let id = SessionId::new();
    let path = session_path(&dir, &id);

    EventLog::create(&path).await.unwrap();

    assert!(path.exists());
}

#[tokio::test]
async fn open_rejects_file_missing_schema_header() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    std::fs::write(&path, b"not a schema header\n").unwrap();

    let result = EventLog::open(&path).await;

    assert!(result.is_err(), "open should reject file without schema header");
}

#[tokio::test]
async fn open_rejects_empty_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    std::fs::write(&path, b"").unwrap();

    let result = EventLog::open(&path).await;

    assert!(result.is_err(), "open should reject empty file");
}

#[tokio::test]
async fn open_accepts_file_with_valid_schema_header() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");

    let _log = EventLog::create(&path).await.unwrap();
    drop(_log);

    EventLog::open(&path).await.expect("open should succeed on a freshly created file");
}

#[tokio::test]
async fn appended_events_appear_after_header() {
    use forge_core::{Event, MessageId};
    use chrono::Utc;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    let mut log = EventLog::create(&path).await.unwrap();

    let event = Event::AssistantDelta {
        id: MessageId::new(),
        at: Utc::now(),
        delta: "hello".to_string(),
    };
    log.append(&event).await.unwrap();
    log.flush().await.unwrap();

    let file = std::fs::File::open(&path).unwrap();
    let lines: Vec<String> = BufReader::new(file).lines().map(|l| l.unwrap()).collect();

    assert_eq!(lines[0], r#"{"schema_version":1}"#);
    assert_eq!(lines.len(), 2, "header + one event");
    let _: Event = serde_json::from_str(&lines[1]).expect("second line should be valid event JSON");
}

#[tokio::test]
async fn background_task_flushes_after_50ms_without_further_appends() {
    use forge_core::{Event, MessageId};
    use chrono::Utc;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    let mut log = EventLog::create(&path).await.unwrap();

    let event = Event::AssistantDelta {
        id: MessageId::new(),
        at: Utc::now(),
        delta: "persisted".to_string(),
    };
    log.append(&event).await.unwrap();

    // No further appends — background flush task should flush within 50ms.
    tokio::time::sleep(tokio::time::Duration::from_millis(75)).await;

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("persisted"), "background flush should write to disk within 75ms");
}

#[tokio::test]
async fn close_flushes_final_buffered_event() {
    use forge_core::{Event, MessageId};
    use chrono::Utc;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    let mut log = EventLog::create(&path).await.unwrap();

    let event = Event::AssistantDelta {
        id: MessageId::new(),
        at: Utc::now(),
        delta: "final".to_string(),
    };
    log.append(&event).await.unwrap();
    log.close().await.unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("final"), "close() must flush the final buffered event");
}
