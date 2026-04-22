//! `fs.edit` tool: applies a unified-diff patch via [`forge_fs::edit`] and
//! returns `{ ok: true }` or `{ error }`. Preview delegates to
//! [`forge_fs::edit_preview`].
//!
//! F-106: `forge_fs::edit` reads + writes synchronously (~100–200 ms on a
//! 10 MB patch target), so running it on a tokio worker stalls
//! concurrent-session streaming. The call is wrapped in
//! `tokio::task::spawn_blocking` so only the blocking pool stalls.

use super::{get_optional_str, get_required_str, Tool, ToolCtx};
use forge_core::ApprovalPreview;

pub struct FsEditTool;

impl FsEditTool {
    pub const NAME: &'static str = "fs.edit";
}

#[async_trait::async_trait]
impl Tool for FsEditTool {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview {
        // Preview reflects whatever the client sent so the approval UI shows
        // the literal request; `invoke` performs the required-arg check (F-074).
        let path = get_optional_str(args, "path").unwrap_or("");
        let patch = get_optional_str(args, "patch").unwrap_or("");
        ApprovalPreview {
            description: forge_fs::edit_preview(path, patch),
        }
    }

    async fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value {
        let path = match get_required_str(args, Self::NAME, "path") {
            Ok(p) => p.to_owned(),
            Err(e) => return serde_json::json!({ "error": e.to_string() }),
        };
        let patch = match get_required_str(args, Self::NAME, "patch") {
            Ok(p) => p.to_owned(),
            Err(e) => return serde_json::json!({ "error": e.to_string() }),
        };
        let allowed_paths = ctx.allowed_paths.clone();
        // F-106: move the synchronous read+write edit off the tokio worker.
        let result = tokio::task::spawn_blocking(move || {
            forge_fs::edit(&path, &patch, &allowed_paths, &forge_fs::Limits::default())
        })
        .await;
        match result {
            Ok(Ok(())) => serde_json::json!({ "ok": true }),
            Ok(Err(e)) => serde_json::json!({ "error": e.to_string() }),
            Err(join_err) => {
                serde_json::json!({ "error": format!("fs.edit blocking task failed: {join_err}") })
            }
        }
    }
}
