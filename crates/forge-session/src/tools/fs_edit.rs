//! `fs.edit` tool: applies a unified-diff patch via [`forge_fs::edit`] and
//! returns `{ ok: true }` or `{ error }`. Preview delegates to
//! [`forge_fs::edit_preview`].

use super::{Tool, ToolCtx};
use forge_core::ApprovalPreview;

pub struct FsEditTool;

impl Tool for FsEditTool {
    fn name(&self) -> &str {
        "fs.edit"
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let patch = args.get("patch").and_then(|v| v.as_str()).unwrap_or("");
        let fs_preview = forge_fs::edit_preview(path, patch);
        ApprovalPreview {
            description: fs_preview.description,
        }
    }

    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let patch = args.get("patch").and_then(|v| v.as_str()).unwrap_or("");
        match forge_fs::edit(
            path,
            patch,
            &ctx.allowed_paths,
            &forge_fs::Limits::default(),
        ) {
            Ok(()) => serde_json::json!({ "ok": true }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        }
    }
}
