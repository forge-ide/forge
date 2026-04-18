//! Ollama provider — NDJSON streaming against a local Ollama daemon.
//!
//! No credentials; the daemon is expected at `http://127.0.0.1:11434` by default.

use crate::{ChatChunk, ChatMessage, ChatRequest, Provider};
use forge_core::Result;
use futures::stream::{BoxStream, StreamExt};

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

pub struct OllamaProvider {
    base_url: String,
    model: String,
    client: reqwest::Client,
}

impl OllamaProvider {
    pub fn new(base_url: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            model: model.into(),
            client: reqwest::Client::new(),
        }
    }

    pub fn with_default_url(model: impl Into<String>) -> Self {
        Self::new(DEFAULT_BASE_URL, model)
    }

    pub async fn list_models(&self) -> Result<Vec<String>> {
        let url = format!("{}/api/tags", self.base_url.trim_end_matches('/'));
        let resp = self
            .client
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

        let value: serde_json::Value = resp
            .json()
            .await
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
        let client = self.client.clone();

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

            let byte_stream = resp.bytes_stream();

            // Buffered NDJSON decoder: accumulate bytes, flush complete lines per chunk.
            let state = (
                byte_stream,
                String::new(),
                std::collections::VecDeque::<ChatChunk>::new(),
                false,
            );
            let chunk_stream = futures::stream::unfold(
                state,
                |(mut bytes, mut buf, mut pending, mut upstream_done)| async move {
                    loop {
                        if let Some(chunk) = pending.pop_front() {
                            return Some((chunk, (bytes, buf, pending, upstream_done)));
                        }

                        if upstream_done {
                            // Flush any trailing partial line as a final parse attempt.
                            if !buf.is_empty() {
                                let line = std::mem::take(&mut buf);
                                if let Some(c) = parse_line(line.trim()) {
                                    return Some((c, (bytes, buf, pending, upstream_done)));
                                }
                            }
                            return None;
                        }

                        match bytes.next().await {
                            Some(Ok(b)) => {
                                buf.push_str(&String::from_utf8_lossy(&b));
                                while let Some(pos) = buf.find('\n') {
                                    let line: String = buf.drain(..=pos).collect();
                                    let trimmed = line.trim();
                                    if trimmed.is_empty() {
                                        continue;
                                    }
                                    if let Some(c) = parse_line(trimmed) {
                                        pending.push_back(c);
                                    }
                                }
                            }
                            Some(Err(_)) | None => {
                                upstream_done = true;
                            }
                        }
                    }
                },
            );

            Ok(Box::pin(chunk_stream) as BoxStream<'static, ChatChunk>)
        }
    }
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
