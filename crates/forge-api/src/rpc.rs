//! JSON-RPC 2.0 envelopes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{ApiError, ErrorData};

pub const JSONRPC_VERSION: &str = "2.0";

/// Standard JSON-RPC codes for protocol-level failures. Application errors use `ErrorKind::code`.
pub mod codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INTERNAL_ERROR: i32 = -32603;
}

/// A request from a client. `id` may be a number or a string, as JSON-RPC allows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<ErrorData>,
}

impl RpcError {
    pub fn protocol(code: i32, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            data: None,
        }
    }
}

impl From<ApiError> for RpcError {
    fn from(err: ApiError) -> Self {
        Self {
            code: err.kind().code(),
            message: err.to_string(),
            data: Some(err.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl Response {
    pub fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn err(id: Value, error: RpcError) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.into(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

/// A server-pushed message with no `id`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: String,
    pub method: String,
    pub params: Value,
}

/// Anything the daemon sends to a client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ServerMessage {
    Response(Response),
    Notification(Notification),
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_errors_carry_code_and_data() {
        let err: RpcError = ApiError::not_found("project prj_x").into();
        assert_eq!(err.code, -32001);
        let value = serde_json::to_value(Response::err(json!(1), err)).unwrap();
        assert_eq!(value["error"]["data"]["kind"], "NotFound");
        assert_eq!(value["error"]["data"]["retryable"], false);
        assert!(value.get("result").is_none());
    }

    #[test]
    fn server_messages_are_distinguished_by_shape() {
        let resp: ServerMessage =
            serde_json::from_value(json!({ "jsonrpc": "2.0", "id": 1, "result": {} })).unwrap();
        assert!(matches!(resp, ServerMessage::Response(_)));
        let note: ServerMessage =
            serde_json::from_value(json!({ "jsonrpc": "2.0", "method": "event", "params": {} }))
                .unwrap();
        assert!(matches!(note, ServerMessage::Notification(_)));
    }
}
