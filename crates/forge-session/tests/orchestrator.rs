use forge_core::Event;
use forge_ipc::{
    ClientInfo, Hello, IpcEvent, IpcMessage, SendUserMessage, Subscribe, ToolCallApproved,
    PROTO_VERSION,
};
use forge_providers::{ChatBlock, MockProvider};
use forge_session::{server::serve_with_session, session::Session};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::net::UnixStream;

// Script 1: provider returns a text delta, then a tool call, then Done("tool_use")
const SCRIPT_INITIAL: &str = r#"{"delta":"Hi there. "}
{"tool_call":{"name":"fs.read","args":{"path":"readme.txt"}}}
{"done":"tool_use"}
"#;

// Script 2: provider receives tool result, returns continuation text, then Done("end_turn")
const SCRIPT_CONTINUATION: &str = r#"{"delta":"Here is the file content."}
{"done":"end_turn"}
"#;

async fn connect_with_retry(path: &std::path::PathBuf) -> UnixStream {
    for _ in 0..20 {
        match UnixStream::connect(path).await {
            Ok(s) => return s,
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(10)).await,
        }
    }
    UnixStream::connect(path)
        .await
        .expect("server did not start in time")
}

async fn do_handshake(stream: &mut UnixStream) {
    let hello = IpcMessage::Hello(Hello {
        proto: PROTO_VERSION,
        client: ClientInfo {
            kind: "test".into(),
            pid: std::process::id(),
            user: "tester".into(),
        },
    });
    forge_ipc::write_frame(stream, &hello).await.unwrap();
    let response = forge_ipc::read_frame(stream).await.unwrap();
    assert!(
        matches!(response, IpcMessage::HelloAck(_)),
        "expected HelloAck"
    );
}

fn extract_event(msg: &IpcMessage) -> Option<Event> {
    if let IpcMessage::Event(IpcEvent { event, .. }) = msg {
        serde_json::from_value::<Event>(event.clone()).ok()
    } else {
        None
    }
}

/// Full turn with tool call: verifies correct event sequence end-to-end.
///
/// Expected sequence:
///   UserMessage
///   AssistantMessage { stream_finalised: false }   ← opened before first chunk
///   AssistantDelta("Hi there. ")
///   ToolCallStarted { tool: "fs.read" }
///   ToolCallApprovalRequested   ← approval gate fires for non-whitelisted tool
///   ToolCallApproved            ← logged after client approves
///   ToolCallCompleted
///   AssistantMessage { stream_finalised: true }    ← finalised when Done("tool_use") arrives
///   AssistantMessage { stream_finalised: false }   ← continuation turn opens
///   AssistantDelta("Here is the file content.")
///   AssistantMessage { stream_finalised: true }    ← continuation finalised
#[tokio::test]
async fn full_turn_with_tool_call_emits_correct_event_sequence() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let sock_path = dir.path().join("test.sock");

    let session = Arc::new(Session::create(log_path).await.unwrap());
    let provider = Arc::new(
        MockProvider::from_responses(vec![
            SCRIPT_INITIAL.to_string(),
            SCRIPT_CONTINUATION.to_string(),
        ])
        .unwrap(),
    );

    let server_session = Arc::clone(&session);
    let server_provider = Arc::clone(&provider);
    let server_sock = sock_path.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            server_session,
            server_provider,
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    });

    let mut stream = connect_with_retry(&sock_path).await;
    do_handshake(&mut stream).await;

    let sub = IpcMessage::Subscribe(Subscribe { since: 0 });
    forge_ipc::write_frame(&mut stream, &sub).await.unwrap();

    let send = IpcMessage::SendUserMessage(SendUserMessage {
        text: "hello".to_string(),
    });
    forge_ipc::write_frame(&mut stream, &send).await.unwrap();

    // Collect events; when we see ToolCallApprovalRequested, send approval back.
    // The full turn produces two AssistantMessage(final) events: one when the
    // initial stream ends with Done("tool_use"), and one when the continuation ends.
    let (mut reader, mut writer) = stream.into_split();
    let mut events: Vec<Event> = Vec::new();
    let mut final_count = 0;

    loop {
        let frame = forge_ipc::read_frame(&mut reader).await.unwrap();
        let Some(event) = extract_event(&frame) else {
            continue;
        };

        // Auto-approve tool calls during the test
        if let Event::ToolCallApprovalRequested { ref id, .. } = event {
            let approval = IpcMessage::ToolCallApproved(ToolCallApproved {
                id: id.to_string(),
                scope: "Once".to_string(),
            });
            forge_ipc::write_frame(&mut writer, &approval)
                .await
                .unwrap();
        }

        if matches!(
            event,
            Event::AssistantMessage {
                stream_finalised: true,
                ..
            }
        ) {
            final_count += 1;
        }
        events.push(event);
        // Two scripts → two provider calls → two finalised messages.
        if final_count >= 2 {
            break;
        }
    }

    // Assert the event sequence
    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            Event::UserMessage { .. } => "UserMessage",
            Event::AssistantMessage {
                stream_finalised: false,
                ..
            } => "AssistantMessage(open)",
            Event::AssistantMessage {
                stream_finalised: true,
                ..
            } => "AssistantMessage(final)",
            Event::AssistantDelta { .. } => "AssistantDelta",
            Event::ToolCallStarted { .. } => "ToolCallStarted",
            Event::ToolCallApprovalRequested { .. } => "ToolCallApprovalRequested",
            Event::ToolCallApproved { .. } => "ToolCallApproved",
            Event::ToolCallCompleted { .. } => "ToolCallCompleted",
            _ => "Other",
        })
        .collect();

    assert_eq!(
        kinds,
        vec![
            "UserMessage",
            "AssistantMessage(open)", // opened before first chunk
            "AssistantDelta",
            "ToolCallStarted",
            "ToolCallApprovalRequested",
            "ToolCallApproved",
            "ToolCallCompleted",
            "AssistantMessage(final)", // finalised when Done("tool_use") arrives
            "AssistantMessage(open)",  // continuation turn opens
            "AssistantDelta",
            "AssistantMessage(final)", // continuation turn finalised
        ],
        "event sequence mismatch: got {kinds:?}"
    );

    // Verify text delta content
    let deltas: Vec<&str> = events
        .iter()
        .filter_map(|e| {
            if let Event::AssistantDelta { delta, .. } = e {
                Some(delta.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(deltas, vec!["Hi there. ", "Here is the file content."]);

    // Verify tool call details
    let tool_name = events.iter().find_map(|e| {
        if let Event::ToolCallStarted { tool, .. } = e {
            Some(tool.as_str())
        } else {
            None
        }
    });
    assert_eq!(tool_name, Some("fs.read"));

    // Verify tool result was fed back: the second provider call happened
    // (proven by receiving continuation events after ToolCallCompleted)
    let continuation_delta_idx = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| matches!(e, Event::AssistantDelta { .. }).then_some(i))
        .nth(1); // second delta
    let tool_completed_idx = events
        .iter()
        .position(|e| matches!(e, Event::ToolCallCompleted { .. }));
    assert!(
        continuation_delta_idx > tool_completed_idx,
        "continuation delta must follow tool completion"
    );
}

/// Verify approval gate fires for non-whitelisted tools (and NOT for the turn to continue
/// without approval being sent — i.e., the orchestrator pauses until client responds).
#[tokio::test]
async fn approval_gate_fires_and_blocks_until_client_approves() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let sock_path = dir.path().join("test2.sock");

    let session = Arc::new(Session::create(log_path).await.unwrap());
    // Single-script provider: tool call only (no continuation needed for this test)
    let provider =
        Arc::new(MockProvider::from_responses(vec![SCRIPT_INITIAL.to_string()]).unwrap());

    let server_session = Arc::clone(&session);
    let server_provider = Arc::clone(&provider);
    let server_sock = sock_path.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            server_session,
            server_provider,
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    });

    let mut stream = connect_with_retry(&sock_path).await;
    do_handshake(&mut stream).await;
    forge_ipc::write_frame(&mut stream, &IpcMessage::Subscribe(Subscribe { since: 0 }))
        .await
        .unwrap();
    forge_ipc::write_frame(
        &mut stream,
        &IpcMessage::SendUserMessage(SendUserMessage {
            text: "hi".to_string(),
        }),
    )
    .await
    .unwrap();

    let (mut reader, mut writer) = stream.into_split();

    // Read events until ToolCallApprovalRequested is received
    let mut saw_approval_requested = false;
    let mut tool_call_id = String::new();
    for _ in 0..10 {
        let frame = forge_ipc::read_frame(&mut reader).await.unwrap();
        if let Some(Event::ToolCallApprovalRequested { id, .. }) = extract_event(&frame) {
            saw_approval_requested = true;
            tool_call_id = id.to_string();
            break;
        }
    }
    assert!(
        saw_approval_requested,
        "expected ToolCallApprovalRequested to be emitted"
    );

    // Now send approval
    forge_ipc::write_frame(
        &mut writer,
        &IpcMessage::ToolCallApproved(ToolCallApproved {
            id: tool_call_id,
            scope: "Once".to_string(),
        }),
    )
    .await
    .unwrap();

    // Verify ToolCallCompleted arrives after approval
    let mut saw_completed = false;
    for _ in 0..10 {
        let frame = forge_ipc::read_frame(&mut reader).await.unwrap();
        if let Some(Event::ToolCallCompleted { .. }) = extract_event(&frame) {
            saw_completed = true;
            break;
        }
    }
    assert!(saw_completed, "expected ToolCallCompleted after approval");
}

/// With --auto-approve-unsafe, tool calls proceed without client approval.
/// ToolCallApproved { by: Auto } must be emitted; ToolCallApprovalRequested must not.
#[tokio::test]
async fn auto_approve_skips_approval_gate_and_emits_auto_approved() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let sock_path = dir.path().join("auto_approve.sock");

    let session = Arc::new(Session::create(log_path).await.unwrap());
    let provider = Arc::new(
        MockProvider::from_responses(vec![
            SCRIPT_INITIAL.to_string(),
            SCRIPT_CONTINUATION.to_string(),
        ])
        .unwrap(),
    );

    let server_session = Arc::clone(&session);
    let server_provider = Arc::clone(&provider);
    let server_sock = sock_path.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            server_session,
            server_provider,
            true,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    });

    let mut stream = connect_with_retry(&sock_path).await;
    do_handshake(&mut stream).await;
    forge_ipc::write_frame(&mut stream, &IpcMessage::Subscribe(Subscribe { since: 0 }))
        .await
        .unwrap();
    forge_ipc::write_frame(
        &mut stream,
        &IpcMessage::SendUserMessage(SendUserMessage {
            text: "hello".to_string(),
        }),
    )
    .await
    .unwrap();

    // Collect events until the turn completes (two finalised AssistantMessages).
    // No client approval is sent — the session must complete without it.
    let mut events: Vec<Event> = Vec::new();
    let mut final_count = 0;
    let (reader, _writer) = stream.into_split();
    // Use a timeout so the test fails fast if the session blocks waiting for approval.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut reader = reader;

    loop {
        let frame = tokio::time::timeout_at(deadline, forge_ipc::read_frame(&mut reader))
            .await
            .expect("timed out — session may be blocked waiting for approval")
            .unwrap();
        let Some(event) = extract_event(&frame) else {
            continue;
        };
        if matches!(
            event,
            Event::AssistantMessage {
                stream_finalised: true,
                ..
            }
        ) {
            final_count += 1;
        }
        events.push(event);
        if final_count >= 2 {
            break;
        }
    }

    let kinds: Vec<&str> = events
        .iter()
        .map(|e| match e {
            Event::UserMessage { .. } => "UserMessage",
            Event::AssistantMessage {
                stream_finalised: false,
                ..
            } => "AssistantMessage(open)",
            Event::AssistantMessage {
                stream_finalised: true,
                ..
            } => "AssistantMessage(final)",
            Event::AssistantDelta { .. } => "AssistantDelta",
            Event::ToolCallStarted { .. } => "ToolCallStarted",
            Event::ToolCallApprovalRequested { .. } => "ToolCallApprovalRequested",
            Event::ToolCallApproved { .. } => "ToolCallApproved",
            Event::ToolCallCompleted { .. } => "ToolCallCompleted",
            _ => "Other",
        })
        .collect();

    // ToolCallApprovalRequested must NOT appear; ToolCallApproved must appear.
    assert!(
        !kinds.contains(&"ToolCallApprovalRequested"),
        "auto-approve must not emit ToolCallApprovalRequested; got: {kinds:?}"
    );
    assert!(
        kinds.contains(&"ToolCallApproved"),
        "auto-approve must emit ToolCallApproved; got: {kinds:?}"
    );
    assert!(
        kinds.contains(&"ToolCallCompleted"),
        "auto-approve must emit ToolCallCompleted; got: {kinds:?}"
    );

    // Verify ToolCallApproved carries ApprovalSource::Auto.
    use forge_core::ApprovalSource;
    let auto_approved = events.iter().any(|e| {
        matches!(
            e,
            Event::ToolCallApproved {
                by: ApprovalSource::Auto,
                ..
            }
        )
    });
    assert!(auto_approved, "ToolCallApproved must have by=Auto");
}

/// Verify tool result is included in the next ChatRequest to the provider.
/// Proven by: the continuation response arrives (provider was called a second time).
#[tokio::test]
async fn tool_result_fed_back_to_provider_in_continuation() {
    let dir = TempDir::new().unwrap();
    let log_path = dir.path().join("events.jsonl");
    let sock_path = dir.path().join("test3.sock");

    let session = Arc::new(Session::create(log_path).await.unwrap());
    let provider = Arc::new(
        MockProvider::from_responses(vec![
            SCRIPT_INITIAL.to_string(),
            SCRIPT_CONTINUATION.to_string(),
        ])
        .unwrap(),
    );

    let server_session = Arc::clone(&session);
    let server_provider = Arc::clone(&provider);
    let server_sock = sock_path.clone();
    tokio::spawn(async move {
        serve_with_session(
            &server_sock,
            server_session,
            server_provider,
            false,
            false,
            None,
            None,
        )
        .await
        .unwrap();
    });

    let mut stream = connect_with_retry(&sock_path).await;
    do_handshake(&mut stream).await;
    forge_ipc::write_frame(&mut stream, &IpcMessage::Subscribe(Subscribe { since: 0 }))
        .await
        .unwrap();
    forge_ipc::write_frame(
        &mut stream,
        &IpcMessage::SendUserMessage(SendUserMessage {
            text: "hello".to_string(),
        }),
    )
    .await
    .unwrap();

    let (mut reader, mut writer) = stream.into_split();
    let mut events: Vec<Event> = Vec::new();
    let mut final_count = 0;

    loop {
        let frame = forge_ipc::read_frame(&mut reader).await.unwrap();
        let Some(event) = extract_event(&frame) else {
            continue;
        };
        if let Event::ToolCallApprovalRequested { ref id, .. } = event {
            forge_ipc::write_frame(
                &mut writer,
                &IpcMessage::ToolCallApproved(ToolCallApproved {
                    id: id.to_string(),
                    scope: "Once".to_string(),
                }),
            )
            .await
            .unwrap();
        }
        if matches!(
            event,
            Event::AssistantMessage {
                stream_finalised: true,
                ..
            }
        ) {
            final_count += 1;
        }
        events.push(event);
        // Two scripts → two provider calls → two finalised messages.
        if final_count >= 2 {
            break;
        }
    }

    // The second AssistantDelta ("Here is the file content.") proves the provider
    // was called a second time — i.e., tool result was fed back.
    let continuation_text = events.iter().find_map(|e| {
        if let Event::AssistantDelta { delta, .. } = e {
            if delta.contains("file content") {
                Some(delta.as_str())
            } else {
                None
            }
        } else {
            None
        }
    });
    assert_eq!(
        continuation_text,
        Some("Here is the file content."),
        "continuation response from second provider call not received"
    );

    // Check that the second ChatRequest included a ToolResult block.
    // We verify this indirectly: MockProvider::from_responses() tracks requests;
    // assert the second request contains a ToolResult block.
    let requests = provider.recorded_requests();
    assert_eq!(requests.len(), 2, "provider should have been called twice");
    let second_req = &requests[1];
    let has_tool_result = second_req.messages.iter().any(|m| {
        m.content
            .iter()
            .any(|b| matches!(b, ChatBlock::ToolResult { .. }))
    });
    assert!(
        has_tool_result,
        "second ChatRequest should contain a ToolResult block"
    );
}
