use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum SessionPersistence {
    Persist,
    Ephemeral,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum SessionState {
    Active,
    Archived,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum RosterScope {
    SessionWide,
    Agent,
    Provider,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum ApprovalScope {
    Once,
    ThisFile,
    ThisPattern,
    ThisTool,
}

/// F-036: persistence tier for an approval.
///
/// `Session` stays in memory for the current session only; `Workspace` writes
/// to `<root>/.forge/approvals.toml`; `User` writes to `{config_dir}/forge/approvals.toml`.
/// Serialized lowercase on the wire so the frontend store can keep its
/// existing `'session' | 'workspace' | 'user'` string union.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "lowercase")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum ApprovalLevel {
    Session,
    Workspace,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum CompactTrigger {
    AutoAt98Pct,
    UserRequested,
}

/// F-143: variant selector for the `rerun_message` Tauri command.
///
/// Only [`RerunVariant::Replace`] is wired today — truncate the transcript
/// logically at the target message and regenerate the assistant response in
/// its place. [`RerunVariant::Branch`] (F-144) and [`RerunVariant::Fresh`]
/// (F-145) are reserved enum tags so the wire shape is stable across the
/// rerun milestone; dispatching them today returns an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum RerunVariant {
    Replace,
    Branch,
    Fresh,
}

/// F-139: step kinds emitted by the session turn loop. The Agent Monitor
/// (F-140) renders the sequence step-by-step, grouping tool/model/wait
/// segments under their parent turn.
///
/// `model` — one pass through the provider stream (text deltas, tool calls).
/// `tool`  — one tool invocation (start → invoke → return → complete).
/// `mcp`   — one MCP server tool call; rendered with `info-bg` chip so the
///           Agent Monitor separates MCP traffic from local tool calls at a
///           glance (see `docs/ui-specs/agent-monitor.md §9.2`).
/// `plan`  — reserved for future agent planning phases; not emitted today.
/// `wait`  — reserved for approval/idle gaps; not emitted today.
/// `spawn` — reserved for sub-agent spawn steps (F-140); not emitted today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum StepKind {
    Plan,
    Tool,
    Mcp,
    Model,
    Wait,
    Spawn,
}

/// F-139: terminal status for a step.
///
/// `ok` — step completed normally (the common case).
/// `error { reason }` — step failed; the reason is a short, display-safe
/// human-readable string. Intentionally minimal; structured failure
/// payloads stay on the underlying event (`ToolCallCompleted.result`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum StepOutcome {
    Ok,
    Error { reason: String },
}

/// F-139: per-step token accounting. Mirrors `UsageTick` field names
/// (`tokens_in`, `tokens_out`) so dashboards can sum step-level with
/// session-level without a field-name remap. `None` on `StepFinished`
/// means the provider didn't report usage for this step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub struct TokenUsage {
    pub tokens_in: u64,
    pub tokens_out: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn session_persistence_serde_roundtrip() {
        for v in [SessionPersistence::Persist, SessionPersistence::Ephemeral] {
            let json = serde_json::to_string(&v).unwrap();
            let decoded: SessionPersistence = serde_json::from_str(&json).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn session_state_serde_roundtrip() {
        for v in [
            SessionState::Active,
            SessionState::Archived,
            SessionState::Ended,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let decoded: SessionState = serde_json::from_str(&json).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn roster_scope_serde_roundtrip() {
        for v in [
            RosterScope::SessionWide,
            RosterScope::Agent,
            RosterScope::Provider,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let decoded: RosterScope = serde_json::from_str(&json).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn approval_scope_serde_roundtrip() {
        for v in [
            ApprovalScope::Once,
            ApprovalScope::ThisFile,
            ApprovalScope::ThisPattern,
            ApprovalScope::ThisTool,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let decoded: ApprovalScope = serde_json::from_str(&json).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn approval_level_serde_roundtrip() {
        for v in [
            ApprovalLevel::Session,
            ApprovalLevel::Workspace,
            ApprovalLevel::User,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let decoded: ApprovalLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(v, decoded);
        }
    }

    #[test]
    fn approval_level_serializes_lowercase() {
        // Frontend stores `level: 'session' | 'workspace' | 'user'`. Lock the
        // wire shape so a rename on the Rust side surfaces as a test failure.
        assert_eq!(
            serde_json::to_string(&ApprovalLevel::Session).unwrap(),
            "\"session\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalLevel::Workspace).unwrap(),
            "\"workspace\""
        );
        assert_eq!(
            serde_json::to_string(&ApprovalLevel::User).unwrap(),
            "\"user\""
        );
    }

    #[test]
    fn step_kind_serializes_snake_case() {
        // F-381: AgentMonitor.tsx reads `StepStarted.kind` and lowercases
        // it before matching `'plan' | 'tool' | 'model' | 'wait' | 'spawn'`.
        // Lock the wire shape so the ts-rs binding and the frontend
        // comparison agree.
        for (kind, wire) in [
            (StepKind::Plan, "\"plan\""),
            (StepKind::Tool, "\"tool\""),
            (StepKind::Mcp, "\"mcp\""),
            (StepKind::Model, "\"model\""),
            (StepKind::Wait, "\"wait\""),
            (StepKind::Spawn, "\"spawn\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), wire);
            let decoded: StepKind = serde_json::from_str(wire).unwrap();
            assert_eq!(decoded, kind);
        }
    }

    #[test]
    fn step_outcome_wire_shape_tagged_snake_case() {
        // F-381: `StepFinished.outcome` is a tagged union with `status: "ok" | "error"`.
        // Lock both tags and the error payload field name.
        assert_eq!(
            serde_json::to_string(&StepOutcome::Ok).unwrap(),
            "{\"status\":\"ok\"}"
        );
        assert_eq!(
            serde_json::to_string(&StepOutcome::Error {
                reason: "boom".into()
            })
            .unwrap(),
            "{\"status\":\"error\",\"reason\":\"boom\"}"
        );
    }

    #[test]
    fn token_usage_wire_shape() {
        // F-381: `StepFinished.token_usage` mirrors `UsageTick` field names.
        // Field-name remap on either side must surface here.
        let usage = TokenUsage {
            tokens_in: 42,
            tokens_out: 7,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert_eq!(json, "{\"tokens_in\":42,\"tokens_out\":7}");
        let decoded: TokenUsage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, usage);
    }
}
