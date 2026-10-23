//! Prefixed ULID identifiers. Every ID on the wire is `<prefix>_<ULID>`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use ulid::Ulid;

/// An ID string that doesn't have the expected prefix or isn't a valid ULID.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} id {value:?}: expected {prefix}<ULID>")]
pub struct InvalidId {
    pub kind: &'static str,
    pub prefix: &'static str,
    pub value: String,
}

macro_rules! prefixed_id {
    ($(#[$doc:meta])* $name:ident, $prefix:literal) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
        #[serde(try_from = "String", into = "String")]
        #[ts(type = "string")]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            /// A fresh ID with the current time.
            pub fn new() -> Self {
                Self(format!("{}{}", $prefix, Ulid::new()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl FromStr for $name {
            type Err = InvalidId;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let invalid = || InvalidId {
                    kind: stringify!($name),
                    prefix: $prefix,
                    value: s.to_owned(),
                };
                let rest = s.strip_prefix($prefix).ok_or_else(invalid)?;
                let ulid = Ulid::from_string(rest).map_err(|_| invalid())?;
                // Normalize so lowercase input still matches stored IDs.
                Ok(Self(format!("{}{}", $prefix, ulid)))
            }
        }

        impl TryFrom<String> for $name {
            type Error = InvalidId;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                s.parse()
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

prefixed_id!(
    /// A project.
    ProjectId,
    "prj_"
);
prefixed_id!(
    /// A session.
    SessionId,
    "ses_"
);
prefixed_id!(
    /// An agent turn.
    TurnId,
    "trn_"
);
prefixed_id!(
    /// A worktree checkpoint.
    CheckpointId,
    "chk_"
);
prefixed_id!(
    /// A pending or decided approval.
    ApprovalId,
    "apr_"
);
prefixed_id!(
    /// A review thread.
    ThreadId,
    "thr_"
);
prefixed_id!(
    /// A comment in a review thread.
    CommentId,
    "cmt_"
);
prefixed_id!(
    /// A diff hunk.
    HunkId,
    "hnk_"
);
prefixed_id!(
    /// An event subscription on one connection.
    SubscriptionId,
    "sub_"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ids_have_prefix_and_round_trip() {
        let id = SessionId::new();
        assert!(id.as_str().starts_with("ses_"));
        assert_eq!(id.as_str().parse::<SessionId>().unwrap(), id);
    }

    #[test]
    fn wrong_prefix_is_rejected() {
        let prj = ProjectId::new();
        let err = prj.as_str().parse::<SessionId>().unwrap_err();
        assert_eq!(err.prefix, "ses_");
    }

    #[test]
    fn garbage_is_rejected() {
        assert!("prj_not-a-ulid".parse::<ProjectId>().is_err());
        assert!("".parse::<ProjectId>().is_err());
    }

    #[test]
    fn lowercase_ulid_is_normalized() {
        let id = ProjectId::new();
        let lower = format!("prj_{}", id.as_str()[4..].to_lowercase());
        assert_eq!(lower.parse::<ProjectId>().unwrap(), id);
    }

    #[test]
    fn serde_uses_plain_string_and_validates() {
        let id = ApprovalId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, format!("\"{id}\""));
        assert_eq!(serde_json::from_str::<ApprovalId>(&json).unwrap(), id);
        assert!(serde_json::from_str::<ApprovalId>("\"ses_01J00000000000000000000000\"").is_err());
    }
}
