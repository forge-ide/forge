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

use std::time::Duration;

use crate::{ChatChunk, ChatMessage, ChatRequest, Provider, StreamErrorKind};
use forge_core::Result;
use futures::stream::{BoxStream, StreamExt};

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

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
                        // Unparseable but valid-shaped line — keep reading.
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

fn parse_line(line: &str) -> Option<ChatChunk> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;

    if value.get("done").and_then(|d| d.as_bool()) == Some(true) {
        let reason = value
            .get("done_reason")
            .and_then(|r| r.as_str())
            .unwrap_or("")
            .to_string();
        return Some(ChatChunk::Done(reason));
    }

    if let Some(first_call) = value
        .get("message")
        .and_then(|m| m.get("tool_calls"))
        .and_then(|tc| tc.as_array())
        .and_then(|arr| arr.first())
    {
        if let Some(func) = first_call.get("function") {
            let name = func.get("name").and_then(|n| n.as_str())?.to_string();
            let args = func
                .get("arguments")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            return Some(ChatChunk::ToolCall { name, args });
        }
    }

    if let Some(content) = value
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
    {
        if !content.is_empty() {
            return Some(ChatChunk::TextDelta(content.to_string()));
        }
    }

    None
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
}
