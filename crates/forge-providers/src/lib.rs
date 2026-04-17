use std::path::{Path, PathBuf};

use forge_core::Result;
use futures::stream::BoxStream;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
pub enum ChatChunk {
    TextDelta(String),
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    Done(String),
}

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

pub struct MockProvider {
    path: PathBuf,
}

impl MockProvider {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn with_default_path() -> Self {
        let path = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("forge/mock.json");
        Self { path }
    }

    pub async fn stream(&self) -> Result<BoxStream<'static, ChatChunk>> {
        let content = tokio::fs::read_to_string(&self.path).await?;
        let chunks: Vec<ChatChunk> = content
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
            .collect::<forge_core::Result<Vec<_>>>()?;

        Ok(Box::pin(futures::stream::iter(chunks)))
    }
}
