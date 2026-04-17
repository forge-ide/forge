use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use forge_core::Result;
use futures::stream::BoxStream;
use serde::Deserialize;

// ── Chat domain types ─────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub system: Option<String>,
    pub messages: Vec<ChatMessage>,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content: Vec<ChatBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChatRole {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub enum ChatBlock {
    Text(String),
    ToolCall {
        id: String,
        name: String,
        args: serde_json::Value,
    },
    ToolResult {
        id: String,
        result: serde_json::Value,
    },
}

// ── Stream chunk ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum ChatChunk {
    TextDelta(String),
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    Done(String),
}

// ── Provider trait ────────────────────────────────────────────────────────────

/// Streaming chat provider.
pub trait Provider: Send + Sync {
    fn chat(
        &self,
        req: ChatRequest,
    ) -> impl std::future::Future<Output = Result<BoxStream<'static, ChatChunk>>> + Send;
}

// ── NDJSON deserialization ────────────────────────────────────────────────────

#[derive(Deserialize)]
#[serde(untagged)]
enum RawChunk {
    Delta { delta: String },
    ToolCall { tool_call: RawToolCall },
    Done { done: String },
}

#[derive(Deserialize)]
struct RawToolCall {
    name: String,
    args: serde_json::Value,
}

fn parse_ndjson(content: &str) -> Result<Vec<ChatChunk>> {
    content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|line| {
            let raw: RawChunk = serde_json::from_str(line)?;
            Ok(match raw {
                RawChunk::Delta { delta } => ChatChunk::TextDelta(delta),
                RawChunk::ToolCall { tool_call } => ChatChunk::ToolCall {
                    name: tool_call.name,
                    args: tool_call.args,
                },
                RawChunk::Done { done } => ChatChunk::Done(done),
            })
        })
        .collect()
}

// ── MockProvider ──────────────────────────────────────────────────────────────

enum MockSource {
    File(PathBuf),
    Sequence {
        scripts: Arc<Mutex<VecDeque<String>>>,
        log: Arc<Mutex<Vec<ChatRequest>>>,
    },
}

/// Scripted provider for testing.
///
/// Two construction modes:
/// - `new(path)` — reads NDJSON from a file on every call
/// - `from_responses(scripts)` — pops the next script per `chat()` call,
///   and records every received `ChatRequest` (see `recorded_requests()`)
pub struct MockProvider {
    source: MockSource,
}

impl MockProvider {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            source: MockSource::File(path.as_ref().to_path_buf()),
        }
    }

    pub fn with_default_path() -> Self {
        let path = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("forge/mock.json");
        Self::new(path)
    }

    /// Construct from a sequence of NDJSON scripts, one per expected `chat()` call.
    pub fn from_responses(scripts: Vec<String>) -> Result<Self> {
        Ok(Self {
            source: MockSource::Sequence {
                scripts: Arc::new(Mutex::new(scripts.into_iter().collect())),
                log: Arc::new(Mutex::new(Vec::new())),
            },
        })
    }

    /// All `ChatRequest` values received in call order (sequence mode only).
    pub fn recorded_requests(&self) -> Vec<ChatRequest> {
        match &self.source {
            MockSource::Sequence { log, .. } => log.lock().unwrap().clone(),
            MockSource::File(_) => vec![],
        }
    }

    /// Stream directly from file (legacy; ignores request content).
    pub async fn stream(&self) -> Result<BoxStream<'static, ChatChunk>> {
        match &self.source {
            MockSource::File(path) => {
                let content = tokio::fs::read_to_string(path).await?;
                Ok(Box::pin(futures::stream::iter(parse_ndjson(&content)?)))
            }
            MockSource::Sequence { scripts, .. } => {
                let script = scripts.lock().unwrap().pop_front().unwrap_or_default();
                Ok(Box::pin(futures::stream::iter(parse_ndjson(&script)?)))
            }
        }
    }
}

impl Provider for MockProvider {
    fn chat(
        &self,
        req: ChatRequest,
    ) -> impl std::future::Future<Output = Result<BoxStream<'static, ChatChunk>>> + Send {
        match &self.source {
            MockSource::File(path) => {
                let path = path.clone();
                futures::future::Either::Left(async move {
                    let content = tokio::fs::read_to_string(&path).await?;
                    Ok(Box::pin(futures::stream::iter(parse_ndjson(&content)?))
                        as BoxStream<'static, ChatChunk>)
                })
            }
            MockSource::Sequence { scripts, log } => {
                log.lock().unwrap().push(req);
                let script = scripts.lock().unwrap().pop_front().unwrap_or_default();
                let result: Result<BoxStream<'static, ChatChunk>> =
                    parse_ndjson(&script).map(|c| Box::pin(futures::stream::iter(c)) as _);
                futures::future::Either::Right(async move { result })
            }
        }
    }
}
