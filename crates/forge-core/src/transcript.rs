use std::collections::{HashMap, HashSet};
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
/// F-145 extends this with [`Event::BranchDeleted`] markers. A branch
/// deletion tombstones every `AssistantMessage` whose
/// `(branch_parent, branch_variant_index)` pair matches the marker; the
/// corresponding `AssistantDelta`s are also hidden via the id set collected
/// during the pre-pass.
///
/// The filter walks the stream once:
///   1. Pre-pass: collect the set of `old_id`s from every `MessageSuperseded`
///      marker and resolve each `BranchDeleted { parent, variant_index }`
///      to the target `MessageId`. Both sets feed the same hide predicate.
///   2. Emit-pass: drop any `AssistantMessage`/`AssistantDelta` whose `id` is
///      in the combined set, plus the `MessageSuperseded` / `BranchDeleted`
///      markers themselves (their purpose is already encoded in the filtered
///      output).
///
/// **Tool-call events are intentionally left in place.** `ToolCallStarted`
/// references the owning message via `msg: MessageId`, but the subsequent
/// `ToolCallApprovalRequested` / `ToolCallApproved` / `ToolCallRejected` /
/// `ToolCallCompleted` events reference only the `ToolCallId`. Filtering
/// only `ToolCallStarted` would leave orphan completion events with no
/// matching start. Filtering the full cluster requires tracking
/// `ToolCallId`s from the started events we hide — a larger change
/// deferred to a future pass. F-144 (Branch / Fresh) and F-145 (branch
/// deletion) inherit the same limitation.
///
/// Consumers in other contexts (e.g. rebuilding a provider request from
/// history) can call this same helper to walk a coherent, non-superseded
/// transcript.
pub fn apply_superseded(events: Vec<(u64, Event)>) -> Vec<(u64, Event)> {
    // F-572: previously O(N×K) — each `BranchDeleted` marker scanned the
    // entire events vector for a matching `AssistantMessage`. Replaced with
    // a single O(N) pre-pass that builds an index keyed on the resolved
    // `(parent, variant_index)` pair, so each `BranchDeleted` resolves in
    // O(1). Also switched the final filter from
    // `into_iter().filter().collect()` to `Vec::retain` to drop the second
    // log-sized Vec allocation on session resume.
    let mut branch_index: HashMap<(MessageId, u32), MessageId> = HashMap::new();
    for (_, ev) in &events {
        if let Event::AssistantMessage {
            id,
            branch_parent,
            branch_variant_index: idx,
            ..
        } = ev
        {
            // variant 0 is the root: `(id, 0)` keys to `id` itself.
            // N >= 1 is a sibling: `(branch_parent, idx)` keys to `id`.
            // Same `MessageId` may appear multiple times (e.g. streaming
            // delta finalisation); the first occurrence wins, matching the
            // pre-F-572 `break`-on-first-match semantics.
            let key = match branch_parent {
                None => (id.clone(), 0u32),
                Some(parent) => (parent.clone(), *idx),
            };
            branch_index.entry(key).or_insert_with(|| id.clone());
        }
    }

    let mut hidden_ids: HashSet<MessageId> = HashSet::new();
    for (_, ev) in &events {
        match ev {
            Event::MessageSuperseded { old_id, .. } => {
                hidden_ids.insert(old_id.clone());
            }
            Event::BranchDeleted {
                parent,
                variant_index,
            } => {
                if let Some(target) = branch_index.get(&(parent.clone(), *variant_index)) {
                    hidden_ids.insert(target.clone());
                }
            }
            _ => {}
        }
    }

    if hidden_ids.is_empty() {
        // Still need to hide bare BranchDeleted markers that did not resolve
        // (client-side drift). Unresolved markers do nothing observable, so
        // passthrough of an empty-set log is a hot-path fast-return today —
        // keep that contract and let unresolved markers sit in the replay.
        return events;
    }
    let mut events = events;
    events.retain(|(_, ev)| !is_hidden_by(ev, &hidden_ids));
    events
}

fn is_hidden_by(event: &Event, hidden_ids: &HashSet<MessageId>) -> bool {
    match event {
        Event::AssistantMessage { id, .. } | Event::AssistantDelta { id, .. } => {
            hidden_ids.contains(id)
        }
        // See doc-comment on `apply_superseded`: filtering `ToolCallStarted`
        // alone would leave orphaned `ToolCallCompleted` events (keyed by
        // `ToolCallId`, not `MessageId`). Full-cluster filtering is a
        // future pass; F-144 / F-145 inherit the limitation.
        // Hide the markers themselves: consumers see a clean transcript.
        Event::MessageSuperseded { .. } | Event::BranchDeleted { .. } => true,
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

    fn assistant_branch(id: &MessageId, parent: &MessageId, idx: u32, text: &str) -> Event {
        Event::AssistantMessage {
            id: id.clone(),
            provider: ProviderId::new(),
            model: "mock".into(),
            at: Utc::now(),
            stream_finalised: true,
            text: Arc::from(text),
            branch_parent: Some(parent.clone()),
            branch_variant_index: idx,
        }
    }

    #[test]
    fn removes_branch_deleted_sibling() {
        // F-145: BranchDeleted { parent, variant_index } hides the
        // sibling with that (parent, index) pair, without touching other
        // variants under the same parent.
        let root = MessageId::new();
        let sib1 = MessageId::new();
        let sib2 = MessageId::new();
        let input = vec![
            (1, assistant(&root, "root answer", true)),
            (2, assistant_branch(&sib1, &root, 1, "variant 1")),
            (3, assistant_branch(&sib2, &root, 2, "variant 2")),
            (
                4,
                Event::BranchDeleted {
                    parent: root.clone(),
                    variant_index: 1,
                },
            ),
        ];
        let out = apply_superseded(input);
        // Expect: root + variant 2 only; the BranchDeleted marker is hidden.
        assert_eq!(out.len(), 2, "got: {:?}", out);
        let ids: Vec<MessageId> = out
            .iter()
            .filter_map(|(_, ev)| match ev {
                Event::AssistantMessage { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert!(ids.contains(&root));
        assert!(ids.contains(&sib2));
        assert!(!ids.contains(&sib1), "deleted variant must not survive");
    }

    #[test]
    fn removes_branch_deleted_root_variant() {
        // F-145: variant_index 0 resolves to the parent message itself.
        // Deleting the root hides the original assistant message while
        // sibling variants remain.
        let root = MessageId::new();
        let sib1 = MessageId::new();
        let input = vec![
            (1, assistant(&root, "root", true)),
            (2, assistant_branch(&sib1, &root, 1, "variant 1")),
            (
                3,
                Event::BranchDeleted {
                    parent: root.clone(),
                    variant_index: 0,
                },
            ),
        ];
        let out = apply_superseded(input);
        // Only the sibling survives.
        assert_eq!(out.len(), 1);
        assert!(matches!(&out[0].1, Event::AssistantMessage { id, .. } if *id == sib1));
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
