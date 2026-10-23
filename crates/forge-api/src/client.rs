//! A minimal async client over the daemon's Unix socket, for the CLI and integration tests.
//!
//! Calls are sequential: `call` sends one request and reads until its response arrives,
//! queueing any event notifications that arrive meanwhile for `next_event`.

use std::collections::VecDeque;
use std::path::Path;

use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::UnixStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::error::ApiError;
use crate::events::{EVENT_NOTIFICATION, EventNotification};
use crate::methods::Method;
use crate::rpc::{JSONRPC_VERSION, Request, RpcError, ServerMessage};

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("connecting to {path}: {source}")]
    Connect {
        path: String,
        source: std::io::Error,
    },
    #[error("websocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("daemon closed the connection")]
    Closed,
    #[error("protocol: {0}")]
    Protocol(String),
    #[error(transparent)]
    Api(#[from] ApiError),
    #[error("rpc error {code}: {message}")]
    Rpc { code: i32, message: String },
}

impl From<RpcError> for ClientError {
    fn from(err: RpcError) -> Self {
        match err.data {
            Some(data) => ClientError::Api(ApiError(data)),
            None => ClientError::Rpc {
                code: err.code,
                message: err.message,
            },
        }
    }
}

pub struct Client {
    ws: WebSocketStream<UnixStream>,
    next_id: u64,
    events: VecDeque<EventNotification>,
}

impl Client {
    pub async fn connect(socket: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket)
            .await
            .map_err(|source| ClientError::Connect {
                path: socket.display().to_string(),
                source,
            })?;
        // The host is ignored on a Unix socket, but the handshake needs a URL.
        let (ws, _) = tokio_tungstenite::client_async("ws://localhost/", stream).await?;
        Ok(Self {
            ws,
            next_id: 1,
            events: VecDeque::new(),
        })
    }

    pub async fn call<M: Method>(&mut self, params: &M::Params) -> Result<M::Result, ClientError> {
        let params =
            serde_json::to_value(params).map_err(|e| ClientError::Protocol(e.to_string()))?;
        let result = self.call_raw(M::NAME, params).await?;
        serde_json::from_value(result)
            .map_err(|e| ClientError::Protocol(format!("{} result: {e}", M::NAME)))
    }

    /// Sends any method by name. Useful for testing error paths with malformed params.
    pub async fn call_raw(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = Request {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Value::from(id),
            method: method.into(),
            params,
        };
        let text =
            serde_json::to_string(&request).map_err(|e| ClientError::Protocol(e.to_string()))?;
        self.ws.send(Message::text(text)).await?;
        loop {
            match self.read().await? {
                ServerMessage::Response(resp) if resp.id == id => {
                    if let Some(err) = resp.error {
                        return Err(err.into());
                    }
                    return Ok(resp.result.unwrap_or(Value::Null));
                }
                ServerMessage::Response(resp) => {
                    return Err(ClientError::Protocol(format!(
                        "unexpected response id {}",
                        resp.id
                    )));
                }
                ServerMessage::Notification(note) => self.queue(note.method, note.params)?,
            }
        }
    }

    /// The next event notification, waiting for one if none is queued.
    pub async fn next_event(&mut self) -> Result<EventNotification, ClientError> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Ok(event);
            }
            match self.read().await? {
                ServerMessage::Notification(note) => self.queue(note.method, note.params)?,
                ServerMessage::Response(resp) => {
                    return Err(ClientError::Protocol(format!(
                        "unexpected response id {}",
                        resp.id
                    )));
                }
            }
        }
    }

    pub async fn close(mut self) -> Result<(), ClientError> {
        self.ws.close(None).await?;
        Ok(())
    }

    fn queue(&mut self, method: String, params: Value) -> Result<(), ClientError> {
        if method != EVENT_NOTIFICATION {
            return Err(ClientError::Protocol(format!(
                "unknown notification {method}"
            )));
        }
        let event = serde_json::from_value(params)
            .map_err(|e| ClientError::Protocol(format!("event: {e}")))?;
        self.events.push_back(event);
        Ok(())
    }

    async fn read(&mut self) -> Result<ServerMessage, ClientError> {
        loop {
            let msg = self.ws.next().await.ok_or(ClientError::Closed)??;
            match msg {
                Message::Text(text) => {
                    return serde_json::from_str(&text)
                        .map_err(|e| ClientError::Protocol(e.to_string()));
                }
                Message::Close(_) => return Err(ClientError::Closed),
                // Pings are answered by tungstenite; binary frames aren't part of the protocol.
                _ => continue,
            }
        }
    }
}
