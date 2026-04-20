use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;

use crate::event::Event;
use crate::event_log::MAX_LINE_BYTES;
use crate::ids::MessageId;
use crate::{ForgeError, Result};

/// F-143: filter a `(seq, event)` replay stream by honouring
/// [`Event::MessageSuperseded`] markers.
///
/// The event log is append-only — regenerated messages don't overwrite the
/// original. Instead, each `MessageSuperseded { old_id, new_id }` marker
/// tells a replay consumer that `old_id`'s assistant-side events
/// (`AssistantMessage`, `AssistantDelta`) are logically hidden.
///
/// The filter walks the stream once:
///   1. Pre-pass: collect the set of `old_id`s from every `MessageSuperseded`
///      marker.
///   2. Emit-pass: drop any `AssistantMessage`/`AssistantDelta` whose `id` is
///      in that set, plus the `MessageSuperseded` markers themselves (their
///      purpose is already encoded in the filtered output).
///
/// **Tool-call events are intentionally left in place.** `ToolCallStarted`
/// references the owning message via `msg: MessageId`, but the subsequent
/// `ToolCallApprovalRequested` / `ToolCallApproved` / `ToolCallRejected` /
/// `ToolCallCompleted` events reference only the `ToolCallId`. Filtering
/// only `ToolCallStarted` would leave orphan completion events with no
/// matching start. Filtering the full cluster requires tracking
/// `ToolCallId`s from the started events we hide — a larger change
/// deferred to F-144 (Branch variant needs the same bookkeeping).
/// For F-143 (Replace, no-tool-call scenarios are the common case) we
/// accept that a superseded turn's tool events remain visible in replay;
/// the UI can interpret them in context of the surviving
/// `AssistantMessage` for `new_id`.
///
/// Consumers in other contexts (e.g. rebuilding a provider request from
/// history) can call this same helper to walk a coherent, non-superseded
/// transcript.
pub fn apply_superseded(events: Vec<(u64, Event)>) -> Vec<(u64, Event)> {
    let mut superseded: HashSet<MessageId> = HashSet::new();
    for (_, ev) in &events {
        if let Event::MessageSuperseded { old_id, .. } = ev {
            superseded.insert(old_id.clone());
        }
    }
    if superseded.is_empty() {
        return events;
    }
    events
        .into_iter()
        .filter(|(_, ev)| !is_hidden_by(ev, &superseded))
        .collect()
}

fn is_hidden_by(event: &Event, superseded: &HashSet<MessageId>) -> bool {
    match event {
        Event::AssistantMessage { id, .. } | Event::AssistantDelta { id, .. } => {
            superseded.contains(id)
        }
        // See doc-comment on `apply_superseded`: filtering `ToolCallStarted`
        // alone would leave orphaned `ToolCallCompleted` events (keyed by
        // `ToolCallId`, not `MessageId`). Deferred to F-144.
        // Hide the markers themselves: consumers see a clean transcript.
        Event::MessageSuperseded { .. } => true,
        _ => false,
    }
}

#[derive(Debug, Default)]
pub struct Transcript {
    events: Vec<Event>,
}

impl Transcript {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, event: Event) {
        self.events.push(event);
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn to_file(&self, path: &Path) -> Result<()> {
        let mut file = std::fs::File::create(path)?;
        for event in &self.events {
            let line = serde_json::to_string(event)?;
            writeln!(file, "{line}")?;
        }
        Ok(())
    }

    pub fn from_file(path: &Path) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let mut reader = BufReader::new(file);
        let mut events = Vec::new();
        let mut line_num: u64 = 0;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            // Cap per-line reads at MAX_LINE_BYTES. `std::io::Take<R: BufRead>`
            // implements `BufRead`, so `read_until` reuses the standard code path
            // while hard-limiting how many bytes the reader will hand back.
            let mut handle = reader.by_ref().take((MAX_LINE_BYTES as u64) + 1);
            let n = handle.read_until(b'\n', &mut buf)?;
            if n == 0 {
                break; // EOF
            }
            line_num += 1;
            // If we read MAX+1 bytes and the last byte is not '\n', the cap was
            // hit mid-line. Anything that stopped at the cap *with* a trailing
            // newline is exactly MAX content bytes — allowed.
            let ended_with_newline = buf.last() == Some(&b'\n');
            if n > MAX_LINE_BYTES && !ended_with_newline {
                return Err(ForgeError::Other(anyhow::anyhow!(
                    "transcript line {line_num} exceeds {MAX_LINE_BYTES} bytes"
                )));
            }
            let content = if ended_with_newline {
                &buf[..buf.len() - 1]
            } else {
                &buf[..]
            };
            let line = std::str::from_utf8(content).map_err(|_| {
                ForgeError::Other(anyhow::anyhow!(
                    "transcript line {line_num} is not valid UTF-8"
                ))
            })?;
            let event: Event = serde_json::from_str(line)?;
            events.push(event);
        }
        Ok(Self { events })
    }
}

#[cfg(test)]
mod superseded_tests {
    use std::sync::Arc;

    use chrono::Utc;

    use super::*;
    use crate::ids::{MessageId, ProviderId};

    fn assistant(id: &MessageId, text: &str, finalised: bool) -> Event {
        Event::AssistantMessage {
            id: id.clone(),
            provider: ProviderId::new(),
            model: "mock".into(),
            at: Utc::now(),
            stream_finalised: finalised,
            text: Arc::from(text),
            branch_parent: None,
            branch_variant_index: 0,
        }
    }

    fn delta(id: &MessageId, chunk: &str) -> Event {
        Event::AssistantDelta {
            id: id.clone(),
            at: Utc::now(),
            delta: Arc::from(chunk),
        }
    }

    #[test]
    fn passthrough_when_no_supersede_markers() {
        let a = MessageId::new();
        let input = vec![(1, assistant(&a, "hi", true))];
        let out = apply_superseded(input.clone());
        assert_eq!(out.len(), input.len());
    }

    #[test]
    fn removes_superseded_assistant_and_deltas() {
        let a = MessageId::new();
        let b = MessageId::new();
        let input = vec![
            (1, assistant(&a, "", false)),
            (2, delta(&a, "old ")),
            (3, assistant(&a, "old", true)),
            (
                4,
                Event::MessageSuperseded {
                    old_id: a.clone(),
                    new_id: b.clone(),
                },
            ),
            (5, assistant(&b, "", false)),
            (6, delta(&b, "new ")),
            (7, assistant(&b, "new", true)),
        ];
        let out = apply_superseded(input);
        // Expect: only events for `b` — three of them, and no MessageSuperseded marker.
        assert_eq!(out.len(), 3, "got: {:?}", out);
        for (_, ev) in &out {
            match ev {
                Event::AssistantMessage { id, .. } | Event::AssistantDelta { id, .. } => {
                    assert_eq!(id, &b, "only new id should survive");
                }
                Event::MessageSuperseded { .. } => panic!("marker must be hidden"),
                _ => panic!("unexpected event kind"),
            }
        }
    }

    #[test]
    fn preserves_user_messages_and_unrelated_events() {
        let a = MessageId::new();
        let b = MessageId::new();
        let user = MessageId::new();
        let input = vec![
            (
                1,
                Event::UserMessage {
                    id: user.clone(),
                    at: Utc::now(),
                    text: Arc::from("ask"),
                    context: vec![],
                    branch_parent: None,
                },
            ),
            (2, assistant(&a, "old", true)),
            (
                3,
                Event::MessageSuperseded {
                    old_id: a,
                    new_id: b.clone(),
                },
            ),
            (4, assistant(&b, "new", true)),
        ];
        let out = apply_superseded(input);
        // UserMessage + regenerated assistant only.
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0].1, Event::UserMessage { .. }));
        assert!(
            matches!(&out[1].1, Event::AssistantMessage { id, .. } if *id == b),
            "regenerated message must survive"
        );
    }
}
