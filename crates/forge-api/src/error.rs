//! Application errors and their JSON-RPC codes.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The kind of an application error. Each maps to one JSON-RPC error code.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
pub enum ErrorKind {
    NotFound,
    InvalidState,
    PolicyDenied,
    AgentUnavailable,
    SandboxUnavailable,
    Conflict,
    Unauthorized,
    ValidationFailed,
}

impl ErrorKind {
    pub const ALL: [ErrorKind; 8] = [
        ErrorKind::NotFound,
        ErrorKind::InvalidState,
        ErrorKind::PolicyDenied,
        ErrorKind::AgentUnavailable,
        ErrorKind::SandboxUnavailable,
        ErrorKind::Conflict,
        ErrorKind::Unauthorized,
        ErrorKind::ValidationFailed,
    ];

    /// The JSON-RPC error code for this kind.
    pub fn code(self) -> i32 {
        match self {
            ErrorKind::NotFound => -32001,
            ErrorKind::InvalidState => -32002,
            ErrorKind::PolicyDenied => -32003,
            ErrorKind::AgentUnavailable => -32004,
            ErrorKind::SandboxUnavailable => -32005,
            ErrorKind::Conflict => -32006,
            ErrorKind::Unauthorized => -32007,
            ErrorKind::ValidationFailed => -32008,
        }
    }

    pub fn from_code(code: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.code() == code)
    }

    pub fn name(self) -> &'static str {
        match self {
            ErrorKind::NotFound => "NotFound",
            ErrorKind::InvalidState => "InvalidState",
            ErrorKind::PolicyDenied => "PolicyDenied",
            ErrorKind::AgentUnavailable => "AgentUnavailable",
            ErrorKind::SandboxUnavailable => "SandboxUnavailable",
            ErrorKind::Conflict => "Conflict",
            ErrorKind::Unauthorized => "Unauthorized",
            ErrorKind::ValidationFailed => "ValidationFailed",
        }
    }
}

/// The structured `data` field of an application error.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ErrorData {
    pub kind: ErrorKind,
    pub detail: String,
    pub retryable: bool,
}

/// An application error as returned by a daemon method.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{}: {}", .0.kind.name(), .0.detail)]
pub struct ApiError(pub ErrorData);

impl ApiError {
    pub fn new(kind: ErrorKind, detail: impl Into<String>) -> Self {
        Self(ErrorData {
            kind,
            detail: detail.into(),
            retryable: false,
        })
    }

    pub fn retryable(mut self) -> Self {
        self.0.retryable = true;
        self
    }

    pub fn not_found(detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, detail)
    }

    pub fn validation(detail: impl Into<String>) -> Self {
        Self::new(ErrorKind::ValidationFailed, detail)
    }

    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    pub fn detail(&self) -> &str {
        &self.0.detail
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_match_spec_and_round_trip() {
        assert_eq!(ErrorKind::NotFound.code(), -32001);
        assert_eq!(ErrorKind::ValidationFailed.code(), -32008);
        for kind in ErrorKind::ALL {
            assert_eq!(ErrorKind::from_code(kind.code()), Some(kind));
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{}\"", kind.name()));
        }
        assert_eq!(ErrorKind::from_code(-32601), None);
    }
}
