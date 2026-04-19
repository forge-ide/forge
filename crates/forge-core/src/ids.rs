use rand::Rng;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

macro_rules! id_type {
    ($(#[$attr:meta])* $name:ident) => {
        id_type!($(#[$attr])* $name, 8);
    };
    ($(#[$attr:meta])* $name:ident, $bytes:literal) => {
        $(#[$attr])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[ts(export, export_to = "../../../web/packages/ipc/src/generated/", type = "string")]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                let bytes: [u8; $bytes] = rand::thread_rng().gen();
                Self(bytes.iter().map(|b| format!("{b:02x}")).collect())
            }

            /// Wrap an existing string as this id type. Use when the id was
            /// generated elsewhere (e.g. read from env, a meta file, or a
            /// client message) and the wire shape is just a string.
            pub fn from_string(s: String) -> Self {
                Self(s)
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
// F-067: 128-bit entropy (16 bytes → 32 hex chars). Defense-in-depth
// against local online guessing when combined with UDS-permission defects
// (see H8/F-044). Other IDs stay at 64 bits — they're validated at narrower
// entry points (SessionId is gated by `^[0-9a-f]{16}$` per F-057/F-063).
id_type!(ToolCallId, 16);
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

    // F-067: ToolCallId uses 128-bit entropy (16 bytes -> 32 hex chars).
    // SessionId deliberately stays at 64-bit because its canonical format is
    // validated by `^[0-9a-f]{16}$` at F-057 (CLI) and F-063 (dashboard).
    #[test]
    fn tool_call_id_is_128_bits_of_hex() {
        let id = ToolCallId::new();
        assert_eq!(id.to_string().len(), 32, "expected 32 hex chars (16 bytes)");
        assert!(id.to_string().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn session_id_is_64_bits_of_hex() {
        let id = SessionId::new();
        assert_eq!(
            id.to_string().len(),
            16,
            "SessionId must remain 16 hex chars to match F-057/F-063 validators"
        );
    }
}
