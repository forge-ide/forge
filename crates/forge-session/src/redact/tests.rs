use forge_api::TurnId;
use forge_api::events::{
    AgentMessageBody, HookRunBody, PolicyDecisionBody, TerminalOutputBody, ToolCallBody,
    ToolResultBody, Verdict,
};

use super::*;

fn token() -> String {
    format!("ghp_{}", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8")
}

fn json(payload: &EventPayload) -> String {
    serde_json::to_string(payload).unwrap()
}

fn out(terminal: &str, text: &str) -> EventPayload {
    EventPayload::TerminalOutput(TerminalOutputBody {
        terminal_id: terminal.into(),
        chunk: text.into(),
    })
}

fn result(terminal: &str) -> EventPayload {
    EventPayload::ToolResult(ToolResultBody {
        tool_call_id: terminal.into(),
        status: "completed".into(),
        output: None,
    })
}

fn delta(message: &str, text: &str) -> EventPayload {
    EventPayload::AgentMessage(AgentMessageBody {
        turn_id: TurnId::new(),
        message_id: message.into(),
        delta: text.into(),
    })
}

fn unrelated() -> Vec<EventPayload> {
    vec![
        EventPayload::HookRun(HookRunBody {
            hook: "h".into(),
            event: "tool.pre".into(),
            verdict: None,
            duration_ms: 1,
            exit_code: 0,
        }),
        EventPayload::PolicyDecision(PolicyDecisionBody {
            tool_call_id: "tc_other".into(),
            verdict: Verdict::Allow,
            reason: "r".into(),
            source: "mode:auto".into(),
        }),
        result("tc_other"),
    ]
}

/// One stream's text as recorded, with `|` for each event of another kind.
fn stream(events: &[EventPayload], id: &str) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            EventPayload::TerminalOutput(b) if b.terminal_id == id => Some(b.chunk.clone()),
            EventPayload::AgentMessage(b) if b.message_id == id => Some(b.delta.clone()),
            EventPayload::ToolResult(b) if b.tool_call_id == id => Some("|".into()),
            _ => None,
        })
        .collect()
}

fn redacted(events: &mut [EventPayload]) -> String {
    let (redactor, session) = (SecretRedactor::new(), SessionId::new());
    for event in events.iter_mut() {
        redactor.redact(&session, event);
    }
    events.iter().map(json).collect()
}

fn builtin() -> Holdback {
    Holdback::new(Arc::new(SecretScanner::default()))
}

#[test]
fn every_string_and_key_of_every_event_is_redacted_with_the_sessions_values() {
    let redactor = SecretRedactor::new();
    let (session, other) = (SessionId::new(), SessionId::new());
    redactor.set(
        &session,
        Arc::new(SecretScanner::new(vec!["injected-value-1".into()])),
    );
    let mut call = EventPayload::ToolCall(ToolCallBody {
        tool_call_id: "t".into(),
        action: Action::Mcp {
            server: "s".into(),
            tool: "x".into(),
            args: serde_json::json!({
                "deep": [{ "header": format!("token {}", token()) }],
                token(): "as a key",
            }),
        },
        title: "injected-value-1".into(),
    });
    let mut copy = call.clone();
    redactor.redact(&session, &mut call);
    let shown = json(&call);
    assert!(
        !shown.contains(&token()) && !shown.contains("injected-value-1"),
        "{shown}"
    );
    assert_eq!(
        shown.matches("[redacted:github-token]").count(),
        2,
        "{shown}"
    );
    // Another session doesn't know this session's values, only the built-in patterns.
    redactor.redact(&other, &mut copy);
    let shown = json(&copy);
    assert!(
        !shown.contains(&token()) && shown.contains("injected-value-1"),
        "{shown}"
    );
    assert!(redactor.has(&session) && !redactor.has(&other));
    redactor.remove(&session);
    assert!(!redactor.has(&session));
}

#[test]
fn approval_actions_go_through_the_same_walk() {
    let scanner = SecretScanner::new(vec!["hunter2-injected".into()]);
    let mcp = Action::Mcp {
        server: "s".into(),
        tool: "t".into(),
        args: serde_json::json!({ "auth": token(), "pw": "hunter2-injected" }),
    };
    let shown = format!("{:?}", redact_action(&scanner, &mcp));
    assert!(
        !shown.contains(&token()) && !shown.contains("hunter2"),
        "{shown}"
    );
    let plain = Action::Exec {
        argv: vec!["ls".into()],
        cwd: ".".into(),
    };
    assert_eq!(redact_action(&scanner, &plain), plain);
}

#[test]
fn unrelated_events_never_split_a_held_secret() {
    let hold = builtin();
    let token = token();
    let mut events = Vec::new();
    events.extend(hold.take(out("tc_1", "building\ntoken: ")));
    events.extend(hold.take(out("tc_1", &token[..10])));
    for e in unrelated() {
        events.extend(hold.take(e));
    }
    events.extend(hold.take(out("tc_1", &format!("{}\ndone", &token[10..]))));
    events.extend(hold.end_turn());
    // The terminal keeps its tail past the end of the turn, until it ends.
    assert_eq!(
        stream(&events, "tc_1"),
        format!("building\ntoken: {token}\n")
    );
    events.extend(hold.take(result("tc_1")));
    assert_eq!(
        stream(&events, "tc_1"),
        format!("building\ntoken: {token}\ndone|")
    );
    let shown = redacted(&mut events);
    assert!(!shown.contains("ghp_"), "{shown}");
    assert_eq!(
        shown.matches("[redacted:github-token]").count(),
        1,
        "{shown}"
    );
}

#[test]
fn a_key_block_across_chunks_becomes_one_placeholder() {
    let hold = builtin();
    let body: String = (0..120)
        .map(|i| format!("MIIEowIBAAKCAQEA{i:04}qwertyuiopasdfghjklzxcvbnm1234567890ABCD\n"))
        .collect();
    let pem = format!("-----BEGIN RSA PRIVATE KEY-----\n{body}-----END RSA PRIVATE KEY-----\n");
    let text = format!("$ cat id_rsa\n{pem}$ next\n");
    let mut events = Vec::new();
    for (i, piece) in text.as_bytes().chunks(512).enumerate() {
        events.extend(hold.take(out("tc_1", std::str::from_utf8(piece).unwrap())));
        if i == 3 {
            for e in unrelated() {
                events.extend(hold.take(e));
            }
        }
    }
    events.extend(hold.take(result("tc_1")));
    let recorded = stream(&events, "tc_1");
    assert_eq!(
        recorded,
        format!("$ cat id_rsa\n{KEY_PLACEHOLDER}\n$ next\n|")
    );
    let shown = redacted(&mut events);
    assert!(
        !shown.contains("MIIE") && !shown.contains("-----END"),
        "{shown}"
    );
}

#[test]
fn an_unfinished_key_block_drops_the_rest_of_its_stream() {
    let hold = builtin();
    let mut events = hold.take(out("tc_1", "-----BEGIN PRIVATE KEY-----\nMIIE"));
    events.extend(hold.take(out("tc_1", "abcd\nefgh\n")));
    events.extend(hold.take(result("tc_1")));
    assert_eq!(stream(&events, "tc_1"), format!("{KEY_PLACEHOLDER}|"));
    // A block whole in one chunk is left to the redactor.
    let mut whole = builtin().take(out(
        "tc_2",
        "a\n-----BEGIN EC PRIVATE KEY-----\nMHc\n-----END EC PRIVATE KEY-----\nb\n",
    ));
    let shown = redacted(&mut whole);
    assert_eq!(
        shown.matches("[redacted:private-key]").count(),
        1,
        "{shown}"
    );
    assert!(!shown.contains("MHc"), "{shown}");
}

#[test]
fn messages_hold_their_trailing_word_until_another_message_or_the_turn_ends() {
    let hold = builtin();
    let mut events = hold.take(delta("m1", "Use the key sk-ant-"));
    for e in unrelated() {
        events.extend(hold.take(e));
    }
    events.extend(hold.take(delta("m1", "api03-abcdefghijklmnopqrst and")));
    events.extend(hold.take(delta("m2", "Next")));
    assert_eq!(
        stream(&events, "m1"),
        "Use the key sk-ant-api03-abcdefghijklmnopqrst and"
    );
    events.extend(hold.end_turn());
    assert_eq!(stream(&events, "m2"), "Next");
    let shown = redacted(&mut events);
    assert!(!shown.contains("sk-ant"), "{shown}");
}

#[test]
fn injected_values_with_punctuation_or_spaces_stay_whole() {
    let value = "p@ss:w0rd! long-enough";
    let hold = Holdback::new(Arc::new(SecretScanner::new(vec![value.into()])));
    let mut events = hold.take(delta("m1", "the password is p@ss:"));
    events.extend(hold.take(delta("m1", "w0rd! long-enough, ok")));
    events.extend(hold.end_turn());
    assert_eq!(
        stream(&events, "m1"),
        format!("the password is {value}, ok")
    );
    let (redactor, session) = (SecretRedactor::new(), SessionId::new());
    redactor.set(&session, Arc::new(SecretScanner::new(vec![value.into()])));
    for e in &mut events {
        redactor.redact(&session, e);
    }
    let shown: String = events.iter().map(json).collect();
    assert!(!shown.contains("w0rd"), "{shown}");
}

#[test]
fn a_bounded_hold_never_cuts_inside_a_word() {
    let hold = builtin();
    let token = token();
    // One long line, the token straddling where a plain 1 KiB cut would fall.
    let line = format!("{} {token} {}", "x ".repeat(1000), "y".repeat(1000));
    let at = line.len() - 1024 - 10;
    let mut events = hold.take(out("tc_1", &line[..at]));
    events.extend(hold.take(out("tc_1", &line[at..])));
    events.extend(hold.take(result("tc_1")));
    assert_eq!(stream(&events, "tc_1"), format!("{line}|"));
    let shown = redacted(&mut events);
    assert!(!shown.contains("ghp_"), "{shown}");
    assert_eq!(bounded("aa bb cc", 0, 4), 6);
    assert_eq!(bounded("aaaaaaaa", 0, 4), 4);
    assert_eq!(bounded("aa bb", 3, 4), 3);
}

#[test]
fn held_streams_are_capped_and_nothing_is_lost() {
    let hold = builtin();
    let mut events = Vec::new();
    for i in 0..1000 {
        events.extend(hold.take(out(&format!("tc_{i}"), &format!("line\npartial {i}"))));
        assert!(lock(&hold.streams).held.len() <= MAX_HELD_STREAMS);
    }
    events.extend(hold.release_all());
    for i in [0, 500, 999] {
        assert_eq!(
            stream(&events, &format!("tc_{i}")),
            format!("line\npartial {i}")
        );
    }
}

fn piped(hold: &Holdback, pipe: Pipe, text: &str) -> Vec<EventPayload> {
    hold.take_output(
        pipe,
        TerminalOutputBody {
            terminal_id: "tc_1".into(),
            chunk: text.into(),
        },
    )
}

#[test]
fn stdout_and_stderr_are_separate_streams() {
    let hold = builtin();
    let token = token();
    let mut events = piped(&hold, Pipe::Stdout, &format!("token: {}", &token[..12]));
    events.extend(piped(&hold, Pipe::Stderr, "   Compiling foo\n"));
    events.extend(piped(&hold, Pipe::Stdout, &format!("{}\n", &token[12..])));
    events.extend(hold.take(result("tc_1")));
    let shown = redacted(&mut events);
    assert!(!shown.contains("ghp_"), "{shown}");
    assert_eq!(
        shown.matches("[redacted:github-token]").count(),
        1,
        "{shown}"
    );
    assert!(shown.contains("Compiling foo"));
}

#[test]
fn injected_values_spanning_lines_stay_whole() {
    let value = "line-one-of-it\nline-two-of-it";
    let scanner = Arc::new(SecretScanner::new(vec![value.into()]));
    let hold = Holdback::new(scanner.clone());
    let mut events = hold.take(out("tc_1", "value: line-one-of-it\n"));
    events.extend(hold.take(out("tc_1", "line-two-of-it\nafter\n")));
    events.extend(hold.take(result("tc_1")));
    let (redactor, session) = (SecretRedactor::new(), SessionId::new());
    redactor.set(&session, scanner);
    for e in &mut events {
        redactor.redact(&session, e);
    }
    let shown: String = events.iter().map(json).collect();
    assert!(!shown.contains("line-one"), "{shown}");
    assert!(shown.contains("[redacted:injected-secret]"), "{shown}");
}

#[test]
fn multibyte_output_with_a_spanning_value_cuts_on_char_boundaries() {
    let value = "line-one-of-it\nline-two-of-it";
    let hold = Holdback::new(Arc::new(SecretScanner::new(vec![value.into()])));
    // Every offset of a multibyte run, so one lands inside a character.
    for n in 0..8 {
        let text = format!("{}✓✓✓✓ done", "x".repeat(n));
        let mut events = hold.take(out(&format!("tc_{n}"), &text));
        events.extend(hold.take(result(&format!("tc_{n}"))));
        assert_eq!(stream(&events, &format!("tc_{n}")), format!("{text}|"));
    }
}
