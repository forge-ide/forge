use chrono::Utc;
use forge_core::{Event, MessageId, Transcript};
use tempfile::NamedTempFile;

#[test]
fn roundtrip_100_events() {
    let mut transcript = Transcript::new();

    for i in 0..100u32 {
        transcript.append(Event::AssistantDelta {
            id: MessageId::new(),
            at: Utc::now(),
            // F-112: `Arc<str>` from owned `String` via `Into`.
            delta: format!("delta {i}").into(),
        });
    }

    let file = NamedTempFile::new().unwrap();
    transcript.to_file(file.path()).unwrap();

    let loaded = Transcript::from_file(file.path()).unwrap();

    assert_eq!(transcript.events(), loaded.events());
}
