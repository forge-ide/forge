use rand::Rng;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

macro_rules! id_type {
    ($(#[$attr:meta])* $name:ident) => {
        $(#[$attr])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[ts(export, export_to = "../../../web/packages/ipc/src/generated/", type = "string")]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                let bytes: [u8; 8] = rand::thread_rng().gen();
                Self(bytes.iter().map(|b| format!("{b:02x}")).collect())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

id_type!(SessionId);
id_type!(WorkspaceId);
id_type!(AgentId);
id_type!(ProviderId);
id_type!(MessageId);
id_type!(ToolCallId);
id_type!(AgentInstanceId);

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn session_id_new_is_unique() {
        assert_ne!(SessionId::new(), SessionId::new());
    }

    #[test]
    fn session_id_serde_roundtrip() {
        let id = SessionId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert!(json.starts_with('"'), "should serialize as JSON string");
        let decoded: SessionId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, decoded);
    }

    #[test]
    fn session_id_display_is_hex() {
        let id = SessionId(String::from("deadbeefcafebabe"));
        assert_eq!(id.to_string(), "deadbeefcafebabe");
    }

    #[test]
    fn all_id_types_serde_roundtrip() {
        macro_rules! check {
            ($t:ident) => {
                let id = $t::new();
                let json = serde_json::to_string(&id).unwrap();
                let decoded: $t = serde_json::from_str(&json).unwrap();
                assert_eq!(id, decoded);
            };
        }
        check!(WorkspaceId);
        check!(AgentId);
        check!(ProviderId);
        check!(MessageId);
        check!(ToolCallId);
        check!(AgentInstanceId);
    }
}
