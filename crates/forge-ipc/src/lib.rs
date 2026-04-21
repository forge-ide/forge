use anyhow::bail;
use bytes::Bytes;
pub use forge_core::RerunVariant;
// F-155: the MCP state + response shapes flow verbatim over UDS, so
// re-export here alongside `RerunVariant` for callers that prefer a single
// IPC import path.
pub use forge_core::{McpStateEvent, ServerState};
pub use forge_mcp::McpServerInfo;
use futures::{SinkExt, StreamExt};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio_util::codec::{Framed, LengthDelimitedCodec};

pub const PROTO_VERSION: u32 = 1;
pub const SCHEMA_VERSION: u32 = 1;
const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum IpcMessage {
    Hello(Hello),
    HelloAck(HelloAck),
    Subscribe(Subscribe),
    Event(IpcEvent),
    SendUserMessage(SendUserMessage),
    ToolCallApproved(ToolCallApproved),
    ToolCallRejected(ToolCallRejected),
    /// F-143: client → session request to re-run an assistant message.
    RerunMessage(RerunMessage),
    /// F-144: client → session request to activate a specific branch variant.
    SelectBranch(SelectBranch),
    /// F-145: client → session request to tombstone a branch variant.
    DeleteBranch(DeleteBranch),
    /// F-155: client → session request for the daemon's MCP server list.
    /// Response arrives as [`IpcMessage::McpServersList`].
    ListMcpServers(ListMcpServers),
    /// F-155: daemon → client response carrying the snapshot the daemon's
    /// authoritative `McpManager::list` returned.
    McpServersList(McpServersList),
    /// F-155: client → session request to toggle an MCP server on/off. The
    /// daemon acts on its single authoritative `McpManager` so a running
    /// session's tool dispatch is affected. Response arrives as
    /// [`IpcMessage::McpToggleResult`] — `error` is `Some` when the name
    /// is unknown or the lifecycle transition failed.
    ToggleMcpServer(ToggleMcpServer),
    /// F-155: daemon → client response for a [`IpcMessage::ToggleMcpServer`].
    McpToggleResult(McpToggleResult),
    /// F-155: client → session request to import a third-party MCP config
    /// into the workspace `.mcp.json`. `apply=false` runs a dry import —
    /// the daemon computes the new server set and returns it without
    /// rewriting the file. Response arrives as
    /// [`IpcMessage::McpImportResult`].
    ImportMcpConfig(ImportMcpConfig),
    /// F-155: daemon → client response for a [`IpcMessage::ImportMcpConfig`].
    McpImportResult(McpImportResult),
}

/// Client → session: re-run the assistant message with `msg_id` using the
/// given `variant`. All three variants (`Replace`, `Branch`, `Fresh`) are
/// wired as of F-144.
#[derive(Debug, Serialize, Deserialize)]
pub struct RerunMessage {
    /// Target assistant message to re-run. Wire shape is the canonical
    /// `MessageId` string to stay symmetric with `ToolCallApproved.id`.
    pub msg_id: String,
    pub variant: RerunVariant,
}

/// Client → session: activate a specific branch variant for replay / UI.
///
/// `parent` is the branch-point message id (the root variant's own id). The
/// daemon resolves `variant_index` against the event log: `0` refers to the
/// root itself; `N >= 1` refers to the `AssistantMessage` with
/// `branch_parent == Some(parent)` and `branch_variant_index == N`. On a
/// successful resolve, the daemon emits `Event::BranchSelected { parent,
/// selected }` where `selected` is the resolved MessageId.
#[derive(Debug, Serialize, Deserialize)]
pub struct SelectBranch {
    pub parent_id: String,
    pub variant_index: u32,
}

/// Client → session: tombstone a specific branch variant (F-145).
///
/// `parent_id` is the branch-point message id; `variant_index` identifies the
/// sibling to delete (0 = the root variant, N >= 1 = the Nth branch sibling).
/// The daemon resolves the target against the event log and emits
/// `Event::BranchDeleted { parent, variant_index }`. Deleting `variant_index
/// == 0` is **not** rejected here — the orchestrator may decide policy
/// (e.g. refuse when it would orphan every sibling).
#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteBranch {
    pub parent_id: String,
    pub variant_index: u32,
}

/// Client → session: send a user message to start a new turn.
#[derive(Debug, Serialize, Deserialize)]
pub struct SendUserMessage {
    pub text: String,
}

/// F-155: client → session: list the daemon's managed MCP servers.
///
/// No fields today — the daemon reports its own merged view built from
/// `<workspace>/.mcp.json` + `~/.mcp.json` at session start. A future
/// revision may add a selector (e.g. filter by server name) but the
/// frontend doesn't need one.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct ListMcpServers {}

/// F-155: daemon → client response carrying the snapshot returned by
/// `McpManager::list()`.
#[derive(Debug, Serialize, Deserialize)]
pub struct McpServersList {
    pub servers: Vec<McpServerInfo>,
}

/// F-155: client → session request to toggle an MCP server on or off.
///
/// `enabled` is the *target* state: `true` starts the server if it is not
/// already running; `false` disables it (parks in `ServerState::Disabled`
/// so the canonical "server disabled" error surfaces for in-flight /
/// subsequent tool calls).
#[derive(Debug, Serialize, Deserialize)]
pub struct ToggleMcpServer {
    pub name: String,
    pub enabled: bool,
}

/// F-155: daemon → client response for a `ToggleMcpServer`. `error` is
/// `None` on success; when `Some`, the toggle was rejected (unknown
/// server, lifecycle transition failed) and `enabled_after` reports the
/// *pre-toggle* state so the UI can reconcile without round-tripping
/// `ListMcpServers`.
#[derive(Debug, Serialize, Deserialize)]
pub struct McpToggleResult {
    pub name: String,
    pub enabled_after: bool,
    pub error: Option<String>,
}

/// F-155: client → session request to import a third-party MCP config
/// into the workspace's universal `.mcp.json`. `source` is the slug
/// accepted by `forge_mcp::import::ImportSource::from_slug`. When `apply`
/// is `false` the daemon runs a dry import — it returns the set of
/// names that *would* be imported and leaves the on-disk config
/// untouched; `true` rewrites the workspace file and rebuilds the
/// manager.
#[derive(Debug, Serialize, Deserialize)]
pub struct ImportMcpConfig {
    pub source: String,
    pub apply: bool,
}

/// F-155: daemon → client response for an `ImportMcpConfig`.
///
/// On success `imported` lists the server names that were applied (or
/// would be applied under `apply=false`). `destination_path` is the
/// absolute path of the rewritten workspace `.mcp.json`; empty when the
/// import was a dry-run. `error` is `Some` when the import failed (bad
/// slug, source file unreadable, write failed, etc.).
#[derive(Debug, Serialize, Deserialize)]
pub struct McpImportResult {
    pub source: String,
    pub imported: Vec<String>,
    pub destination_path: String,
    pub error: Option<String>,
}

/// Client → session: approve a pending tool call.
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallApproved {
    /// The ToolCallId to approve.
    pub id: String,
    /// Approval scope: "Once" | "ThisFile" | "ThisPattern" | "ThisTool".
    pub scope: String,
}

/// Client → session: reject a pending tool call.
#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCallRejected {
    pub id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Subscribe {
    pub since: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IpcEvent {
    pub seq: u64,
    // F-112: typed `Event` (not `serde_json::Value`).
    //
    // Previously the emission path was `Event -> serde_json::Value -> bytes`,
    // which forced serde to walk the dynamic tagged-union `Value` tree once
    // to construct it and a second time to write it out. Carrying the typed
    // `Event` directly collapses the pipeline to a single static traversal:
    // `Event -> bytes`. The wire shape is identical (serde flattens nested
    // structs the same way regardless of intermediate `Value`), so IPC
    // peers and the TS adapter see no change.
    pub event: forge_core::Event,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    pub client: ClientInfo,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClientInfo {
    pub kind: String,
    pub pid: u32,
    pub user: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HelloAck {
    pub session_id: String,
    pub workspace: String,
    pub started_at: String,
    pub event_seq: u64,
    pub schema_version: u32,
}

pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    msg: &IpcMessage,
) -> anyhow::Result<()> {
    let body = serde_json::to_vec(msg)?;
    if body.len() > MAX_FRAME_SIZE {
        bail!("frame too large: {} bytes", body.len());
    }
    writer.write_u32(body.len() as u32).await?;
    writer.write_all(&body).await?;
    Ok(())
}

pub async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> anyhow::Result<IpcMessage> {
    let len = reader.read_u32().await? as usize;
    if len > MAX_FRAME_SIZE {
        bail!("frame too large: {} bytes", len);
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf).await?;
    let msg = serde_json::from_slice(&buf)?;
    Ok(msg)
}

pub struct FramedStream {
    inner: Framed<UnixStream, LengthDelimitedCodec>,
}

impl FramedStream {
    pub fn new(stream: UnixStream) -> Self {
        let codec = LengthDelimitedCodec::builder()
            .max_frame_length(MAX_FRAME_SIZE)
            .new_codec();
        Self {
            inner: Framed::new(stream, codec),
        }
    }

    pub async fn send<T: Serialize>(&mut self, msg: &T) -> anyhow::Result<()> {
        let bytes = Bytes::from(serde_json::to_vec(msg)?);
        self.inner.send(bytes).await?;
        Ok(())
    }

    pub async fn recv<T: DeserializeOwned>(&mut self) -> anyhow::Result<Option<T>> {
        match self.inner.next().await {
            Some(Ok(bytes)) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Some(Err(e)) => Err(e.into()),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hello_msg() -> IpcMessage {
        IpcMessage::Hello(Hello {
            proto: PROTO_VERSION,
            client: ClientInfo {
                kind: "test".to_string(),
                pid: 1234,
                user: "alice".to_string(),
            },
        })
    }

    fn hello_ack_msg() -> IpcMessage {
        IpcMessage::HelloAck(HelloAck {
            session_id: "sess-abc".to_string(),
            workspace: "/tmp/ws".to_string(),
            started_at: "2024-01-01T00:00:00Z".to_string(),
            event_seq: 7,
            schema_version: SCHEMA_VERSION,
        })
    }

    #[tokio::test]
    async fn framed_stream_round_trips_hello() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut sender = FramedStream::new(a);
        let mut receiver = FramedStream::new(b);

        let sent = hello_msg();
        sender.send(&sent).await.unwrap();
        let got: IpcMessage = receiver.recv().await.unwrap().unwrap();

        let sent_json = serde_json::to_string(&sent).unwrap();
        let got_json = serde_json::to_string(&got).unwrap();
        assert_eq!(sent_json, got_json);
    }

    #[tokio::test]
    async fn framed_stream_round_trips_hello_ack() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut sender = FramedStream::new(a);
        let mut receiver = FramedStream::new(b);

        let sent = hello_ack_msg();
        sender.send(&sent).await.unwrap();
        let got: IpcMessage = receiver.recv().await.unwrap().unwrap();

        let sent_json = serde_json::to_string(&sent).unwrap();
        let got_json = serde_json::to_string(&got).unwrap();
        assert_eq!(sent_json, got_json);
    }
}
