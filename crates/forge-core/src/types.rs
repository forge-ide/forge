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
}
