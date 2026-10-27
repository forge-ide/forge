//! Keeping secrets out of what Forge records: the event log's redactor, which replaces secrets
//! in every string of every event, and the holdback that keeps a secret split across streamed
//! chunks from slipping past it.
//!
//! This covers what Forge records, not what the agent sees: a terminal's output reaches the
//! agent as it is. Secrets stay out of the agent's reach by being injected by the egress proxy.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use forge_acp::Pipe;
use forge_api::events::{EventPayload, TerminalOutputBody};
use forge_api::model::Action;
use forge_api::{ErrorData, ErrorKind, SessionId};
use forge_events::Redactor;
use forge_policy::SecretScanner;

/// Least text a stream may hold back while waiting to see whether it continues a secret; more
/// when the session scans for a longer injected value.
const HOLD_LIMIT: usize = 1024;
/// Most streams one session holds text for at once; past it, the oldest is released.
const MAX_HELD_STREAMS: usize = 256;
/// What a private key block a stream starts without finishing becomes.
const KEY_PLACEHOLDER: &str = "[redacted:private-key; output withheld until END]";

/// Redacts every event with its session's scanner: the built-in patterns plus the values of
/// the secrets that session's proxy injects.
pub struct SecretRedactor {
    builtin: Arc<SecretScanner>,
    sessions: Mutex<HashMap<SessionId, Arc<SecretScanner>>>,
}

impl Default for SecretRedactor {
    fn default() -> Self {
        Self {
            builtin: Arc::new(SecretScanner::default()),
            sessions: Mutex::default(),
        }
    }
}

impl SecretRedactor {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Use `scanner` for `session`'s events from now on.
    pub(crate) fn set(&self, session: &SessionId, scanner: Arc<SecretScanner>) {
        lock(&self.sessions).insert(session.clone(), scanner);
    }

    /// Whether `session` has a scanner of its own.
    pub(crate) fn has(&self, session: &SessionId) -> bool {
        lock(&self.sessions).contains_key(session)
    }

    /// Forget `session`'s scanner and the secret values in it.
    pub(crate) fn remove(&self, session: &SessionId) {
        lock(&self.sessions).remove(session);
    }

    /// The session's scanner, or the built-in patterns' when it has none yet.
    pub(crate) fn scanner(&self, session: &SessionId) -> Arc<SecretScanner> {
        lock(&self.sessions)
            .get(session)
            .cloned()
            .unwrap_or_else(|| self.builtin.clone())
    }
}

impl Redactor for SecretRedactor {
    fn redact(&self, session: &SessionId, payload: &mut EventPayload) {
        let scanner = self.scanner(session);
        let Ok(mut value) = serde_json::to_value(&*payload) else {
            *payload = unrecordable(session);
            return;
        };
        if !redact_strings(&scanner, &mut value) {
            return;
        }
        // Every string in the event, whatever its kind and fields, so a new field can't
        // carry a secret past the log. An event that no longer fits its type isn't recorded.
        *payload = serde_json::from_value(value).unwrap_or_else(|_| unrecordable(session));
    }
}

/// `action` with secrets replaced in every string, for an approval's stored copy.
pub(crate) fn redact_action(scanner: &SecretScanner, action: &Action) -> Action {
    let Ok(mut value) = serde_json::to_value(action) else {
        return unrecordable_action();
    };
    if !redact_strings(scanner, &mut value) {
        return action.clone();
    }
    serde_json::from_value(value).unwrap_or_else(|_| unrecordable_action())
}

fn unrecordable_action() -> Action {
    Action::Exec {
        argv: vec!["[redacted]".into()],
        cwd: ".".into(),
    }
}

/// Replace secrets in every string within `value`, object keys included; whether anything
/// changed.
fn redact_strings(scanner: &SecretScanner, value: &mut serde_json::Value) -> bool {
    use serde_json::Value;
    match value {
        Value::String(s) => match scanner.redact(s) {
            std::borrow::Cow::Owned(redacted) => {
                *s = redacted;
                true
            }
            std::borrow::Cow::Borrowed(_) => false,
        },
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |changed, v| redact_strings(scanner, v) | changed),
        Value::Object(map) => {
            let mut changed = false;
            let entries = std::mem::take(map);
            for (key, mut v) in entries {
                changed |= redact_strings(scanner, &mut v);
                let key = match scanner.redact(&key) {
                    std::borrow::Cow::Owned(redacted) => {
                        changed = true;
                        redacted
                    }
                    std::borrow::Cow::Borrowed(_) => key,
                };
                map.insert(key, v);
            }
            changed
        }
        _ => false,
    }
}

fn unrecordable(session: &SessionId) -> EventPayload {
    tracing::error!(session_id = %session, "an event could not be redacted and was not recorded");
    EventPayload::Error(ErrorData {
        kind: ErrorKind::ValidationFailed,
        detail: "an event could not be redacted, so it was not recorded".into(),
        retryable: false,
    })
}

/// Holds back the end of streamed chunks (terminal output, agent message deltas) that might
/// be the start of a secret, until the stream shows how it ends. The redactor sees one event
/// at a time; this makes sure that whatever a stream's chunks are joined into, each secret
/// lies whole inside one event.
///
/// A terminal's stdout and stderr are separate streams. Held text is released only on
/// evidence the stream can't continue it: its own next chunk,
/// the end of its terminal (the terminal's `tool.result`), another message starting, the end
/// of the turn (messages only), or the agent stopping. An unrelated event in between never
/// releases it, so a released tail can come after other streams' events: order holds within a
/// stream, not across streams. A private key block a stream starts without finishing is
/// recorded as one placeholder, and everything after it is dropped until the block's END line
/// or the end of the stream.
pub(crate) struct Holdback {
    scanner: Arc<SecretScanner>,
    limit: usize,
    streams: Mutex<Streams>,
}

#[derive(Default)]
struct Streams {
    held: HashMap<String, Held>,
    /// Counts holds, to find the oldest.
    next: u64,
}

struct Held {
    order: u64,
    /// The stream's latest event, to send the held text in.
    template: EventPayload,
    text: String,
    /// Inside an unfinished private key block: `text` is dropped, not released.
    in_key: bool,
}

impl Holdback {
    pub(crate) fn new(scanner: Arc<SecretScanner>) -> Self {
        let limit = HOLD_LIMIT.max(scanner.longest_value());
        Self {
            scanner,
            limit,
            streams: Mutex::default(),
        }
    }

    /// What to record now for `payload`.
    pub(crate) fn take(&self, payload: EventPayload) -> Vec<EventPayload> {
        let mut streams = lock(&self.streams);
        match &payload {
            EventPayload::TerminalOutput(b) => {
                // Without its pipe, as if from stdout.
                let (key, text) = (format!("t:{}:out", b.terminal_id), b.chunk.clone());
                self.chunk(&mut streams, key, payload, &text, true)
            }
            EventPayload::AgentMessage(b) => {
                // Another message starting means the earlier ones are done.
                let key = format!("m:{}", b.message_id);
                let done: Vec<String> = streams
                    .held
                    .keys()
                    .filter(|k| k.starts_with("m:") && **k != key)
                    .cloned()
                    .collect();
                let mut out: Vec<EventPayload> = done
                    .iter()
                    .filter_map(|k| release(&mut streams, k))
                    .collect();
                let text = b.delta.clone();
                out.extend(self.chunk(&mut streams, key, payload, &text, false));
                out
            }
            EventPayload::ToolResult(b) => {
                // A terminal's result ends its output; terminal IDs are tool call IDs.
                let mut out: Vec<EventPayload> = ["out", "err"]
                    .iter()
                    .filter_map(|pipe| {
                        release(&mut streams, &format!("t:{}:{pipe}", b.tool_call_id))
                    })
                    .collect();
                out.push(payload);
                out
            }
            _ => vec![payload],
        }
    }

    /// What to record now for a chunk of a terminal's output. Each pipe is its own stream, so
    /// a line on one never splits a secret on the other.
    pub(crate) fn take_output(&self, pipe: Pipe, body: TerminalOutputBody) -> Vec<EventPayload> {
        let mut streams = lock(&self.streams);
        let pipe = match pipe {
            Pipe::Stdout => "out",
            Pipe::Stderr => "err",
        };
        let key = format!("t:{}:{pipe}", body.terminal_id);
        let text = body.chunk.clone();
        self.chunk(
            &mut streams,
            key,
            EventPayload::TerminalOutput(body),
            &text,
            true,
        )
    }

    /// Release the text of messages; the turn is over. Terminals keep theirs until they end.
    pub(crate) fn end_turn(&self) -> Vec<EventPayload> {
        let mut streams = lock(&self.streams);
        let messages: Vec<String> = streams
            .held
            .keys()
            .filter(|k| k.starts_with("m:"))
            .cloned()
            .collect();
        messages
            .iter()
            .filter_map(|k| release(&mut streams, k))
            .collect()
    }

    /// Release everything; the agent stopped.
    pub(crate) fn release_all(&self) -> Vec<EventPayload> {
        let mut streams = lock(&self.streams);
        let mut keys: Vec<(u64, String)> = streams
            .held
            .iter()
            .map(|(k, h)| (h.order, k.clone()))
            .collect();
        keys.sort();
        keys.iter()
            .filter_map(|(_, k)| release(&mut streams, k))
            .collect()
    }

    fn chunk(
        &self,
        streams: &mut Streams,
        key: String,
        payload: EventPayload,
        text: &str,
        line: bool,
    ) -> Vec<EventPayload> {
        let (mut joined, mut in_key) = match streams.held.remove(&key) {
            Some(held) => (held.text, held.in_key),
            None => (String::new(), false),
        };
        joined.push_str(text);
        let mut send = String::new();
        loop {
            if in_key {
                match self.scanner.key_block_end(&joined) {
                    Some(end) => {
                        joined.drain(..end);
                        in_key = false;
                    }
                    None => {
                        // Keep the unfinished last line, which may be the start of END.
                        let keep = joined.rfind('\n').map_or(0, |i| i + 1);
                        let keep = bounded(&joined, keep, self.limit);
                        joined.drain(..keep);
                        break;
                    }
                }
            } else if let Some(begin) = self.scanner.open_key_block(&joined) {
                send.push_str(&joined[..begin]);
                send.push_str(KEY_PLACEHOLDER);
                joined.drain(..begin);
                in_key = true;
            } else {
                let cut = self.hold_from(&joined, line);
                send.push_str(&joined[..cut]);
                joined.drain(..cut);
                break;
            }
        }
        let mut out = Vec::new();
        if !joined.is_empty() || in_key {
            let order = streams.next;
            streams.next += 1;
            streams.held.insert(
                key,
                Held {
                    order,
                    template: payload.clone(),
                    text: joined,
                    in_key,
                },
            );
            if streams.held.len() > MAX_HELD_STREAMS {
                let oldest = streams
                    .held
                    .iter()
                    .min_by_key(|(_, h)| h.order)
                    .map(|(k, _)| k.clone());
                if let Some(oldest) = oldest {
                    tracing::warn!("too many streams held back; releasing the oldest");
                    out.extend(release(streams, &oldest));
                }
            }
        }
        if !send.is_empty() {
            out.push(with_text(&payload, &send));
        }
        out
    }

    /// Where the held-back end of `text` starts. Terminal output holds its unfinished last
    /// line, since a secret's pattern may include spaces; a message holds its trailing
    /// non-whitespace, an unfinished private key header, and enough bytes for the start of the
    /// longest injected value (which may contain spaces). At most the hold limit, cut where a
    /// word starts, and never inside a secret already whole in the text.
    fn hold_from(&self, text: &str, line: bool) -> usize {
        let last_line = text.rfind('\n').map_or(0, |i| i + 1);
        let start = if line {
            // An injected value spanning lines may have started on an earlier line.
            let spanning = self.scanner.longest_multiline_value();
            if spanning > 0 {
                let mut floor = text.len().saturating_sub(spanning);
                while !text.is_char_boundary(floor) {
                    floor -= 1;
                }
                let line_start = text[..floor].rfind('\n').map_or(0, |i| i + 1);
                last_line.min(line_start)
            } else {
                last_line
            }
        } else {
            let word = text
                .char_indices()
                .rev()
                .take_while(|(_, c)| !c.is_whitespace())
                .last()
                .map_or(text.len(), |(i, _)| i);
            let header = text[last_line..]
                .rfind("-----BEGIN")
                .map_or(text.len(), |i| last_line + i);
            // A value still arriving starts within its length, less one, of the end.
            let mut value = text
                .len()
                .saturating_sub(self.scanner.longest_value().saturating_sub(1));
            while !text.is_char_boundary(value) {
                value -= 1;
            }
            word.min(header).min(value)
        };
        let mut cut = bounded(text, start, self.limit);
        // Never inside a secret already whole in the text: send it whole instead.
        for found in self.scanner.scan_cut(text) {
            if found.range.start < cut && cut < found.range.end {
                cut = found.range.end;
            }
        }
        cut
    }
}

/// `start`, or, if that holds more than `limit` bytes of `text`, the first word start after
/// the limit, so the cut never splits a word (one word longer than the limit is cut anyway).
fn bounded(text: &str, start: usize, limit: usize) -> usize {
    let floor = text.len().saturating_sub(limit);
    if start >= floor {
        return start;
    }
    let mut floor = floor;
    while !text.is_char_boundary(floor) {
        floor += 1;
    }
    if text[..floor].ends_with(char::is_whitespace) {
        return floor;
    }
    match text[floor..].find(char::is_whitespace) {
        Some(i) => {
            let space = floor + i;
            space + text[space..].chars().next().map_or(0, char::len_utf8)
        }
        None => floor,
    }
}

/// The held text of stream `key` as an event, unless it is key material.
fn release(streams: &mut Streams, key: &str) -> Option<EventPayload> {
    let held = streams.held.remove(key)?;
    (!held.in_key && !held.text.is_empty()).then(|| with_text(&held.template, &held.text))
}

fn with_text(payload: &EventPayload, text: &str) -> EventPayload {
    let mut payload = payload.clone();
    match &mut payload {
        EventPayload::TerminalOutput(b) => b.chunk = text.to_owned(),
        EventPayload::AgentMessage(b) => b.delta = text.to_owned(),
        _ => {}
    }
    payload
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

#[cfg(test)]
mod tests;
