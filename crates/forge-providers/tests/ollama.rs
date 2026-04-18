//! Integration tests for `OllamaProvider` using a mocked `reqwest` server.

use forge_providers::ollama::OllamaProvider;
use forge_providers::{ChatBlock, ChatChunk, ChatMessage, ChatRequest, ChatRole, Provider};
use futures::StreamExt;
use wiremock::matchers::{body_partial_json, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ndjson_body(lines: &[&str]) -> String {
    let mut s = lines.join("\n");
    s.push('\n');
    s
}

fn user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        role: ChatRole::User,
        content: vec![ChatBlock::Text(text.into())],
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_streams_text_tool_call_and_done() {
    let server = MockServer::start().await;

    let body = ndjson_body(&[
        r#"{"message":{"role":"assistant","content":"Hel"},"done":false}"#,
        r#"{"message":{"role":"assistant","content":"lo"},"done":false}"#,
        r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"fs.read","arguments":{"path":"/x"}}}]},"done":false}"#,
        r#"{"model":"llama3","done":true,"done_reason":"stop"}"#,
    ]);

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .and(body_partial_json(serde_json::json!({
            "model": "llama3",
            "stream": true,
        })))
        .respond_with(ResponseTemplate::new(200).set_body_string(body))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(server.uri(), "llama3");

    let req = ChatRequest {
        system: Some("be helpful".into()),
        messages: vec![user_msg("hi")],
    };
    let mut stream = provider.chat(req).await.expect("chat call succeeds");

    let mut chunks = Vec::new();
    while let Some(chunk) = stream.next().await {
        chunks.push(chunk);
    }

    assert_eq!(
        chunks,
        vec![
            ChatChunk::TextDelta("Hel".into()),
            ChatChunk::TextDelta("lo".into()),
            ChatChunk::ToolCall {
                name: "fs.read".into(),
                args: serde_json::json!({"path": "/x"}),
            },
            ChatChunk::Done("stop".into()),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_maps_http_errors() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/chat"))
        .respond_with(ResponseTemplate::new(500).set_body_string("model not found"))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(server.uri(), "llama3");
    let req = ChatRequest {
        system: None,
        messages: vec![user_msg("hi")],
    };

    let err = match provider.chat(req).await {
        Ok(_) => panic!("HTTP 500 must map to an error"),
        Err(e) => e,
    };
    let msg = format!("{err}");
    assert!(msg.contains("500"), "error should mention status: {msg}");
    assert!(
        msg.contains("model not found"),
        "error should include body: {msg}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn list_models_returns_tag_names() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/api/tags"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "models": [
                {"name": "llama3"},
                {"name": "mistral"},
            ]
        })))
        .mount(&server)
        .await;

    let provider = OllamaProvider::new(server.uri(), "llama3");
    let models = provider.list_models().await.expect("list_models succeeds");
    assert_eq!(models, vec!["llama3", "mistral"]);
}
