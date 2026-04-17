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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../../web/packages/ipc/src/generated/")]
pub enum CompactTrigger {
    AutoAt98Pct,
    UserRequested,
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
}
