//! OpenAI Chat Completions API provider — SSE-streamed responses against
//! `https://api.openai.com/v1/chat/completions` (or any compatible base URL).
//!
//! Authentication uses the `Authorization: Bearer <api-key>` header. Unlike
//! Anthropic, OpenAI does not pin a wire-version header — the `/v1/` path
//! prefix is the only versioning surface.
//!
//! # Streaming bounds
//!
//! The HTTP-layer client and the SSE decoder share Ollama's hardening posture:
//! per-line byte cap, inter-event idle timeout, and overall wall-clock budget.
//! Any of these terminates the stream with a typed [`ChatChunk::Error`] —
//! the SSE adapter ([`crate::sse`]) yields a typed [`crate::sse::SseError`]
//! that this module maps onto [`StreamErrorKind`] one-for-one.

use std::time::Duration;

use crate::sse::{self, SseError, SseEvent};
use crate::{ChatChunk, ChatRequest, Provider, StreamErrorKind};
use bytes::Bytes;
use forge_core::Result;
use futures::stream::{BoxStream, StreamExt};

pub mod custom;
pub mod translate;

pub use custom::{AuthShape, CustomOpenAiProvider};

pub const DEFAULT_BASE_URL: &str = "https://api.openai.com";

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);
pub const DEFAULT_TCP_KEEPALIVE: Duration = Duration::from_secs(30);

/// HTTP-layer timeouts applied at `reqwest::ClientBuilder` construction.
#[derive(Debug, Clone, Copy)]
pub struct ClientConfig {
    pub connect_timeout: Duration,
    pub read_timeout: Duration,
    pub tcp_keepalive: Duration,
}

impl ClientConfig {
    pub const DEFAULT: ClientConfig = ClientConfig {
        connect_timeout: DEFAULT_CONNECT_TIMEOUT,
        read_timeout: DEFAULT_READ_TIMEOUT,
        tcp_keepalive: DEFAULT_TCP_KEEPALIVE,
    };
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

fn build_stream_client(cfg: &ClientConfig) -> reqwest::Client {
    reqwest::Client::builder()
        .connect_timeout(cfg.connect_timeout)
        .read_timeout(cfg.read_timeout)
        .tcp_keepalive(Some(cfg.tcp_keepalive))
        .build()
        .expect("reqwest stream client builder — only fails if no TLS backend is available")
}

pub struct OpenAiProvider {
    base_url: String,
    api_key: String,
    model: String,
    /// Optional `max_tokens` cap — OpenAI omits the field when `None`,
    /// letting the server default apply.
    max_tokens: Option<u32>,
    stream_client: reqwest::Client,
    stream_cfg: sse::StreamConfig,
}

impl OpenAiProvider {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            max_tokens: None,
            stream_client: build_stream_client(&ClientConfig::DEFAULT),
            stream_cfg: sse::StreamConfig::DEFAULT,
        }
    }

    /// Set an explicit `max_tokens` cap. Builder-style.
    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Override the SSE decoder bounds. Builder-style; primarily a test
    /// affordance for fast idle-timeout / line-cap regression tests.
    #[doc(hidden)]
    pub fn with_config(mut self, stream_cfg: sse::StreamConfig) -> Self {
        self.stream_cfg = stream_cfg;
        self
    }
}

impl Provider for OpenAiProvider {
    fn chat(
        &self,
        req: ChatRequest,
    ) -> impl std::future::Future<Output = Result<BoxStream<'static, ChatChunk>>> + Send {
        let body_result = translate::serialize_request(&req, &self.model, self.max_tokens);
        let auth = vec![(
            reqwest::header::AUTHORIZATION,
            format!("Bearer {}", self.api_key),
        )];
        chat_request(
            self.stream_client.clone(),
            self.base_url.clone(),
            auth,
            body_result,
            self.stream_cfg,
        )
    }
}

/// Shared chat-request pipeline used by both [`OpenAiProvider`] and
/// [`CustomOpenAiProvider`]. Lifted here so the two providers share a single
/// implementation of the OpenAI Chat Completions wire protocol — only the
/// auth-header construction differs between them.
///
/// `auth_headers` is an `(HeaderName, header-value-string)` list rather than
/// a fully-typed `HeaderMap` so call sites stay terse and the
/// custom-provider's `AuthShape::None` variant maps to an empty `vec![]`
/// without wrestling with `HeaderMap::new()`.
pub(crate) fn chat_request(
    client: reqwest::Client,
    base_url: String,
    auth_headers: Vec<(reqwest::header::HeaderName, String)>,
    body_result: std::result::Result<Vec<u8>, serde_json::Error>,
    cfg: sse::StreamConfig,
) -> impl std::future::Future<Output = Result<BoxStream<'static, ChatChunk>>> + Send {
    let url = format!("{}/v1/chat/completions", base_url.trim_end_matches('/'));

    async move {
        let body = body_result
            .map_err(|e| anyhow::anyhow!("openai chat body serialization failed: {e}"))?;
        let mut builder = client
            .post(&url)
            .header(reqwest::header::CONTENT_TYPE, "application/json");
        for (name, value) in auth_headers {
            builder = builder.header(name, value);
        }
        let resp = builder
            .body(body)
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("openai chat request failed: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(
                anyhow::anyhow!("openai chat HTTP {status}: {}", truncate(&body, 500)).into(),
            );
        }

        Ok(decode_openai_stream(resp.bytes_stream(), cfg))
    }
}

/// Construct a stream HTTP client with the same hardening posture as
/// [`OpenAiProvider`]. Re-exported so [`CustomOpenAiProvider`] can build its
/// own client with identical timeouts.
pub(crate) fn build_stream_client_default() -> reqwest::Client {
    build_stream_client(&ClientConfig::DEFAULT)
}

/// Decode the raw `bytes` stream into a `ChatChunk` stream by piping through
/// the shared SSE adapter and translating each event payload via
/// [`translate::OpenAiEventAccumulator`].
fn decode_openai_stream<S, E>(
    byte_stream: S,
    cfg: sse::StreamConfig,
) -> BoxStream<'static, ChatChunk>
where
    S: futures::Stream<Item = std::result::Result<Bytes, E>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let sse_stream = sse::decode_sse_stream(byte_stream, cfg);
    parse_openai_events(sse_stream)
}

/// Translate a stream of `Result<SseEvent, SseError>` into `ChatChunk`s. Public
/// for the integration-test harness so it can drive the parser from a static
/// fixture without an HTTP roundtrip.
#[doc(hidden)]
pub fn parse_openai_events<S>(events: S) -> BoxStream<'static, ChatChunk>
where
    S: futures::Stream<Item = std::result::Result<SseEvent, SseError>> + Send + 'static,
{
    let mut acc = translate::OpenAiEventAccumulator::default();
    let stream = events.flat_map(move |item| {
        let chunks = match item {
            Ok(ev) => acc.consume(&ev),
            Err(e) => vec![ChatChunk::Error {
                kind: map_sse_error(&e),
                message: e.to_string(),
            }],
        };
        futures::stream::iter(chunks)
    });
    Box::pin(stream.fuse())
}

fn map_sse_error(e: &SseError) -> StreamErrorKind {
    match e {
        SseError::LineTooLong => StreamErrorKind::LineTooLong,
        SseError::IdleTimeout => StreamErrorKind::IdleTimeout,
        SseError::WallClockTimeout => StreamErrorKind::WallClockTimeout,
        SseError::Transport(_) => StreamErrorKind::Transport,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        // Walk char boundaries so we never split a multi-byte codepoint.
        let cut = s
            .char_indices()
            .take_while(|(i, _)| *i < max)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        format!("{}…", &s[..cut])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_config() {
        let p = OpenAiProvider::new("https://example.com", "sk-test", "gpt-4o");
        assert_eq!(p.base_url, "https://example.com");
        assert_eq!(p.api_key, "sk-test");
        assert_eq!(p.model, "gpt-4o");
        assert_eq!(p.max_tokens, None);
    }

    #[test]
    fn with_max_tokens_sets_value() {
        let p = OpenAiProvider::new("https://x", "sk", "gpt-4o").with_max_tokens(2048);
        assert_eq!(p.max_tokens, Some(2048));
    }

    #[test]
    fn truncate_does_not_panic_on_utf8_boundary() {
        // 'é' is two UTF-8 bytes; byte index 2 falls inside the codepoint, so a
        // naive `&s[..2]` slice panics. Should produce a valid prefix.
        let s = "héllo";
        let _ = truncate(s, 2);
    }

    #[test]
    fn truncate_short_input_returns_as_is() {
        assert_eq!(truncate("hi", 100), "hi");
    }

    #[test]
    fn map_sse_error_covers_all_variants() {
        assert_eq!(
            map_sse_error(&SseError::LineTooLong),
            StreamErrorKind::LineTooLong
        );
        assert_eq!(
            map_sse_error(&SseError::IdleTimeout),
            StreamErrorKind::IdleTimeout
        );
        assert_eq!(
            map_sse_error(&SseError::WallClockTimeout),
            StreamErrorKind::WallClockTimeout
        );
        assert_eq!(
            map_sse_error(&SseError::Transport("x".into())),
            StreamErrorKind::Transport
        );
    }
}
