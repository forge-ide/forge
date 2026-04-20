use std::sync::Arc;

use rand::Rng;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

// F-107: IDs store their hex payload in `Arc<str>` so `.clone()` on a hot
// path (orchestrator event emission, per-token AssistantDelta) is a
// ref-count bump rather than a heap allocation. Public API — constructors,
// `Display`, `Serialize`/`Deserialize`, `PartialEq`, `Hash` — is unchanged;
// the wire shape remains a bare JSON string and ts-rs still emits
// `export type <Id> = string`. See the `*_clone_is_shared_allocation`
// tests at the bottom of the file for the contract.
macro_rules! id_type {
    ($(#[$attr:meta])* $name:ident) => {
        id_type!($(#[$attr])* $name, 8);
    };
    ($(#[$attr:meta])* $name:ident, $bytes:literal) => {
        $(#[$attr])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
        #[ts(export, export_to = "../../../web/packages/ipc/src/generated/", type = "string")]
        // serde's built-in `Deserialize` for `Arc<str>` is gated behind the
        // `rc` feature (not enabled here), so we go through `String` for the
        // wire round-trip. The wrap cost only pays at construction time;
        // cloning stays O(refcount_bump).
        #[serde(from = "String", into = "String")]
        pub struct $name(Arc<str>);

        impl $name {
            pub fn new() -> Self {
                let bytes: [u8; $bytes] = rand::thread_rng().gen();
                let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
                Self(Arc::from(hex))
            }

            /// Wrap an existing string as this id type. Use when the id was
            /// generated elsewhere (e.g. read from env, a meta file, or a
            /// client message) and the wire shape is just a string.
            pub fn from_string(s: String) -> Self {
                Self(Arc::from(s))
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

        // Bridges for `#[serde(from = "String", into = "String")]`.
        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(Arc::from(s))
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> Self {
                id.0.as_ref().to_owned()
            }
        }
    };
}

id_type!(SessionId);
id_type!(WorkspaceId);
id_type!(AgentId);
id_type!(ProviderId);
id_type!(MessageId);
// F-139: identifies one agent-step within a turn. 64-bit entropy is
// sufficient — the id space is per-turn (each session owns its own emission
// cursor) and never used as an authorization handle. Round-trips through a
// bare JSON string so the Agent Monitor (F-140) can key step rows by
// string equality without a custom decoder.
id_type!(StepId);
// F-067: 128-bit entropy (16 bytes → 32 hex chars). Defense-in-depth
// against local online guessing when combined with UDS-permission defects
// (see H8/F-044). Other IDs stay at 64 bits — they're validated at narrower
// entry points (SessionId is gated by `^[0-9a-f]{16}$` per F-057/F-063).
id_type!(ToolCallId, 16);
id_type!(AgentInstanceId);
// F-125: identifies a terminal session within a single session window.
// 64-bit entropy is sufficient — the id space is per-window (each `session-*`
// webview owns its own `TerminalRegistry`), never serialized to disk, and the
// command handlers verify the calling webview owns the terminal before each
// operation. See `forge-shell::ipc::TerminalRegistry`.
id_type!(TerminalId);

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
        let id = SessionId(Arc::from("deadbeefcafebabe"));
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

    // F-107: cloning an id must be a ref-count bump, not a heap allocation.
    // Hot-path emission sites in `forge-session::orchestrator` clone IDs
    // (msg_id, provider_id, call_id) multiple times per streamed token; any
    // per-clone allocation is observable as allocator pressure on long
    // responses. We enforce the contract at the type level: identical
    // byte-pointer after clone == shared backing buffer.
    #[test]
    fn message_id_clone_is_shared_allocation() {
        let id = MessageId::new();
        let cloned = id.clone();
        assert_eq!(
            id.0.as_ptr(),
            cloned.0.as_ptr(),
            "MessageId::clone() must share its backing buffer (Arc<str> or equivalent) \
             to keep per-token cost O(refcount_bump)"
        );
    }

    #[test]
    fn provider_id_clone_is_shared_allocation() {
        let id = ProviderId::new();
        let cloned = id.clone();
        assert_eq!(
            id.0.as_ptr(),
            cloned.0.as_ptr(),
            "ProviderId::clone() must share its backing buffer"
        );
    }

    #[test]
    fn tool_call_id_clone_is_shared_allocation() {
        let id = ToolCallId::new();
        let cloned = id.clone();
        assert_eq!(
            id.0.as_ptr(),
            cloned.0.as_ptr(),
            "ToolCallId::clone() must share its backing buffer"
        );
    }
}
