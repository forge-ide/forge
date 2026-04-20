//! Ollama provider — NDJSON streaming against a local Ollama daemon.
//!
//! No credentials; the daemon is expected at `http://127.0.0.1:11434` by default.
//!
//! # Streaming bounds
//!
//! A local squatter on port 11434 (race at startup, Ollama crash, malicious
//! binary) controls the byte stream we consume. The decoder is hardened with
//! three independent bounds — per-line byte cap, inter-chunk idle timeout, and
//! overall wall-clock timeout — any of which terminates the stream with a
//! typed [`ChatChunk::Error`] rather than panicking or growing unbounded.
//!
//! # Client bounds
//!
//! Below the decoder, the reqwest client itself is built with explicit
//! `connect_timeout`, `read_timeout`, and `tcp_keepalive` settings (see
//! [`ClientConfig`]). Two clients live on the provider: `stream_client` for
//! `/api/chat` omits a total `.timeout()` so long generations are not cut,
//! while `request_client` for short-lived `/api/tags` applies one. These
//! HTTP-layer bounds fire on half-open connects and stalled header reads —
//! conditions that occur *before* the NDJSON decoder starts and therefore
//! cannot be caught by its stream-level timers.

use std::borrow::Cow;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::{ChatChunk, ChatMessage, ChatRequest, Provider, StreamErrorKind};
use forge_core::Result;
use futures::stream::{BoxStream, StreamExt};
use reqwest::Url;
use serde::Deserialize;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

/// Opt-in env var that gates non-loopback `OLLAMA_BASE_URL` values. Strict
/// literal match against `"1"` — no other spelling is accepted so typos
/// don't accidentally enable remote dialing.
pub const ALLOW_REMOTE_ENV: &str = "FORGE_ALLOW_REMOTE_OLLAMA";

/// Validate a user-supplied Ollama base URL against the loopback policy.
///
/// F-058 / M5 (T7 — config injection / trust boundary). `OLLAMA_BASE_URL` is
/// read from the process environment and sets where LLM traffic and every
/// `ChatBlock::ToolResult` payload (including `fs.read` content) is sent.
/// Because `reqwest` is built with `rustls-tls`, an unvalidated URL can
/// TLS-dial any host — an attacker with env-var write access (shell-init,
/// `terminal.integrated.env`, `.envrc`, PATH-hijacked launcher) turns the
/// agent into a remote-controlled exfiltration channel.
///
/// Policy:
/// - `raw` is `None` or empty → fall back to [`DEFAULT_BASE_URL`].
/// - Scheme must be `http` (Ollama's loopback convention; `https` is rejected
///   even under opt-in — the trust model is loopback, not TLS-anywhere).
/// - Host must be exact-match `127.0.0.1`, `localhost`, or `::1` unless
///   `allow_remote` is true. Exact-match — not `starts_with` — so
///   `127.0.0.1.attacker.com` is rejected.
pub fn validate_base_url(raw: Option<&str>, allow_remote: bool) -> Result<Url> {
    let raw = raw.map(str::trim).filter(|s| !s.is_empty());
    let input = raw.unwrap_or(DEFAULT_BASE_URL);

    let url = Url::parse(input)
        .map_err(|e| anyhow::anyhow!("OLLAMA_BASE_URL parse failed for {input:?}: {e}"))?;

    if url.scheme() != "http" {
        return Err(anyhow::anyhow!(
            "OLLAMA_BASE_URL scheme {:?} is not allowed; only `http` is accepted \
             (Ollama loopback convention)",
            url.scheme()
        )
        .into());
    }

    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("OLLAMA_BASE_URL {input:?} has no host"))?;

    // `Url::host_str()` lowercases DNS names and, for IPv6 literals, returns
    // the bracketed form (`"[::1]"`). Match both spellings defensively; a
    // future `url` upgrade that switches to unbracketed will still pass.
    let is_loopback = matches!(host, "127.0.0.1" | "localhost" | "::1" | "[::1]");

    if !is_loopback && !allow_remote {
        return Err(anyhow::anyhow!(
            "OLLAMA_BASE_URL host {host:?} is not loopback; set \
             {ALLOW_REMOTE_ENV}=1 to explicitly opt in to a remote Ollama endpoint"
        )
        .into());
    }

    Ok(url)
}

/// Parse the `FORGE_ALLOW_REMOTE_OLLAMA` env-var value. Strict: only literal
/// `"1"` is accepted. Any other value (including `"true"`, `"yes"`, or empty)
/// is treated as not-set so a typo cannot silently unlock remote dialing.
pub fn parse_allow_remote(raw: Option<&str>) -> bool {
    raw == Some("1")
}

/// Per-line NDJSON byte cap (1 MiB). Real Ollama chunks are <100 KB.
pub const DEFAULT_MAX_LINE_BYTES: usize = 1 << 20;
/// Cap on the buffered response body for `list_models()` (1 MiB). Real
/// `/api/tags` responses are a few kilobytes; a multi-megabyte body is
/// pathological and is rejected before `serde_json::from_slice` to bound
/// peak allocation against a hostile peer on the loopback interface.
pub const DEFAULT_MAX_BODY_BYTES: usize = 1 << 20;
/// Wall-clock gap between consecutive chunks. 30 s is generous for local
/// models but still bounds half-open and slow-drip peers.
pub const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Wall-clock cap on the entire stream. 10 min accommodates slow local
/// inference; pathological runs can be re-attempted by the user.
pub const DEFAULT_WALL_CLOCK_TIMEOUT: Duration = Duration::from_secs(600);

/// Cap on the TCP-connect handshake.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Per-read idle cap at the HTTP layer (reqwest ≥ 0.12.5). Applies to header
/// reads and between-chunk gaps on streaming responses. Aligns numerically
/// with `DEFAULT_IDLE_TIMEOUT` but fires on a different condition (HTTP
/// transport vs NDJSON line gap); both are intentionally kept.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Total-request cap for buffered (non-streaming) endpoints only. Streaming
/// clients omit this so long generations are not cut mid-stream.
pub const DEFAULT_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
/// TCP keepalive probe interval; helps detect dead peers on long-lived
/// streaming connections that might otherwise linger at the OS layer.
pub const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(30);

/// Bounds that make the NDJSON decoder DoS-resistant against a hostile peer.
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

/// HTTP-layer timeouts applied at `reqwest::ClientBuilder` construction.
///
/// `total_timeout` is enforced on the buffered `list_models()` path only; the
/// streaming `chat()` path relies on `read_timeout` (per-read) so long
/// generations are not cut by a wall-clock cap.
#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub total_timeout: Duration,
    pub tcp_keepalive: Duration,
}

impl ClientConfig {
    pub const DEFAULT: ClientConfig = ClientConfig {
        connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        read_timeout: DEFAULT_READ_TIMEOUT,
        total_timeout: DEFAULT_TOTAL_TIMEOUT,
        tcp_keepalive: DEFAULT_TCP_KEEPALIVE,
    };
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Build the streaming client (no total `.timeout()`; long generations must
/// not be cut by a wall-clock cap — `read_timeout` is the per-read guard).
fn build_stream_client(cfg: &ClientConfig) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(cfg.connect_timeout)
        .read_timeout(cfg.read_timeout)
        .tcp_keepalive(Some(cfg.tcp_keepalive))
        .build()
        .expect("reqwest stream client builder — only fails if no TLS backend is available")
}

/// Build the buffered request client (short-lived endpoints; total timeout
/// bounds every call end-to-end in addition to the per-read guard).
fn build_request_client(cfg: &ClientConfig) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(cfg.connect_timeout)
        .read_timeout(cfg.read_timeout)
        .timeout(cfg.total_timeout)
        .tcp_keepalive(Some(cfg.tcp_keepalive))
        .build()
        .expect("reqwest request client builder — only fails if no TLS backend is available")
}

pub struct OllamaProvider {
    base_url: String,
    model: String,
    stream_client: reqwest::Client,
    request_client: reqwest::Client,
    stream_cfg: StreamConfig,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self::with_config_full(
            base_url,
            model,
            ClientConfig::DEFAULT,
            StreamConfig::DEFAULT,
        )
    }

    pub fn with_default_url(model: impl Into<String>) -> Self {
        Self::new(DEFAULT_BASE_URL, model)
    }

    /// Construct a provider with explicit streaming bounds. Primarily a test
    /// affordance; production code should use [`OllamaProvider::new`].
    #[doc(hidden)]
    pub fn with_config(
        base_url: impl Into<String>,
        model: impl Into<String>,
        stream_cfg: StreamConfig,
    ) -> Self {
        Self::with_config_full(base_url, model, ClientConfig::DEFAULT, stream_cfg)
    }

    /// Construct a provider with explicit HTTP-client and decoder bounds.
    /// Primarily a test affordance for regression tests that need fast
    /// `read_timeout` windows; production code should use
    /// [`OllamaProvider::new`].
    #[doc(hidden)]
    pub fn with_config_full(
        base_url: impl Into<String>,
        model: impl Into<String>,
        client_cfg: ClientConfig,
        stream_cfg: StreamConfig,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            stream_client: build_stream_client(&client_cfg),
            request_client: build_request_client(&client_cfg),
            stream_cfg,
        }
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url.trim_end_matches('/'));
        let resp = self
            .request_client
            .get(&url)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("ollama list_models request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "ollama list_models HTTP {status}: {}",
                truncate(&body, 500)
            )
            .into());
        }

        // Buffer the body and enforce a size cap before `serde_json::from_slice`.
        // A hostile local peer can advertise any `Content-Length` and stream
        // gigabytes into memory; the cap bounds peak allocation on the
        // dashboard-refresh path. `resp.bytes()` still buffers, so this guards
        // deserialization cost — transport-layer bounds (see `ClientConfig`)
        // handle the wall-clock side.
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| anyhow::anyhow!("ollama list_models read failed: {e}"))?;
        if bytes.len() > DEFAULT_MAX_BODY_BYTES {
            return Err(anyhow::anyhow!(
                "ollama list_models body too large: {} bytes exceeds cap of {} bytes",
                bytes.len(),
                DEFAULT_MAX_BODY_BYTES
            )
            .into());
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|e| anyhow::anyhow!("ollama list_models decode failed: {e}"))?;

        let models = value
            .get("models")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("name").and_then(|n| n.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        Ok(models)
    }
}

impl Provider for OllamaProvider {
    fn chat(
        &self,
        req: ChatRequest,
    ) -> impl std::future::Future<Output = Result<BoxStream<'static, ChatChunk>>> + Send {
        let url = format!("{}/api/chat", self.base_url.trim_end_matches('/'));
        let body = serde_json::json!({
            "model": self.model,
            "messages": to_ollama_messages(&req.system, &req.messages),
            "stream": true,
        });
        let client = self.stream_client.clone();
        let cfg = self.stream_cfg;

        async move {
            let resp = client
                .post(&url)
                .json(&body)
                .send()
                .await
                .map_err(|e| anyhow::anyhow!("ollama chat request failed: {e}"))?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(
                    anyhow::anyhow!("ollama chat HTTP {status}: {}", truncate(&body, 500)).into(),
                );
            }

            Ok(decode_ndjson_stream(resp.bytes_stream(), cfg))
        }
    }
}

/// Decode a byte stream of NDJSON into [`ChatChunk`]s under the configured
/// bounds. Terminal failures (cap exceeded, idle/wall-clock elapsed, transport
/// error) surface as a single [`ChatChunk::Error`] and close the stream.
/// Transport errors here include reqwest's client-level `read_timeout` firing
/// mid-stream (see [`ClientConfig`]), which surfaces as
/// [`StreamErrorKind::Transport`].
fn decode_ndjson_stream<S, E>(byte_stream: S, cfg: StreamConfig) -> BoxStream<'static, ChatChunk>
where
    S: futures::Stream<Item = std::result::Result<bytes::Bytes, E>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    use tokio_util::codec::{FramedRead, LinesCodec};
    use tokio_util::io::StreamReader;

    // Adapt reqwest's `bytes_stream()` to an `AsyncRead`, then decode bounded
    // NDJSON lines. `LinesCodec::new_with_max_length` returns
    // `MaxLineLengthExceeded` cleanly instead of growing the buffer past the cap.
    // `reqwest::Response::bytes_stream()` is `!Unpin`, so pin the adapter chain
    // once here; `FramedRead` requires `AsyncRead + Unpin`.
    let pinned = Box::pin(byte_stream.map(|r| r.map_err(std::io::Error::other)));
    let reader = StreamReader::new(pinned);
    let framed = FramedRead::new(reader, LinesCodec::new_with_max_length(cfg.max_line_bytes));

    let deadline = tokio::time::Instant::now() + cfg.wall_clock_timeout;
    let state = (framed, cfg, deadline, false);

    let chunks = futures::stream::unfold(
        state,
        |(mut framed, cfg, deadline, mut terminated)| async move {
            if terminated {
                return None;
            }

            loop {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    terminated = true;
                    return Some((
                        ChatChunk::Error {
                            kind: StreamErrorKind::WallClockTimeout,
                            message: format!(
                                "ollama stream exceeded wall-clock budget of {:?}",
                                cfg.wall_clock_timeout
                            ),
                        },
                        (framed, cfg, deadline, terminated),
                    ));
                }

                // Idle timeout between consecutive lines. Whichever expires
                // first — the idle window or the wall-clock deadline — wins.
                let idle = cfg.idle_timeout.min(deadline - now);
                match tokio::time::timeout(idle, framed.next()).await {
                    Err(_) => {
                        terminated = true;
                        let kind = if tokio::time::Instant::now() >= deadline {
                            StreamErrorKind::WallClockTimeout
                        } else {
                            StreamErrorKind::IdleTimeout
                        };
                        let message = match kind {
                            StreamErrorKind::WallClockTimeout => format!(
                                "ollama stream exceeded wall-clock budget of {:?}",
                                cfg.wall_clock_timeout
                            ),
                            _ => format!("ollama stream idle for more than {:?}", cfg.idle_timeout),
                        };
                        return Some((
                            ChatChunk::Error { kind, message },
                            (framed, cfg, deadline, terminated),
                        ));
                    }
                    Ok(None) => return None,
                    Ok(Some(Err(e))) => {
                        // Decoder errors are terminal — critically,
                        // `MaxLineLengthExceeded` must NOT be swallowed; that
                        // would let `FramedRead` keep reading-and-discarding
                        // bytes from a hostile peer until a newline finally
                        // arrives. Emit the typed error and stop.
                        use tokio_util::codec::LinesCodecError;
                        let (kind, message) = match e {
                            LinesCodecError::MaxLineLengthExceeded => (
                                StreamErrorKind::LineTooLong,
                                format!(
                                    "ollama NDJSON line exceeded cap of {} bytes",
                                    cfg.max_line_bytes
                                ),
                            ),
                            LinesCodecError::Io(err) => (
                                StreamErrorKind::Transport,
                                format!("ollama stream transport error: {err}"),
                            ),
                        };
                        terminated = true;
                        return Some((
                            ChatChunk::Error { kind, message },
                            (framed, cfg, deadline, terminated),
                        ));
                    }
                    Ok(Some(Ok(line))) => {
                        let trimmed = line.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Some(chunk) = parse_line(trimmed) {
                            return Some((chunk, (framed, cfg, deadline, terminated)));
                        }
                        // F-080: Unparseable line — surface the failure path
                        // (invalid JSON vs valid-JSON-unrecognized-shape) so a
                        // noisy or hostile peer is observable instead of
                        // silently burning CPU. Rate-limited to bound log cost
                        // on adversarial streams (see `log_unparseable_line`).
                        log_unparseable_line(trimmed);
                        continue;
                    }
                }
            }
        },
    );

    // `.fuse()` turns polls-after-`None` into more `None`s instead of the
    // `unfold` panic, which matters for callers that poll once more after
    // the terminal error chunk.
    Box::pin(chunks.fuse())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max])
    }
}

// ── NDJSON line decode ────────────────────────────────────────────────────────
//
// F-108: this was previously a two-step decode (`serde_json::from_str::<Value>`
// then `Value::get(..).as_str().to_string()`), which allocated three Strings
// per streamed text-delta token — the hottest per-token path in the app.
//
// The structures below deserialize a single line directly into stack storage.
// String fields carry `Cow<'a, str>`, so `#[serde(borrow)]` points them at the
// input slice for the common case (no JSON escapes) and only allocates when
// unescaping is necessary. `.into_owned()` runs exactly once at the
// `ChatChunk::*` emission boundary, matching the "0 or 1 allocation per
// text-delta token" budget in the F-108 DoD.
//
// `tool_calls` / `arguments` are held as owned `serde_json::Value` so the
// previous `.cloned()` on the tool-call path is now a move. That replaces one
// `Value`-tree clone per tool-call token (issue §Finding, line 498).
//
// `#[serde(default)]` on every optional field means the common text-delta
// shape (`{"message":{"content":".."},"done":false}`) deserializes without
// erroring on missing tool-call / done-reason fields.
#[derive(Deserialize)]
struct RawLine<'a> {
    #[serde(default)]
    done: bool,
    #[serde(default, borrow)]
    done_reason: Option<Cow<'a, str>>,
    #[serde(default, borrow)]
    message: Option<RawMessage<'a>>,
}

#[derive(Deserialize)]
struct RawMessage<'a> {
    #[serde(default, borrow)]
    content: Option<Cow<'a, str>>,
    #[serde(default)]
    tool_calls: Vec<RawToolCall<'a>>,
}

#[derive(Deserialize)]
struct RawToolCall<'a> {
    #[serde(borrow)]
    function: Option<RawFunction<'a>>,
}

#[derive(Deserialize)]
struct RawFunction<'a> {
    #[serde(borrow)]
    name: Cow<'a, str>,
    #[serde(default)]
    arguments: serde_json::Value,
}

/// Parse one Ollama NDJSON line into a [`ChatChunk`]. Public for the criterion
/// bench at `benches/ollama_stream.rs`; not part of the stable API surface.
#[doc(hidden)]
pub fn parse_line(line: &str) -> Option<ChatChunk> {
    let raw: RawLine<'_> = serde_json::from_str(line).ok()?;

    if raw.done {
        let reason = raw.done_reason.map(Cow::into_owned).unwrap_or_default();
        return Some(ChatChunk::Done(reason));
    }

    let message = raw.message?;

    // Tool-call chunks take priority over the text field — the shape is
    // `{"content":"","tool_calls":[..]}` and the empty content would otherwise
    // be discarded by the text-delta branch below.
    if let Some(first) = message.tool_calls.into_iter().next() {
        if let Some(func) = first.function {
            return Some(ChatChunk::ToolCall {
                name: func.name.into_owned(),
                args: func.arguments,
            });
        }
    }

    let content = message.content?;
    if content.is_empty() {
        return None;
    }
    Some(ChatChunk::TextDelta(content.into_owned()))
}

/// Cap on `log_unparseable_line` emissions per process. A hostile or buggy
/// peer that produces malformed lines on every chunk would otherwise turn the
/// log itself into an amplification surface — exactly the CPU-burn condition
/// the silent-drop finding (F-080 item 2) names. After the cap is hit the
/// counter still increments so a final summary message can quote the count.
const MAX_UNPARSEABLE_LOG_EMISSIONS: usize = 16;

/// Per-process count of `log_unparseable_line` calls (both emitted and
/// suppressed). The first `MAX_UNPARSEABLE_LOG_EMISSIONS` are written to
/// stderr; one summary line is emitted at the cap; further drops are silent.
static UNPARSEABLE_LINE_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Categorize an unparseable NDJSON line so the warning surfaces *why* the
/// decoder dropped it. `parse_line` returns `None` in two cases: the line is
/// not JSON at all, or it parses but does not match any recognized message
/// shape. Distinguishing the two helps an operator triage between provider
/// version skew and a hostile peer feeding garbage.
fn classify_unparseable(line: &str) -> &'static str {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(_) => "valid-json-unrecognized-shape",
        Err(_) => "invalid-json",
    }
}

fn log_unparseable_line(line: &str) {
    let prior = UNPARSEABLE_LINE_COUNT.fetch_add(1, Ordering::Relaxed);
    if prior < MAX_UNPARSEABLE_LOG_EMISSIONS {
        let kind = classify_unparseable(line);
        let preview = truncate(line, 120);
        eprintln!("ollama NDJSON dropped malformed line ({kind}): {preview}");
    } else if prior == MAX_UNPARSEABLE_LOG_EMISSIONS {
        eprintln!(
            "ollama NDJSON dropped malformed line cap reached ({MAX_UNPARSEABLE_LOG_EMISSIONS}); \
             further drops will be silent"
        );
    }
}

#[cfg(test)]
fn reset_unparseable_log_counter_for_test() {
    UNPARSEABLE_LINE_COUNT.store(0, Ordering::Relaxed);
}

#[cfg(test)]
fn unparseable_log_counter_for_test() -> usize {
    UNPARSEABLE_LINE_COUNT.load(Ordering::Relaxed)
}

fn to_ollama_messages(system: &Option<String>, messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    use crate::{ChatBlock, ChatRole};

    let mut out = Vec::with_capacity(messages.len() + 1);

    if let Some(sys) = system {
        out.push(serde_json::json!({"role": "system", "content": sys}));
    }

    for msg in messages {
        // `ChatBlock::ToolResult` can't coexist with text/tool-call blocks in Ollama's
        // flat message schema, so each block family emits its own message.
        let mut text_parts: Vec<&str> = Vec::new();
        let mut tool_calls: Vec<serde_json::Value> = Vec::new();

        for block in &msg.content {
            match block {
                ChatBlock::Text(t) => text_parts.push(t),
                ChatBlock::ToolCall { name, args, .. } => {
                    tool_calls.push(serde_json::json!({
                        "function": {
                            "name": name,
                            "arguments": args,
                        }
                    }));
                }
                ChatBlock::ToolResult { result, .. } => {
                    // Ollama tool responses are flat `role: "tool"` messages; serialize
                    // the structured result to a JSON string per Ollama's convention.
                    let content = serde_json::to_string(result).unwrap_or_else(|_| "null".into());
                    out.push(serde_json::json!({
                        "role": "tool",
                        "content": content,
                    }));
                }
            }
        }

        if text_parts.is_empty() && tool_calls.is_empty() {
            continue;
        }

        let role = match msg.role {
            ChatRole::User => "user",
            ChatRole::Assistant => "assistant",
        };
        let mut entry = serde_json::json!({
            "role": role,
            "content": text_parts.concat(),
        });
        if !tool_calls.is_empty() {
            entry["tool_calls"] = serde_json::Value::Array(tool_calls);
        }
        out.push(entry);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_text_delta_extracts_content() {
        let line = r#"{"message":{"role":"assistant","content":"hi"},"done":false}"#;
        assert_eq!(parse_line(line), Some(ChatChunk::TextDelta("hi".into())));
    }

    #[test]
    fn parse_line_done_with_reason() {
        let line = r#"{"model":"llama3","done":true,"done_reason":"stop"}"#;
        assert_eq!(parse_line(line), Some(ChatChunk::Done("stop".into())));
    }

    #[test]
    fn parse_line_done_missing_reason_yields_empty_string() {
        let line = r#"{"model":"llama3","done":true}"#;
        assert_eq!(parse_line(line), Some(ChatChunk::Done(String::new())));
    }

    #[test]
    fn parse_line_malformed_returns_none() {
        assert_eq!(parse_line("not-json"), None);
    }

    // F-080 item 2: a malformed NDJSON line must not be silently dropped —
    // the decoder classifies and rate-limit-logs it instead. These tests pin
    // the classifier behavior; the rate-limit counter is exercised separately
    // because its global state would couple unrelated tests.

    #[test]
    fn classify_unparseable_distinguishes_invalid_json_from_unknown_shape() {
        assert_eq!(classify_unparseable("not-json"), "invalid-json");
        // Valid JSON but does not match any of `parse_line`'s recognized
        // shapes (no `done`, no `message.tool_calls`, no `message.content`).
        assert_eq!(
            classify_unparseable(r#"{"unrelated":"shape"}"#),
            "valid-json-unrecognized-shape",
        );
    }

    #[test]
    fn log_unparseable_line_increments_counter_per_call() {
        // Run serially within this test since the counter is process-global.
        reset_unparseable_log_counter_for_test();
        log_unparseable_line("not-json");
        log_unparseable_line(r#"{"unrelated":"shape"}"#);
        assert_eq!(unparseable_log_counter_for_test(), 2);
        reset_unparseable_log_counter_for_test();
    }

    #[test]
    fn to_ollama_messages_prepends_system_and_flattens_text_blocks() {
        use crate::{ChatBlock, ChatMessage, ChatRole};

        let msgs = vec![ChatMessage {
            role: ChatRole::User,
            content: vec![
                ChatBlock::Text("hello ".into()),
                ChatBlock::Text("world".into()),
            ],
        }];

        let out = to_ollama_messages(&Some("sys-prompt".into()), &msgs);

        assert_eq!(
            out,
            vec![
                serde_json::json!({"role": "system", "content": "sys-prompt"}),
                serde_json::json!({"role": "user", "content": "hello world"}),
            ]
        );
    }

    #[test]
    fn to_ollama_messages_serializes_tool_call_and_result_blocks() {
        use crate::{ChatBlock, ChatMessage, ChatRole};

        let msgs = vec![
            ChatMessage {
                role: ChatRole::Assistant,
                content: vec![ChatBlock::ToolCall {
                    id: "c1".into(),
                    name: "fs.read".into(),
                    args: serde_json::json!({"path": "/a"}),
                }],
            },
            ChatMessage {
                role: ChatRole::User,
                content: vec![ChatBlock::ToolResult {
                    id: "c1".into(),
                    result: serde_json::json!({"content": "file data"}),
                }],
            },
        ];

        let out = to_ollama_messages(&None, &msgs);

        assert_eq!(
            out,
            vec![
                serde_json::json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "function": {
                            "name": "fs.read",
                            "arguments": {"path": "/a"}
                        }
                    }]
                }),
                serde_json::json!({
                    "role": "tool",
                    "content": "{\"content\":\"file data\"}"
                }),
            ]
        );
    }

    #[test]
    fn parse_line_tool_call_extracts_name_and_args() {
        let line = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"fs.read","arguments":{"path":"/tmp/x"}}}]},"done":false}"#;
        assert_eq!(
            parse_line(line),
            Some(ChatChunk::ToolCall {
                name: "fs.read".into(),
                args: serde_json::json!({"path": "/tmp/x"}),
            })
        );
    }

    // F-108: tool-call chunks used to carry an empty `content: ""` field
    // alongside `tool_calls`; the old `Value`-tree parser discarded that field
    // by testing `!content.is_empty()` after the tool-call branch. The typed
    // parser must preserve that ordering — tool_calls win over empty content.
    #[test]
    fn parse_line_tool_call_beats_empty_content() {
        let line = r#"{"message":{"content":"","tool_calls":[{"function":{"name":"n","arguments":{}}}]},"done":false}"#;
        assert!(matches!(parse_line(line), Some(ChatChunk::ToolCall { .. })));
    }

    // F-108: the common-case text delta has no `tool_calls`, no `done_reason`,
    // and no `done: true`. The typed decode must accept the lean shape without
    // requiring a `done: false` field.
    #[test]
    fn parse_line_text_delta_accepts_lean_shape() {
        let line = r#"{"message":{"content":"hello"}}"#;
        assert_eq!(parse_line(line), Some(ChatChunk::TextDelta("hello".into())));
    }

    // F-108: JSON escapes in the content field exercise the Cow::Owned path
    // (serde must allocate to unescape). The decoded payload must still be
    // the unescaped string, not the raw source bytes.
    #[test]
    fn parse_line_text_delta_with_escapes_is_unescaped() {
        let line = r#"{"message":{"content":"a\"b\nc"},"done":false}"#;
        assert_eq!(
            parse_line(line),
            Some(ChatChunk::TextDelta("a\"b\nc".into()))
        );
    }

    // F-108: empty content with no tool_calls emits nothing (the stream sends
    // these as keepalives on some models).
    #[test]
    fn parse_line_empty_content_without_tool_calls_returns_none() {
        let line = r#"{"message":{"content":""},"done":false}"#;
        assert_eq!(parse_line(line), None);
    }

    // ── validate_base_url: scheme/host policy ─────────────────────────────────
    //
    // F-058 / M5 (T7): `OLLAMA_BASE_URL` is a single-user trust boundary. An
    // attacker who can plant shell-init env vars otherwise redirects LLM and
    // tool-result traffic off-box. Policy: `http` only, loopback host only,
    // remote hosts gated by an explicit opt-in (`FORGE_ALLOW_REMOTE_OLLAMA=1`).

    #[test]
    fn validate_base_url_accepts_default_when_raw_is_none() {
        let url = validate_base_url(None, false).expect("default must pass");
        assert_eq!(url.scheme(), "http");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
    }

    #[test]
    fn validate_base_url_accepts_default_when_raw_is_empty() {
        // Empty string (unset-but-present env var shape) is treated as unset
        // so operators can't accidentally zero the URL to ""
        let url = validate_base_url(Some(""), false).expect("empty must fall back");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
    }

    #[test]
    fn validate_base_url_accepts_loopback_ipv4() {
        let url = validate_base_url(Some("http://127.0.0.1:11434"), false).expect("loopback v4");
        assert_eq!(url.host_str(), Some("127.0.0.1"));
    }

    #[test]
    fn validate_base_url_accepts_localhost() {
        validate_base_url(Some("http://localhost:11434"), false).expect("localhost");
    }

    #[test]
    fn validate_base_url_accepts_localhost_uppercase() {
        // `Url::parse` lowercases host; lock that assumption so a future
        // upgrade that changes it doesn't silently reject valid input.
        validate_base_url(Some("http://LOCALHOST:11434"), false).expect("LOCALHOST");
    }

    #[test]
    fn validate_base_url_accepts_loopback_ipv6() {
        // `Url::host_str()` strips the brackets; assert the normalized form.
        let url = validate_base_url(Some("http://[::1]:11434"), false).expect("loopback v6");
        assert_eq!(url.host_str(), Some("[::1]"));
    }

    #[test]
    fn validate_base_url_rejects_https_even_loopback() {
        let err = validate_base_url(Some("https://127.0.0.1:11434"), false)
            .expect_err("https must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("scheme"),
            "error must name the scheme policy, got: {msg}"
        );
    }

    #[test]
    fn validate_base_url_rejects_remote_host_without_opt_in() {
        let err = validate_base_url(Some("http://example.com"), false)
            .expect_err("non-loopback must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.contains("FORGE_ALLOW_REMOTE_OLLAMA"),
            "error must name the opt-in env var, got: {msg}"
        );
    }

    #[test]
    fn validate_base_url_rejects_https_remote_even_with_opt_in() {
        // Scheme policy is independent of the host opt-in — `https` is always
        // rejected because our trust model is for the Ollama loopback
        // convention, not for TLS-dialing arbitrary servers.
        let err = validate_base_url(Some("https://example.com"), true)
            .expect_err("https must still be rejected under opt-in");
        let msg = format!("{err}");
        assert!(msg.contains("scheme"), "got: {msg}");
    }

    #[test]
    fn validate_base_url_rejects_prefix_spoof_of_loopback() {
        // A host like `127.0.0.1.attacker.com` must be rejected — the policy
        // is exact host match, not a `starts_with` check.
        let err = validate_base_url(Some("http://127.0.0.1.attacker.com"), false)
            .expect_err("prefix-spoof must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("FORGE_ALLOW_REMOTE_OLLAMA"), "got: {msg}");
    }

    #[test]
    fn validate_base_url_accepts_remote_with_opt_in() {
        let url = validate_base_url(Some("http://example.com"), true).expect("opt-in remote");
        assert_eq!(url.host_str(), Some("example.com"));
    }

    #[test]
    fn validate_base_url_rejects_non_http_scheme() {
        let err =
            validate_base_url(Some("ftp://127.0.0.1"), false).expect_err("ftp must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("scheme"), "got: {msg}");
    }

    #[test]
    fn validate_base_url_rejects_file_scheme() {
        let err = validate_base_url(Some("file:///etc/passwd"), false)
            .expect_err("file must be rejected");
        let msg = format!("{err}");
        assert!(msg.contains("scheme"), "got: {msg}");
    }

    #[test]
    fn validate_base_url_rejects_malformed_url() {
        let err =
            validate_base_url(Some("not a url"), false).expect_err("malformed must be rejected");
        let msg = format!("{err}");
        assert!(
            msg.to_lowercase().contains("parse") || msg.contains("OLLAMA_BASE_URL"),
            "error should name the input or parse failure, got: {msg}"
        );
    }

    #[test]
    fn validate_base_url_rejects_url_without_host() {
        // `http:///foo` parses but has no host — treat as invalid.
        let err =
            validate_base_url(Some("http:///foo"), false).expect_err("no-host must be rejected");
        let msg = format!("{err}");
        // Message should reach the user; exact wording is not asserted, just
        // that we fail rather than dial an empty host.
        assert!(!msg.is_empty());
    }

    // ── opt-in env-var parsing ────────────────────────────────────────────────
    //
    // `FORGE_ALLOW_REMOTE_OLLAMA` is strict: only literal `"1"` counts.
    // Anything else (including common truthy spellings) is not opt-in so
    // typos don't accidentally unlock remote dialing.

    #[test]
    fn allow_remote_parses_one_as_true() {
        assert!(parse_allow_remote(Some("1")));
    }

    #[test]
    fn allow_remote_parses_unset_as_false() {
        assert!(!parse_allow_remote(None));
    }

    #[test]
    fn allow_remote_rejects_true_spelling() {
        assert!(!parse_allow_remote(Some("true")));
        assert!(!parse_allow_remote(Some("yes")));
        assert!(!parse_allow_remote(Some("0")));
        assert!(!parse_allow_remote(Some("")));
    }
}
