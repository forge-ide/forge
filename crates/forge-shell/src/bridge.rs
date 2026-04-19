//! Tauri ↔ session UDS bridge.
//!
//! The Tauri commands in [`crate::ipc`] are thin wrappers over this module.
//! This split keeps the bridge logic testable without a live Tauri runtime:
//! tests spawn a real `forge-session` daemon, drive the bridge via
//! [`SessionBridge`], and capture forwarded events through a generic
//! [`EventSink`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use forge_core::ApprovalScope;
use forge_ipc::{
    read_frame, write_frame, ClientInfo, Hello, HelloAck, IpcMessage, SendUserMessage, Subscribe,
    ToolCallApproved, ToolCallRejected, PROTO_VERSION,
};
use serde::Serialize;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::UnixStream;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

/// Payload emitted to the webview for every session event forwarded by the
/// background reader task. The `event` value is the daemon's `Event` JSON
/// pass-through (see `forge-core::Event`).
#[derive(Debug, Clone, Serialize)]
pub struct SessionEventPayload {
    pub session_id: String,
    pub seq: u64,
    pub event: serde_json::Value,
}

/// Sink for forwarded session events. In production the Tauri command layer
/// implements this by calling `AppHandle::emit("session:event", payload)`;
/// tests use an in-memory channel to assert end-to-end delivery.
pub trait EventSink: Send + Sync + 'static {
    fn emit(&self, payload: SessionEventPayload);
}

/// A single open session connection: a writer half used for send/approve/reject
/// plus an optional background reader task that pumps events into the sink.
struct Connection {
    writer: Arc<Mutex<OwnedWriteHalf>>,
    reader: Option<OwnedReadHalf>,
    reader_task: Option<JoinHandle<()>>,
}

/// Session-id keyed registry of active bridge connections.
#[derive(Clone, Default)]
pub struct SessionConnections {
    inner: Arc<Mutex<HashMap<String, Connection>>>,
}

impl SessionConnections {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn len(&self) -> usize {
        self.inner.lock().await.len()
    }

    pub async fn is_empty(&self) -> bool {
        self.inner.lock().await.is_empty()
    }

    pub async fn contains(&self, session_id: &str) -> bool {
        self.inner.lock().await.contains_key(session_id)
    }
}

/// Resolve the default socket path for a session id, following the same
/// rules as `forge-session::main`. Exposed so the Tauri command layer and
/// tests agree on the convention.
pub fn default_socket_path(session_id: &str) -> PathBuf {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let uid = std::env::var("UID").unwrap_or_else(|_| "0".to_string());
            PathBuf::from(format!("/tmp/forge-{uid}"))
        });
    base.join("forge/sessions")
        .join(format!("{session_id}.sock"))
}

/// Top-level bridge operations invoked by Tauri commands. Keyed on
/// [`SessionConnections`]; safe to clone and share across command handlers.
#[derive(Clone)]
pub struct SessionBridge {
    connections: SessionConnections,
}

impl SessionBridge {
    pub fn new(connections: SessionConnections) -> Self {
        Self { connections }
    }

    pub fn connections(&self) -> &SessionConnections {
        &self.connections
    }

    /// Open (or reuse) a UDS connection for `session_id` and perform the
    /// framed `Hello`/`HelloAck` handshake. Returns the daemon's ack.
    ///
    /// If `socket_path` is `None`, [`default_socket_path`] is used.
    pub async fn hello(&self, session_id: &str, socket_path: Option<&Path>) -> Result<HelloAck> {
        {
            let map = self.connections.inner.lock().await;
            if map.contains_key(session_id) {
                return Err(anyhow!(
                    "session_hello: already connected to session {session_id}"
                ));
            }
        }

        let path: PathBuf = socket_path
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_socket_path(session_id));
        let stream = UnixStream::connect(&path)
            .await
            .with_context(|| format!("connect UDS {}", path.display()))?;
        let (mut reader, mut writer) = stream.into_split();

        let hello = IpcMessage::Hello(Hello {
            proto: PROTO_VERSION,
            client: ClientInfo {
                kind: "shell".to_string(),
                pid: std::process::id(),
                user: std::env::var("USER").unwrap_or_else(|_| "forge".to_string()),
            },
        });
        write_frame(&mut writer, &hello).await?;

        let ack = read_frame(&mut reader)
            .await
            .context("read HelloAck frame")?;
        let IpcMessage::HelloAck(ack) = ack else {
            return Err(anyhow!("expected HelloAck, got unexpected frame"));
        };

        let conn = Connection {
            writer: Arc::new(Mutex::new(writer)),
            reader: Some(reader),
            reader_task: None,
        };
        self.connections
            .inner
            .lock()
            .await
            .insert(session_id.to_string(), conn);

        Ok(ack)
    }

    /// Send `Subscribe { since }` to the daemon and spawn a background task
    /// that reads event frames and delivers them to `sink`. Idempotent per
    /// connection: a second call is a no-op (task already running).
    pub async fn subscribe(
        &self,
        session_id: &str,
        since: u64,
        sink: Arc<dyn EventSink>,
    ) -> Result<()> {
        let mut map = self.connections.inner.lock().await;
        let conn = map
            .get_mut(session_id)
            .ok_or_else(|| anyhow!("session_subscribe: no active connection for {session_id}"))?;

        if conn.reader_task.is_some() {
            return Ok(());
        }

        let sub = IpcMessage::Subscribe(Subscribe { since });
        {
            let mut writer = conn.writer.lock().await;
            write_frame(&mut *writer, &sub).await?;
        }

        let reader = conn
            .reader
            .take()
            .ok_or_else(|| anyhow!("session_subscribe: reader already consumed"))?;
        let session_id_owned = session_id.to_string();
        let task = tokio::spawn(async move {
            pump_events(reader, session_id_owned, sink).await;
        });
        conn.reader_task = Some(task);

        Ok(())
    }

    pub async fn send_message(&self, session_id: &str, text: String) -> Result<()> {
        let writer = self.writer_for(session_id).await?;
        let mut writer = writer.lock().await;
        let frame = IpcMessage::SendUserMessage(SendUserMessage { text });
        write_frame(&mut *writer, &frame).await
    }

    pub async fn approve_tool(
        &self,
        session_id: &str,
        id: String,
        scope: ApprovalScope,
    ) -> Result<()> {
        let writer = self.writer_for(session_id).await?;
        let mut writer = writer.lock().await;
        // F-069 / L5 (T7): the Tauri command layer is the trust boundary — it
        // accepts a typed `ApprovalScope` only. The wire shape in
        // `forge_ipc::ToolCallApproved` still carries `scope: String` so the
        // forge-session daemon's deserializer keeps working unchanged while
        // F-053 (M7) — the complementary server-side fix — is still pending.
        let scope = scope_to_wire(&scope);
        let frame = IpcMessage::ToolCallApproved(ToolCallApproved { id, scope });
        write_frame(&mut *writer, &frame).await
    }

    pub async fn reject_tool(
        &self,
        session_id: &str,
        id: String,
        reason: Option<String>,
    ) -> Result<()> {
        let writer = self.writer_for(session_id).await?;
        let mut writer = writer.lock().await;
        let frame = IpcMessage::ToolCallRejected(ToolCallRejected { id, reason });
        write_frame(&mut *writer, &frame).await
    }

    async fn writer_for(&self, session_id: &str) -> Result<Arc<Mutex<OwnedWriteHalf>>> {
        let map = self.connections.inner.lock().await;
        let conn = map
            .get(session_id)
            .ok_or_else(|| anyhow!("no active connection for session {session_id}"))?;
        Ok(Arc::clone(&conn.writer))
    }
}

/// F-069 / L5 (T7): map a typed [`ApprovalScope`] to the short PascalCase
/// string that `forge_ipc::ToolCallApproved.scope` (and the existing daemon
/// parser via F-053) expects. Kept explicit — not derived through
/// `serde_json::to_value` — so the wire strings stay grep-able and any
/// future variant addition forces a compile error here.
fn scope_to_wire(scope: &ApprovalScope) -> String {
    match scope {
        ApprovalScope::Once => "Once",
        ApprovalScope::ThisFile => "ThisFile",
        ApprovalScope::ThisPattern => "ThisPattern",
        ApprovalScope::ThisTool => "ThisTool",
    }
    .to_string()
}

async fn pump_events(mut reader: OwnedReadHalf, session_id: String, sink: Arc<dyn EventSink>) {
    loop {
        match read_frame(&mut reader).await {
            Ok(IpcMessage::Event(event)) => {
                sink.emit(SessionEventPayload {
                    session_id: session_id.clone(),
                    seq: event.seq,
                    event: event.event,
                });
            }
            Ok(_) => {
                // Non-event frames (e.g. late HelloAck) are ignored; only
                // session events flow to the webview.
            }
            Err(_) => {
                // Peer closed or unrecoverable framing error. Task exits.
                break;
            }
        }
    }
}
