//! Tauri ↔ session UDS bridge.
//!
//! The Tauri commands in [`crate::ipc`] are thin wrappers over this module.
//! This split keeps the bridge logic testable without a live Tauri runtime:
//! tests spawn a real `forge-session` daemon, drive the bridge via
//! [`SessionBridge`], and capture forwarded events through a generic
//! [`EventSink`].
//!
//! **Locking discipline (F-109):** no function in this file may hold the
//! `SessionConnections` map lock (`inner`) across an `.await` point. The
//! map lock serializes every Tauri command's lookup; holding it across a
//! UDS write stalls concurrent commands on unrelated sessions. The pattern
//! is always: acquire → capture the per-connection handle (writer `Arc`,
//! reader `Option`) → drop → await → re-acquire briefly to install state.
//! The `clippy::await_holding_lock` deny below is structural documentation:
//! it catches `std::sync::Mutex` regressions even though it does not cover
//! `tokio::sync::Mutex` (tokio's guards are deliberately excluded from that
//! lint). Reviewers and the accompanying concurrency regression test are
//! the enforcement for the tokio case.
#![deny(clippy::await_holding_lock)]

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
/// rules as `forge-session::socket_path::resolve_socket_path`. Exposed so
/// the Tauri command layer and tests agree on the convention.
///
/// Returns `Err` when `XDG_RUNTIME_DIR` is unset. F-044 (H8) closed the
/// pre-existing `/tmp/forge-<uid>` fallback; `forged` itself refuses to
/// start without `XDG_RUNTIME_DIR`, so a shell that silently resolved to
/// `/tmp/...` would only ever hit `ENOENT`. Surfacing the missing env var
/// here gives the operator a clearer error than "no such file".
pub fn default_socket_path(session_id: &str) -> Result<PathBuf> {
    let base = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            anyhow!(
                "XDG_RUNTIME_DIR is unset: forge-shell refuses to fall \
                 back to /tmp because the socket there would be \
                 world-connectable. Set XDG_RUNTIME_DIR to a per-user \
                 0o700 directory (systemd sets /run/user/<uid> \
                 automatically). (F-044 / H8)"
            )
        })?;
    Ok(base
        .join("forge/sessions")
        .join(format!("{session_id}.sock")))
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

        let path: PathBuf = match socket_path {
            Some(p) => p.to_path_buf(),
            None => default_socket_path(session_id)?,
        };
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
    /// connection: a second call is a no-op (task already running or in
    /// flight).
    ///
    /// **F-109:** this function must not hold the `SessionConnections` map
    /// lock across any `.await`. The sequence is:
    ///
    /// 1. Lock the map, validate, clone the writer `Arc`, take the reader
    ///    half (which also acts as a "subscription-in-flight" reservation),
    ///    drop the lock.
    /// 2. Await `write_frame` on the per-connection writer mutex.
    /// 3. Re-acquire the map lock briefly to install the reader task, or —
    ///    if the write failed — restore the reader so a retry is possible.
    ///
    /// A concurrent second call that lands between step 1 and step 3 sees
    /// `reader: None` and treats the subscription as already in flight; it
    /// returns `Ok(())` rather than failing with "reader already consumed".
    pub async fn subscribe(
        &self,
        session_id: &str,
        since: u64,
        sink: Arc<dyn EventSink>,
    ) -> Result<()> {
        // Step 1: capture per-connection handles under the map lock, then
        // drop the lock before any `.await`.
        let (writer, reader) = {
            let mut map = self.connections.inner.lock().await;
            let conn = map.get_mut(session_id).ok_or_else(|| {
                anyhow!("session_subscribe: no active connection for {session_id}")
            })?;

            // Idempotent in two states: reader_task already spawned, or an
            // in-flight subscribe has reserved the reader by taking it.
            if conn.reader_task.is_some() || conn.reader.is_none() {
                return Ok(());
            }

            let writer = Arc::clone(&conn.writer);
            // Reserve the reader under the map lock so a racing call sees
            // `reader: None` and bails early rather than duplicating the
            // subscribe frame or fighting for the reader half.
            let reader = conn.reader.take().expect("reader present per check above");
            (writer, reader)
        };

        // Step 2: await the Subscribe frame with the map lock released.
        let sub = IpcMessage::Subscribe(Subscribe { since });
        let write_result = {
            let mut writer_guard = writer.lock().await;
            write_frame(&mut *writer_guard, &sub).await
        };

        // Step 3: re-acquire the map lock briefly to either install the
        // reader task or restore the reader on failure.
        let mut map = self.connections.inner.lock().await;
        let conn = map.get_mut(session_id).ok_or_else(|| {
            anyhow!("session_subscribe: connection for {session_id} disappeared mid-subscribe")
        })?;

        if let Err(err) = write_result {
            // Put the reader back so a subsequent subscribe can try again.
            conn.reader = Some(reader);
            return Err(err);
        }

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

    /// Test-only accessor for a session's writer mutex. Used by concurrency
    /// tests (F-109) to externally hold the writer so a `subscribe()` frame
    /// write stalls deterministically, then verify that unrelated sessions'
    /// commands are not blocked behind the `SessionConnections` map lock.
    ///
    /// Not used in production paths.
    #[doc(hidden)]
    pub async fn writer_arc_for_testing(
        &self,
        session_id: &str,
    ) -> Option<Arc<Mutex<OwnedWriteHalf>>> {
        let map = self.connections.inner.lock().await;
        map.get(session_id).map(|c| Arc::clone(&c.writer))
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
