use forge_core::{read_since, MAX_LINE_BYTES};
use std::io::Write;
use tempfile::TempDir;

/// Write a crafted events.jsonl with a valid schema header plus one event line
/// whose serialized JSON exceeds MAX_LINE_BYTES. Streams the oversize content
/// byte-by-byte so the test itself doesn't need the entire line resident in RAM.
fn write_oversize_events_jsonl(path: &std::path::Path, oversize_bytes: usize) {
    let mut f = std::fs::File::create(path).unwrap();
    // schema header
    f.write_all(br#"{"schema_version":1}"#).unwrap();
    f.write_all(b"\n").unwrap();
    // one oversized event line: well-formed JSON prefix, then a huge run of 'a'
    // inside a string field, then closing. No newline char is ever emitted
    // until the very end — the reader must refuse before reaching it.
    f.write_all(br#"{"type":"assistant_delta","id":"m","at":"1970-01-01T00:00:00Z","delta":""#)
        .unwrap();
    // Stream garbage content in chunks; total content size = oversize_bytes.
    let chunk = vec![b'a'; 64 * 1024];
    let mut remaining = oversize_bytes;
    while remaining > 0 {
        let take = remaining.min(chunk.len());
        f.write_all(&chunk[..take]).unwrap();
        remaining -= take;
    }
    f.write_all(br#""}"#).unwrap();
    f.write_all(b"\n").unwrap();
}

#[tokio::test]
async fn read_since_rejects_line_exceeding_max_line_bytes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    // Oversized line content by 1 KiB above the cap — enough to trigger rejection.
    write_oversize_events_jsonl(&path, MAX_LINE_BYTES + 1024);

    let result = read_since(&path, 0).await;

    let err = result.expect_err("read_since must reject oversized line");
    let msg = err.to_string();
    assert!(
        msg.contains("exceeds"),
        "error must mention the cap, got: {msg}"
    );
}

/// Attacker crafts a file whose header is valid but the first event line has
/// no trailing newline *at all* — the original OOM primitive described in the
/// finding. The reader must refuse the file instead of buffering it whole.
#[tokio::test]
async fn read_since_rejects_oversized_line_with_no_trailing_newline() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(br#"{"schema_version":1}"#).unwrap();
    f.write_all(b"\n").unwrap();
    let chunk = vec![b'x'; 64 * 1024];
    // Write (MAX + 256 KiB) raw bytes — no newline anywhere after the header.
    let mut remaining = MAX_LINE_BYTES + 256 * 1024;
    while remaining > 0 {
        let take = remaining.min(chunk.len());
        f.write_all(&chunk[..take]).unwrap();
        remaining -= take;
    }
    drop(f);

    let result = read_since(&path, 0).await;

    let err = result.expect_err("read_since must reject unterminated oversize line");
    assert!(err.to_string().contains("exceeds"));
}

#[tokio::test]
async fn read_since_accepts_legitimate_small_line() {
    use chrono::Utc;
    use forge_core::{Event, EventLog, MessageId};

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("events.jsonl");
    let mut log = EventLog::create(&path).await.unwrap();

    // A legitimate delta well under the cap must still round-trip through read_since.
    let event = Event::AssistantDelta {
        id: MessageId::new(),
        at: Utc::now(),
        delta: "hello".into(),
    };
    log.append(&event).await.unwrap();
    log.close().await.unwrap();

    let events = read_since(&path, 0)
        .await
        .expect("legitimate line must read");
    assert_eq!(events.len(), 1);
}
