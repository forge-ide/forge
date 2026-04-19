//! `fs.read` tool: reads a file via [`forge_fs::read_file`] and returns
//! `{ content, bytes, sha256 }` or `{ error }`.

use super::{get_optional_str, get_required_str, Tool, ToolCtx};
use forge_core::ApprovalPreview;

pub struct FsReadTool;

impl FsReadTool {
    pub const NAME: &'static str = "fs.read";
}

impl Tool for FsReadTool {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        // Preview shows whatever the client sent (including blank) so the user
        // sees exactly what was requested before approval. Required-argument
        // validation runs in `invoke` only — no point rejecting a malformed
        // call until the user has had a chance to refuse it. (F-074)
        let path = get_optional_str(args, "path").unwrap_or("");
        ApprovalPreview {
            description: format!("Read file '{path}'"),
        }
    }

    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        let path = match get_required_str(args, Self::NAME, "path") {
            Ok(p) => p,
            Err(e) => return serde_json::json!({ "error": e.to_string() }),
        };
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
