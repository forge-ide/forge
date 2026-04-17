use anyhow::bail;
use bytes::Bytes;
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Subscribe {
    pub since: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct IpcEvent {
    pub seq: u64,
    pub event: serde_json::Value,
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
