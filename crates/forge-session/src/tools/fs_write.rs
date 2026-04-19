//! `fs.write` tool: writes a file via [`forge_fs::write`] and returns
//! `{ ok: true }` or `{ error }`. Preview delegates to
//! [`forge_fs::write_preview`].

use super::{get_required_str, Tool, ToolCtx};
use forge_core::ApprovalPreview;

pub struct FsWriteTool;

impl FsWriteTool {
    pub const NAME: &'static str = "fs.write";
}

impl Tool for FsWriteTool {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        // Preview reflects whatever the client sent so the approval UI shows
        // the literal request; `invoke` performs the required-arg check (F-074).
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let fs_preview = forge_fs::write_preview(path, content);
        ApprovalPreview {
            description: fs_preview.description,
        }
    }

    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        let path = match get_required_str(args, Self::NAME, "path") {
            Ok(p) => p,
            Err(e) => return serde_json::json!({ "error": e.to_string() }),
        };
        let content = match get_required_str(args, Self::NAME, "content") {
            Ok(c) => c,
            Err(e) => return serde_json::json!({ "error": e.to_string() }),
        };
        match forge_fs::write(
            path,
            content,
            &ctx.allowed_paths,
            &forge_fs::Limits::default(),
        ) {
            Ok(()) => serde_json::json!({ "ok": true }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        }
    }
}
