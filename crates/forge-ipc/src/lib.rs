use anyhow::bail;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const PROTO_VERSION: u32 = 1;
pub const SCHEMA_VERSION: u32 = 1;
const MAX_FRAME_SIZE: usize = 4 * 1024 * 1024;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t")]
pub enum IpcMessage {
    Hello(Hello),
    HelloAck(HelloAck),
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
