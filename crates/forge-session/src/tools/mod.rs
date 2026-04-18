//! Tool dispatch: name → handler routing for orchestrator tool calls.

use forge_core::ApprovalPreview;

pub mod fs_edit;
pub mod fs_read;
pub mod fs_write;

pub use fs_edit::FsEditTool;
pub use fs_read::FsReadTool;
pub use fs_write::FsWriteTool;

pub struct ToolCtx {
    pub allowed_paths: Vec<String>,
}

pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn approval_preview(&self, args: &serde_json::Value) -> ApprovalPreview;
    fn invoke(&self, args: &serde_json::Value, ctx: &ToolCtx) -> serde_json::Value;
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ToolError {
    #[error("tool '{0}' is already registered")]
    DuplicateName(String),
    #[error("unknown tool '{0}'")]
    UnknownTool(String),
}

#[derive(Default)]
pub struct ToolDispatcher {
    tools: std::collections::HashMap<String, Box<dyn Tool>>,
}

impl ToolDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) -> Result<(), ToolError> {
        let name = tool.name().to_string();
        if self.tools.contains_key(&name) {
            return Err(ToolError::DuplicateName(name));
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<&dyn Tool, ToolError> {
        self.tools
            .get(name)
            .map(|b| b.as_ref())
            .ok_or_else(|| ToolError::UnknownTool(name.to_string()))
    }

    pub fn dispatch(
        &self,
        name: &str,
        args: &serde_json::Value,
        ctx: &ToolCtx,
    ) -> Result<serde_json::Value, ToolError> {
        Ok(self.get(name)?.invoke(args, ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::NamedTempFile;

    struct StubTool {
        name: &'static str,
        response: serde_json::Value,
    }

    impl Tool for StubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn approval_preview(&self, _args: &serde_json::Value) -> ApprovalPreview {
            ApprovalPreview {
                description: format!("stub: {}", self.name),
            }
        }
        fn invoke(&self, _args: &serde_json::Value, _ctx: &ToolCtx) -> serde_json::Value {
            self.response.clone()
        }
    }

    fn empty_ctx() -> ToolCtx {
        ToolCtx {
            allowed_paths: vec![],
        }
    }

    #[test]
    fn register_and_dispatch_returns_tool_result() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(StubTool {
            name: "noop",
            response: json!({"ok": true}),
        }))
        .unwrap();

        let result = d.dispatch("noop", &json!({}), &empty_ctx()).unwrap();
        assert_eq!(result, json!({"ok": true}));
    }

    #[test]
    fn duplicate_registration_returns_error() {
        let mut d = ToolDispatcher::new();
        d.register(Box::new(StubTool {
            name: "noop",
            response: json!({}),
        }))
        .unwrap();

        let err = d
            .register(Box::new(StubTool {
                name: "noop",
                response: json!({}),
            }))
            .unwrap_err();
        assert_eq!(err, ToolError::DuplicateName("noop".to_string()));
    }

    #[test]
    fn dispatch_unknown_tool_returns_error() {
        let d = ToolDispatcher::new();
        let err = d.dispatch("nope", &json!({}), &empty_ctx()).unwrap_err();
        assert_eq!(err, ToolError::UnknownTool("nope".to_string()));
    }

    #[test]
    fn fs_write_dispatch_writes_file_and_previews_diff() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("out.txt");
        let canonical_parent = std::fs::canonicalize(dir.path()).unwrap();
        let allowed = format!("{}/**", canonical_parent.to_str().unwrap());

        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsWriteTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![allowed],
        };
        let result = d
            .dispatch(
                "fs.write",
                &json!({"path": target.to_str().unwrap(), "content": "hi"}),
                &ctx,
            )
            .unwrap();
        assert_eq!(result["ok"].as_bool(), Some(true));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "hi");

        let preview = d
            .get("fs.write")
            .unwrap()
            .approval_preview(&json!({"path": target.to_str().unwrap(), "content": "hi"}));
        assert!(preview.description.contains("Write file"));
    }

    #[test]
    fn fs_edit_dispatch_applies_patch_and_previews_diff() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("src.txt");
        std::fs::write(&target, "alpha\nbeta\n").unwrap();
        let canonical_parent = std::fs::canonicalize(dir.path()).unwrap();
        let allowed = format!("{}/**", canonical_parent.to_str().unwrap());

        let patch = similar::TextDiff::from_lines("alpha\nbeta\n", "alpha\nBETA\n")
            .unified_diff()
            .to_string();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsEditTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![allowed],
        };
        let result = d
            .dispatch(
                "fs.edit",
                &json!({"path": target.to_str().unwrap(), "patch": patch}),
                &ctx,
            )
            .unwrap();
        assert_eq!(result["ok"].as_bool(), Some(true));
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "alpha\nBETA\n");

        let preview = d
            .get("fs.edit")
            .unwrap()
            .approval_preview(&json!({"path": target.to_str().unwrap(), "patch": patch}));
        assert!(preview.description.contains("Edit file"));
    }

    #[test]
    fn fs_read_dispatch_returns_content_bytes_sha256() {
        let mut file = NamedTempFile::new().unwrap();
        let body = "hello dispatcher";
        file.write_all(body.as_bytes()).unwrap();
        let path = file.path().to_str().unwrap().to_string();
        let canonical = std::fs::canonicalize(&path).unwrap();
        let allowed = canonical.to_str().unwrap().to_string();

        let mut d = ToolDispatcher::new();
        d.register(Box::new(FsReadTool)).unwrap();

        let ctx = ToolCtx {
            allowed_paths: vec![allowed],
        };
        let result = d.dispatch("fs.read", &json!({"path": path}), &ctx).unwrap();

        assert_eq!(result["content"].as_str().unwrap(), body);
        assert_eq!(result["bytes"].as_u64().unwrap(), body.len() as u64);
        assert_eq!(result["sha256"].as_str().unwrap().len(), 64);
    }
}
