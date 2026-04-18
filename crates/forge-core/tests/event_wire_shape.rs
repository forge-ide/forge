//! Golden wire-shape test for `forge_core::Event`.
//!
//! Pins the JSON serialization of each variant the TS adapter
//! (`web/packages/app/src/ipc/events.ts`) depends on. The same JSON literals
//! appear in `web/packages/app/src/ipc/events.test.ts` as adapter test inputs,
//! so if this file is the source-of-truth for the wire contract.
//!
//! When this test fails, the wire shape changed — update both sides together.
//! When you add a new `Event` variant, add a golden case here before writing
//! adapter code in TS. This satisfies the F-037 DoD requirement that adapter
//! tests be "seeded from real `forged` output (via bridge_e2e.rs or a
//! comparable harness)": the harness is this file — every shape the adapter
//! consumes round-trips through real `serde_json::to_value` on a real
//! `forge_core::Event`, not a hand-crafted object literal.
//!
//! Canonical field values are chosen to be deterministic (no timestamps
//! derived from `Utc::now()`, no random IDs). IDs deserialize from bare JSON
//! strings because `MessageId(String)` / `ToolCallId(String)` have private
//! fields; `serde_json::from_value` is the only stable constructor.

use chrono::{DateTime, Utc};
use forge_core::{ApprovalPreview, Event, MessageId, ToolCallId};
use serde_json::{json, Value};

fn fixed_time() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-04-18T10:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn msg_id(s: &str) -> MessageId {
    serde_json::from_value(Value::String(s.to_string())).unwrap()
}

fn tool_call_id(s: &str) -> ToolCallId {
    serde_json::from_value(Value::String(s.to_string())).unwrap()
}

fn assert_wire_eq(event: Event, expected: Value) {
    let actual = serde_json::to_value(&event).expect("Event serializes");
    assert_eq!(
        actual, expected,
        "wire shape drifted — update TS adapter golden fixtures too"
    );
}

#[test]
fn user_message_wire_shape() {
    assert_wire_eq(
        Event::UserMessage {
            id: msg_id("mid-1"),
            at: fixed_time(),
            text: "hello".into(),
            context: vec![],
            branch_parent: None,
        },
        json!({
            "type": "user_message",
            "id": "mid-1",
            "at": "2026-04-18T10:00:00Z",
            "text": "hello",
            "context": [],
            "branch_parent": null,
        }),
    );
}

#[test]
fn assistant_delta_wire_shape() {
    assert_wire_eq(
        Event::AssistantDelta {
            id: msg_id("mid-3"),
            at: fixed_time(),
            delta: "partial ".into(),
        },
        json!({
            "type": "assistant_delta",
            "id": "mid-3",
            "at": "2026-04-18T10:00:00Z",
            "delta": "partial ",
        }),
    );
}

#[test]
fn assistant_message_finalised_wire_shape() {
    use forge_core::ProviderId;
    let provider: ProviderId = serde_json::from_value(Value::String("mock".into())).unwrap();
    assert_wire_eq(
        Event::AssistantMessage {
            id: msg_id("mid-2"),
            provider,
            model: "mock-1".into(),
            at: fixed_time(),
            stream_finalised: true,
            text: "hi there".into(),
            branch_parent: None,
            branch_variant_index: 0,
        },
        json!({
            "type": "assistant_message",
            "id": "mid-2",
            "provider": "mock",
            "model": "mock-1",
            "at": "2026-04-18T10:00:00Z",
            "stream_finalised": true,
            "text": "hi there",
            "branch_parent": null,
            "branch_variant_index": 0,
        }),
    );
}

#[test]
fn assistant_message_stream_open_wire_shape() {
    use forge_core::ProviderId;
    let provider: ProviderId = serde_json::from_value(Value::String("mock".into())).unwrap();
    // The orchestrator emits this at stream-start (text empty, stream_finalised:false).
    // The adapter MUST return null for it so the first AssistantDelta is what creates
    // the streaming turn.
    assert_wire_eq(
        Event::AssistantMessage {
            id: msg_id("mid-open"),
            provider,
            model: "mock-1".into(),
            at: fixed_time(),
            stream_finalised: false,
            text: String::new(),
            branch_parent: None,
            branch_variant_index: 0,
        },
        json!({
            "type": "assistant_message",
            "id": "mid-open",
            "provider": "mock",
            "model": "mock-1",
            "at": "2026-04-18T10:00:00Z",
            "stream_finalised": false,
            "text": "",
            "branch_parent": null,
            "branch_variant_index": 0,
        }),
    );
}

#[test]
fn tool_call_started_wire_shape() {
    assert_wire_eq(
        Event::ToolCallStarted {
            id: tool_call_id("tc-1"),
            msg: msg_id("mid-3"),
            tool: "fs.read".into(),
            args: json!({ "path": "readable.txt" }),
            at: fixed_time(),
            parallel_group: None,
        },
        json!({
            "type": "tool_call_started",
            "id": "tc-1",
            "msg": "mid-3",
            "tool": "fs.read",
            "args": { "path": "readable.txt" },
            "at": "2026-04-18T10:00:00Z",
            "parallel_group": null,
        }),
    );
}

#[test]
fn tool_call_started_with_parallel_group_wire_shape() {
    assert_wire_eq(
        Event::ToolCallStarted {
            id: tool_call_id("tc-2"),
            msg: msg_id("mid-3"),
            tool: "fs.read".into(),
            args: json!({ "path": "a.txt" }),
            at: fixed_time(),
            parallel_group: Some(7),
        },
        json!({
            "type": "tool_call_started",
            "id": "tc-2",
            "msg": "mid-3",
            "tool": "fs.read",
            "args": { "path": "a.txt" },
            "at": "2026-04-18T10:00:00Z",
            "parallel_group": 7,
        }),
    );
}

#[test]
fn tool_call_approval_requested_wire_shape() {
    assert_wire_eq(
        Event::ToolCallApprovalRequested {
            id: tool_call_id("tc-9"),
            preview: ApprovalPreview {
                description: "Edit file /src/foo.ts".into(),
            },
        },
        json!({
            "type": "tool_call_approval_requested",
            "id": "tc-9",
            "preview": { "description": "Edit file /src/foo.ts" },
        }),
    );
}

#[test]
fn tool_call_rejected_wire_shape() {
    assert_wire_eq(
        Event::ToolCallRejected {
            id: tool_call_id("tc-rej-1"),
            reason: Some("user denied".into()),
        },
        json!({
            "type": "tool_call_rejected",
            "id": "tc-rej-1",
            "reason": "user denied",
        }),
    );
}

#[test]
fn tool_call_rejected_without_reason_wire_shape() {
    assert_wire_eq(
        Event::ToolCallRejected {
            id: tool_call_id("tc-rej-2"),
            reason: None,
        },
        json!({
            "type": "tool_call_rejected",
            "id": "tc-rej-2",
            "reason": null,
        }),
    );
}

#[test]
fn tool_call_completed_wire_shape() {
    assert_wire_eq(
        Event::ToolCallCompleted {
            id: tool_call_id("tc-7"),
            result: json!({ "ok": true, "bytes": 42 }),
            duration_ms: 12,
            at: fixed_time(),
        },
        json!({
            "type": "tool_call_completed",
            "id": "tc-7",
            "result": { "ok": true, "bytes": 42 },
            "duration_ms": 12,
            "at": "2026-04-18T10:00:00Z",
        }),
    );
}
