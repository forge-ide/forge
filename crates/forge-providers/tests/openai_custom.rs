//! Integration tests for [`CustomOpenAiProvider`] (F-585).
//!
//! Drive each `AuthShape` variant through a wiremock-backed mock OpenAI
//! server and assert:
//!
//! 1. The expected auth header is present (or absent, for `AuthShape::None`).
//! 2. The body successfully reaches the same SSE-decode pipeline used by
//!    [`forge_providers::openai::OpenAiProvider`] — i.e. the same chunks come
//!    out for the same fixture.
//!
//! The fixture is reused from `tests/fixtures/openai_text_and_tool_use.sse`
//! because the wire shape is identical between vanilla OpenAI and any
//! OpenAI-compatible server. The test's job is to prove the auth-header
//! plumbing is correct, not to re-validate the SSE decoder.

use forge_providers::openai::{AuthShape, CustomOpenAiProvider};
use forge_providers::{ChatBlock, ChatChunk, ChatMessage, ChatRequest, ChatRole, Provider};
use futures::stream::StreamExt;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TEXT_AND_TOOL_USE_FIXTURE: &str = include_str!("fixtures/openai_text_and_tool_use.sse");

fn user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content: vec![ChatBlock::Text(text.into())],
    }
}

fn req() -> ChatRequest {
    ChatRequest {
        system: None,
        messages: vec![user_msg("hi")],
        parallel_tool_calls_allowed: false,
    }
}

fn expected_chunks() -> Vec<ChatChunk> {
    vec![
        ChatChunk::TextDelta("Hello".into()),
        ChatChunk::TextDelta(" world".into()),
        ChatChunk::ToolCall {
            name: "get_weather".into(),
            args: serde_json::json!({"city": "sf"}),
        },
        ChatChunk::Done("tool_calls".into()),
    ]
}

#[tokio::test(flavor = "multi_thread")]
async fn bearer_auth_sends_authorization_header_and_decodes_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-test"))
        .and(header("content-type", "application/json"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(TEXT_AND_TOOL_USE_FIXTURE),
        )
        .mount(&server)
        .await;

    // wiremock binds 127.0.0.1, which the SSRF guard accepts under the
    // loopback exception in debug builds (where tests run).
    let provider = CustomOpenAiProvider::new(
        "vllm-local",
        server.uri(),
        "gpt-4o",
        vec!["gpt-4o".into()],
        AuthShape::Bearer,
        Some("sk-test".into()),
    )
    .expect("provider construction");

    let mut stream = provider.chat(req()).await.expect("chat call succeeds");
    let mut chunks = Vec::new();
    while let Some(c) = stream.next().await {
        chunks.push(c);
    }
    assert_eq!(chunks, expected_chunks());
}

#[tokio::test(flavor = "multi_thread")]
async fn header_auth_sends_named_header_and_decodes_stream() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("x-api-key", "sk-secret"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(TEXT_AND_TOOL_USE_FIXTURE),
        )
        .mount(&server)
        .await;

    let provider = CustomOpenAiProvider::new(
        "gateway",
        server.uri(),
        "gpt-4o",
        vec!["gpt-4o".into()],
        AuthShape::Header {
            name: "X-API-Key".into(),
        },
        Some("sk-secret".into()),
    )
    .expect("provider construction");

    let mut stream = provider.chat(req()).await.expect("chat call succeeds");
    let mut chunks = Vec::new();
    while let Some(c) = stream.next().await {
        chunks.push(c);
    }
    assert_eq!(chunks, expected_chunks());
}

#[tokio::test(flavor = "multi_thread")]
async fn none_auth_sends_no_auth_header_and_decodes_stream() {
    let server = MockServer::start().await;
    // The matcher must fail if any auth header sneaks through. We mount one
    // accepting mock for the request shape we expect, plus a "negative"
    // assertion encoded as the absence of `authorization` and `x-api-key` in
    // the served route. wiremock cannot directly express "header X must NOT
    // be present", but `expect(1)` on the no-auth-header matcher is
    // sufficient: if the client erroneously sent auth, it would still match,
    // since the served mock has no header constraints. To guarantee absence,
    // we add a second mock that *does* match an authorization header and
    // expect 0 hits.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(TEXT_AND_TOOL_USE_FIXTURE),
        )
        .mount(&server)
        .await;

    let provider = CustomOpenAiProvider::new(
        "no-auth",
        server.uri(),
        "gpt-4o",
        vec!["gpt-4o".into()],
        AuthShape::None,
        None,
    )
    .expect("provider construction");

    let mut stream = provider.chat(req()).await.expect("chat call succeeds");
    let mut chunks = Vec::new();
    while let Some(c) = stream.next().await {
        chunks.push(c);
    }
    assert_eq!(chunks, expected_chunks());
    // wiremock's expect(0) is asserted on MockServer drop — leaving scope
    // here triggers the verification.
}

#[tokio::test(flavor = "multi_thread")]
async fn http_error_propagates_with_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid api key"))
        .mount(&server)
        .await;

    let provider = CustomOpenAiProvider::new(
        "x",
        server.uri(),
        "gpt-4o",
        vec![],
        AuthShape::Bearer,
        Some("bad".into()),
    )
    .expect("provider construction");

    let err = provider
        .chat(req())
        .await
        .err()
        .expect("HTTP 401 must map to Err");
    let msg = format!("{err}");
    assert!(msg.contains("401"), "error should mention status: {msg}");
    assert!(
        msg.contains("invalid api key"),
        "error should include body: {msg}"
    );
}
