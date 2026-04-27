//! Server-Sent Events (SSE) decoding adapter.
//!
//! Decodes a byte stream of SSE-framed messages into a stream of typed
//! `(event, data)` pairs per the WHATWG SSE spec. The adapter:
//!
//! - Splits frames on `\n` or `\r\n` (both are spec-legal).
//! - Captures the most-recent `event:` field as the dispatch name; OpenAI
//!   omits it (empty string), Anthropic uses named events like
//!   `content_block_delta`.
//! - Concatenates multiple `data:` lines within a single event with `\n`.
//! - Ignores lines starting with `:` (SSE comments).
//! - Dispatches the buffered event on a blank line.
//!
//! ## Error model
//!
//! The adapter yields `Result<SseEvent, SseError>` so each error variant
//! carries its own type — `LineTooLong`, `IdleTimeout`, `WallClockTimeout`,
//! `Transport`. The provider-layer caller maps these onto
//! [`crate::ChatChunk::Error`] with the matching [`crate::StreamErrorKind`].
//! Keeping the typed `SseError` here means the SSE adapter is reusable from
//! any caller (Anthropic, OpenAI, future providers) without coupling to
//! `ChatChunk`.
//!
//! Bound defaults mirror Ollama's NDJSON decoder so the two transports share
//! a single DoS-resistance posture (1 MiB per line, 30 s idle, 600 s wall
//! clock).

use bytes::{Bytes, BytesMut};
use futures::stream::{BoxStream, StreamExt};
use std::time::Duration;
use tokio_util::codec::FramedRead;
use tokio_util::io::StreamReader;

/// Per-line SSE byte cap (1 MiB). Matches Ollama's NDJSON cap so a hostile
/// peer cannot exhaust memory by streaming a single newline-less line.
pub const DEFAULT_MAX_LINE_BYTES: usize = 1 << 20;
/// Wall-clock gap between consecutive SSE lines. Matches Ollama's idle cap.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Wall-clock cap on the entire SSE stream. Matches Ollama's wall-clock cap.
pub const DEFAULT_WALL_CLOCK_TIMEOUT: Duration = Duration::from_secs(600);

/// Bounds that make the SSE decoder DoS-resistant against a hostile peer.
/// Mirrors `crate::ollama::StreamConfig` — the two configs are deliberately
/// separate types because their callers wire different defaults at different
/// layers, but the defaults themselves are identical.
#[derive(Debug, Clone, Copy)]
pub struct StreamConfig {
    pub max_line_bytes: usize,
    pub idle_timeout: Duration,
    pub wall_clock_timeout: Duration,
}

impl StreamConfig {
    pub const DEFAULT: StreamConfig = StreamConfig {
        max_line_bytes: DEFAULT_MAX_LINE_BYTES,
        idle_timeout: DEFAULT_IDLE_TIMEOUT,
        wall_clock_timeout: DEFAULT_WALL_CLOCK_TIMEOUT,
    };
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One dispatched SSE message.
///
/// `event` is empty when the upstream omitted an `event:` field (typical of
/// OpenAI's chat completions stream). `data` is the raw payload bytes; if
/// the upstream split it across multiple `data:` lines they are joined here
/// with `\n` per the SSE spec.
#[derive(Debug, Clone, PartialEq)]
pub struct SseEvent {
    pub event: String,
    pub data: Bytes,
}

/// Terminal SSE adapter failure. Mirrors `StreamErrorKind` one-for-one so
/// the provider-layer caller can map directly without losing information.
#[derive(Debug)]
pub enum SseError {
    /// One SSE line exceeded `StreamConfig::max_line_bytes`.
    LineTooLong,
    /// No bytes received within `StreamConfig::idle_timeout`.
    IdleTimeout,
    /// Stream exceeded `StreamConfig::wall_clock_timeout`.
    WallClockTimeout,
    /// Transport-level error from the underlying byte stream.
    Transport(String),
}

impl std::fmt::Display for SseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SseError::LineTooLong => write!(f, "sse line exceeded max bytes"),
            SseError::IdleTimeout => write!(f, "sse idle timeout"),
            SseError::WallClockTimeout => write!(f, "sse wall-clock timeout"),
            SseError::Transport(msg) => write!(f, "sse transport: {msg}"),
        }
    }
}

impl std::error::Error for SseError {}

/// Decode a byte stream of SSE-framed messages into typed events.
///
/// Terminal failures (line cap exceeded, idle window elapsed, wall-clock
/// budget elapsed, transport error) yield a single `Err(SseError::*)` and
/// close the stream. The caller is responsible for translating these onto
/// its own terminal error shape.
pub fn decode_sse_stream<S, E>(
    byte_stream: S,
    cfg: StreamConfig,
) -> BoxStream<'static, Result<SseEvent, SseError>>
where
    S: futures::Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let pinned = Box::pin(byte_stream.map(|r| r.map_err(std::io::Error::other)));
    let reader = StreamReader::new(pinned);
    let framed = FramedRead::new(reader, SseLineCodec::new(cfg.max_line_bytes));

    let deadline = tokio::time::Instant::now() + cfg.wall_clock_timeout;
    let state = DecoderState {
        framed,
        cfg,
        deadline,
        terminated: false,
        event: String::new(),
        data: BytesMut::new(),
        data_started: false,
        has_field: false,
    };

    let stream = futures::stream::unfold(state, |mut s| async move {
        if s.terminated {
            return None;
        }

        loop {
            let now = tokio::time::Instant::now();
            if now >= s.deadline {
                s.terminated = true;
                return Some((Err(SseError::WallClockTimeout), s));
            }

            let idle = s.cfg.idle_timeout.min(s.deadline - now);
            match tokio::time::timeout(idle, s.framed.next()).await {
                Err(_) => {
                    s.terminated = true;
                    let err = if tokio::time::Instant::now() >= s.deadline {
                        SseError::WallClockTimeout
                    } else {
                        SseError::IdleTimeout
                    };
                    return Some((Err(err), s));
                }
                Ok(None) => return None,
                Ok(Some(Err(e))) => {
                    s.terminated = true;
                    let err = match e {
                        SseLineError::MaxLineLengthExceeded => SseError::LineTooLong,
                        SseLineError::Io(io) => SseError::Transport(io.to_string()),
                    };
                    return Some((Err(err), s));
                }
                Ok(Some(Ok(line))) => {
                    if let Some(event) = handle_line(&mut s, &line) {
                        return Some((Ok(event), s));
                    }
                }
            }
        }
    });

    Box::pin(stream.fuse())
}

struct DecoderState<R> {
    framed: FramedRead<R, SseLineCodec>,
    cfg: StreamConfig,
    deadline: tokio::time::Instant,
    terminated: bool,
    event: String,
    data: BytesMut,
    data_started: bool,
    has_field: bool,
}

/// Process one decoded SSE line. Returns `Some(event)` when a blank line
/// dispatches an accumulated event; `None` when the line is buffered
/// (field, comment, or empty without a pending event).
fn handle_line<R>(state: &mut DecoderState<R>, line: &Bytes) -> Option<SseEvent> {
    let line = strip_cr(line);

    if line.is_empty() {
        // Blank line — dispatch only if we have at least one field.
        if !state.has_field {
            return None;
        }
        let event = SseEvent {
            event: std::mem::take(&mut state.event),
            data: state.data.split().freeze(),
        };
        state.data_started = false;
        state.has_field = false;
        return Some(event);
    }

    // Comment line.
    if line[0] == b':' {
        return None;
    }

    // Field parsing per SSE spec: split on the first `:`. The portion after
    // an optional single space is the value. A line with no `:` treats the
    // whole line as the field name with an empty value.
    let (field, value) = match line.iter().position(|b| *b == b':') {
        Some(idx) => {
            let f = &line[..idx];
            let mut v = &line[idx + 1..];
            if let Some((b' ', rest)) = v.split_first() {
                v = rest;
            }
            (f, v)
        }
        None => (line, &b""[..]),
    };

    state.has_field = true;

    match field {
        b"event" => {
            // Last `event:` wins. Non-UTF-8 collapses to empty (the spec
            // allows replacement, but every real provider sends ASCII names).
            state.event = std::str::from_utf8(value).unwrap_or("").to_string();
        }
        b"data" => {
            if state.data_started {
                state.data.extend_from_slice(b"\n");
            }
            state.data.extend_from_slice(value);
            state.data_started = true;
        }
        _ => {
            // `id:` / `retry:` / unknown fields — buffered (count as a
            // field for dispatch) but otherwise ignored. SSE consumers in
            // this codebase don't use them.
        }
    }

    None
}

fn strip_cr(line: &Bytes) -> &[u8] {
    let bytes = line.as_ref();
    match bytes.last() {
        Some(b'\r') => &bytes[..bytes.len() - 1],
        _ => bytes,
    }
}

// ── SseLineCodec ──────────────────────────────────────────────────────────────
//
// Lifted from `ollama::BytesLineCodec` because the line-framing requirements
// are identical (\n-delimited, byte-cap-bounded, yields `Bytes` slices over
// the codec's buffer). The two crates intentionally do NOT share a codec —
// pulling Ollama's into a shared module would be churn outside this task's
// DoD. Diverging behavior between the two would be caught by the unit
// tests in each module.

#[derive(Debug)]
struct SseLineCodec {
    max_line_bytes: usize,
    next_index: usize,
    discarding: bool,
}

#[derive(Debug)]
enum SseLineError {
    MaxLineLengthExceeded,
    Io(std::io::Error),
}

impl From<std::io::Error> for SseLineError {
    fn from(e: std::io::Error) -> Self {
        SseLineError::Io(e)
    }
}

impl std::fmt::Display for SseLineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SseLineError::MaxLineLengthExceeded => write!(f, "max line length exceeded"),
            SseLineError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SseLineError {}

impl SseLineCodec {
    fn new(max_line_bytes: usize) -> Self {
        Self {
            max_line_bytes,
            next_index: 0,
            discarding: false,
        }
    }
}

impl tokio_util::codec::Decoder for SseLineCodec {
    type Item = Bytes;
    type Error = SseLineError;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Bytes>, Self::Error> {
        use bytes::Buf;
        if self.discarding {
            if let Some(nl_offset) = buf.iter().position(|b| *b == b'\n') {
                buf.advance(nl_offset + 1);
                self.discarding = false;
                self.next_index = 0;
            } else {
                let len = buf.len();
                buf.advance(len);
                return Ok(None);
            }
        }

        let read_to = std::cmp::min(self.max_line_bytes.saturating_add(1), buf.len());
        let newline = buf[self.next_index..read_to]
            .iter()
            .position(|b| *b == b'\n');

        match newline {
            Some(offset) => {
                let nl_index = self.next_index + offset;
                let mut line = buf.split_to(nl_index + 1);
                line.truncate(line.len() - 1);
                self.next_index = 0;
                Ok(Some(line.freeze()))
            }
            None if buf.len() > self.max_line_bytes => {
                self.discarding = true;
                self.next_index = 0;
                Err(SseLineError::MaxLineLengthExceeded)
            }
            None => {
                self.next_index = buf.len();
                Ok(None)
            }
        }
    }

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Bytes>, Self::Error> {
        match self.decode(buf)? {
            Some(line) => Ok(Some(line)),
            None => {
                if buf.is_empty() || self.discarding {
                    self.discarding = false;
                    self.next_index = 0;
                    Ok(None)
                } else {
                    let line = buf.split_to(buf.len()).freeze();
                    self.next_index = 0;
                    Ok(Some(line))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::{self, StreamExt};
    use std::convert::Infallible;

    fn bytes_stream(
        chunks: Vec<&'static [u8]>,
    ) -> impl futures::Stream<Item = Result<Bytes, Infallible>> {
        stream::iter(chunks.into_iter().map(|c| Ok(Bytes::from_static(c))))
    }

    async fn collect_events(input: &'static [u8]) -> Vec<Result<SseEvent, SseError>> {
        let s = bytes_stream(vec![input]);
        decode_sse_stream(s, StreamConfig::DEFAULT).collect().await
    }

    #[tokio::test]
    async fn single_event_with_one_data_line() {
        let out = collect_events(b"event: foo\ndata: hello\n\n").await;
        assert_eq!(out.len(), 1, "expected exactly one event, got {out:?}");
        let ev = out[0].as_ref().expect("event ok");
        assert_eq!(ev.event, "foo");
        assert_eq!(ev.data.as_ref(), b"hello");
    }

    #[tokio::test]
    async fn multiple_data_lines_concatenate_with_newline() {
        let out = collect_events(b"event: chunk\ndata: line1\ndata: line2\ndata: line3\n\n").await;
        assert_eq!(out.len(), 1);
        let ev = out[0].as_ref().expect("event ok");
        assert_eq!(ev.event, "chunk");
        assert_eq!(ev.data.as_ref(), b"line1\nline2\nline3");
    }

    #[tokio::test]
    async fn omitted_event_field_yields_empty_string() {
        let out = collect_events(b"data: payload\n\n").await;
        assert_eq!(out.len(), 1);
        let ev = out[0].as_ref().expect("event ok");
        assert_eq!(ev.event, "");
        assert_eq!(ev.data.as_ref(), b"payload");
    }

    #[tokio::test]
    async fn comment_lines_are_ignored() {
        let out = collect_events(b": this is a comment\ndata: real\n: another comment\n\n").await;
        assert_eq!(out.len(), 1, "comments must not dispatch events");
        let ev = out[0].as_ref().expect("event ok");
        assert_eq!(ev.event, "");
        assert_eq!(ev.data.as_ref(), b"real");
    }

    #[tokio::test]
    async fn crlf_line_endings_work() {
        let out = collect_events(b"event: crlf\r\ndata: hello\r\n\r\n").await;
        assert_eq!(
            out.len(),
            1,
            "CRLF framing must produce one event, got {out:?}"
        );
        let ev = out[0].as_ref().expect("event ok");
        assert_eq!(ev.event, "crlf");
        assert_eq!(ev.data.as_ref(), b"hello");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn idle_timeout_yields_terminal_idle_error() {
        // Yield one valid event, then never produce more bytes.
        let (tx, rx) = futures::channel::mpsc::unbounded::<Result<Bytes, Infallible>>();
        tx.unbounded_send(Ok(Bytes::from_static(b"data: hi\n\n")))
            .unwrap();
        // Hold tx open forever — no further bytes arrive.
        let _hold_open = tx;

        let cfg = StreamConfig {
            max_line_bytes: 1024,
            idle_timeout: Duration::from_millis(80),
            wall_clock_timeout: Duration::from_secs(30),
        };
        let mut out = decode_sse_stream(rx, cfg);

        let first = tokio::time::timeout(Duration::from_secs(1), out.next())
            .await
            .expect("first event must arrive promptly")
            .expect("at least one event");
        let ev = first.expect("first item is the valid event");
        assert_eq!(ev.data.as_ref(), b"hi");

        let second = tokio::time::timeout(Duration::from_secs(2), out.next())
            .await
            .expect("must terminate via idle timeout, not hang")
            .expect("must yield terminal error");
        assert!(
            matches!(second, Err(SseError::IdleTimeout)),
            "expected IdleTimeout, got {second:?}"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(200), out.next())
                .await
                .expect("stream must close")
                .is_none(),
            "stream must close after terminal error"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn wall_clock_timeout_yields_terminal_error() {
        // A drip-feeder that keeps the idle timer happy but exceeds the
        // wall-clock budget. Send an empty heartbeat every 30 ms so the
        // 80 ms idle window never trips, but the 200 ms wall-clock will.
        let (tx, rx) = futures::channel::mpsc::unbounded::<Result<Bytes, Infallible>>();
        tokio::spawn(async move {
            for _ in 0..50 {
                if tx
                    .unbounded_send(Ok(Bytes::from_static(b": heartbeat\n")))
                    .is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(30)).await;
            }
        });

        let cfg = StreamConfig {
            max_line_bytes: 1024,
            idle_timeout: Duration::from_millis(80),
            wall_clock_timeout: Duration::from_millis(200),
        };
        let mut out = decode_sse_stream(rx, cfg);

        let result = tokio::time::timeout(Duration::from_secs(2), out.next())
            .await
            .expect("must terminate via wall-clock, not hang")
            .expect("must yield terminal error");
        assert!(
            matches!(result, Err(SseError::WallClockTimeout)),
            "expected WallClockTimeout, got {result:?}"
        );
        assert!(
            out.next().await.is_none(),
            "stream must close after terminal error"
        );
    }

    #[tokio::test]
    async fn transport_error_yields_terminal_transport_error() {
        #[derive(Debug)]
        struct BadIo;
        impl std::fmt::Display for BadIo {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "boom")
            }
        }
        impl std::error::Error for BadIo {}

        let stream = stream::iter(vec![
            Ok::<Bytes, BadIo>(Bytes::from_static(b"data: ok\n\n")),
            Err(BadIo),
        ]);

        let mut out = decode_sse_stream(stream, StreamConfig::DEFAULT);
        let first = out.next().await.expect("first event").expect("ok event");
        assert_eq!(first.data.as_ref(), b"ok");

        let second = out.next().await.expect("transport error");
        match second {
            Err(SseError::Transport(msg)) => {
                assert!(
                    msg.contains("boom"),
                    "transport error must surface source: {msg}"
                );
            }
            other => panic!("expected Transport error, got {other:?}"),
        }
        assert!(
            out.next().await.is_none(),
            "stream must close after terminal error"
        );
    }

    #[tokio::test]
    async fn line_exceeding_cap_yields_terminal_line_too_long() {
        // 200 bytes of `a` followed by `\n` — cap of 100 must terminate.
        let big: Vec<u8> = std::iter::repeat_n(b'a', 200)
            .chain(std::iter::once(b'\n'))
            .collect();
        let stream = stream::iter(vec![Ok::<_, Infallible>(Bytes::from(big))]);
        let cfg = StreamConfig {
            max_line_bytes: 100,
            ..StreamConfig::DEFAULT
        };
        let mut out = decode_sse_stream(stream, cfg);
        let first = out.next().await.expect("must yield terminal error");
        assert!(
            matches!(first, Err(SseError::LineTooLong)),
            "expected LineTooLong, got {first:?}"
        );
        // Stream must be closed after the terminal error.
        assert!(
            out.next().await.is_none(),
            "stream must close after terminal error"
        );
    }

    #[tokio::test]
    async fn multiple_events_yield_in_order() {
        let input = b"event: first\ndata: 1\n\nevent: second\ndata: 2\n\nevent: third\ndata: 3\n\n";
        let out = collect_events(input).await;
        assert_eq!(out.len(), 3, "expected three events, got {out:?}");
        let names: Vec<&str> = out
            .iter()
            .map(|r| r.as_ref().unwrap().event.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second", "third"]);
        let datas: Vec<&[u8]> = out
            .iter()
            .map(|r| r.as_ref().unwrap().data.as_ref())
            .collect();
        assert_eq!(datas, vec![&b"1"[..], &b"2"[..], &b"3"[..]]);
    }
}
