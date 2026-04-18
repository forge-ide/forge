//! `fs.write` tool: writes a file via [`forge_fs::write`] and returns
//! `{ ok: true }` or `{ error }`. Preview delegates to
//! [`forge_fs::write_preview`].

use super::{Tool, ToolCtx};
use forge_core::ApprovalPreview;

pub struct FsWriteTool;

impl Tool for FsWriteTool {
    fn name(&self) -> &str {
        "fs.write"
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let fs_preview = forge_fs::write_preview(path, content);
        ApprovalPreview {
            description: fs_preview.description,
        }
    }

    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        match forge_fs::write(path, content, &ctx.allowed_paths) {
            Ok(()) => serde_json::json!({ "ok": true }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        }
    }
}
