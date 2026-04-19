//! `fs.read` tool: reads a file via [`forge_fs::read_file`] and returns
//! `{ content, bytes, sha256 }` or `{ error }`.

use super::{Tool, ToolCtx};
use forge_core::ApprovalPreview;

pub struct FsReadTool;

impl Tool for FsReadTool {
    fn name(&self) -> &str {
        "fs.read"
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        ApprovalPreview {
            description: format!("Read file '{path}'"),
        }
    }

    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        match forge_fs::read_file(path, &ctx.allowed_paths, &forge_fs::Limits::default()) {
            Ok(r) => serde_json::json!({
                "content": r.content,
                "bytes": r.bytes,
                "sha256": r.sha256,
            }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        }
    }
}
