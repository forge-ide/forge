//! `fs.edit` tool: applies a unified-diff patch via [`forge_fs::edit`] and
//! returns `{ ok: true }` or `{ error }`. Preview delegates to
//! [`forge_fs::edit_preview`].

use super::{get_required_str, Tool, ToolCtx};
use forge_core::ApprovalPreview;

pub struct FsEditTool;

impl FsEditTool {
    pub const NAME: &'static str = "fs.edit";
}

impl Tool for FsEditTool {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        // Preview reflects whatever the client sent so the approval UI shows
        // the literal request; `invoke` performs the required-arg check (F-074).
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let patch = args.get("patch").and_then(|v| v.as_str()).unwrap_or("");
        let fs_preview = forge_fs::edit_preview(path, patch);
        ApprovalPreview {
            description: fs_preview.description,
        }
    }

    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        let path = match get_required_str(args, Self::NAME, "path") {
            Ok(p) => p,
            Err(e) => return serde_json::json!({ "error": e.to_string() }),
        };
        let patch = match get_required_str(args, Self::NAME, "patch") {
            Ok(p) => p,
            Err(e) => return serde_json::json!({ "error": e.to_string() }),
        };
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
