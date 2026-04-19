use forge_core::{Transcript, MAX_LINE_BYTES};
use std::io::Write;
use tempfile::TempDir;

/// Write a transcript-style jsonl file containing one line whose total serialized
/// size exceeds MAX_LINE_BYTES (no schema header — Transcript::from_file reads raw).
fn write_oversize_transcript(path: &std::path::Path, oversize_bytes: usize) {
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(br#"{"type":"assistant_delta","id":"m","at":"1970-01-01T00:00:00Z","delta":""#)
        .unwrap();
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

#[test]
fn from_file_rejects_line_exceeding_max_line_bytes() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("transcript.jsonl");
    write_oversize_transcript(&path, MAX_LINE_BYTES + 1024);

    let result = Transcript::from_file(&path);

    let err = result.expect_err("Transcript::from_file must reject oversized line");
    let msg = err.to_string();
    assert!(
        msg.contains("exceeds"),
        "error must mention the cap, got: {msg}"
    );
}
