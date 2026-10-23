//! The Unix socket listener: JSON-RPC over WebSocket, one task per connection.

use std::future::Future;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, mpsc};
use tokio_tungstenite::tungstenite::Message;

use crate::rpc::{Conn, Outbox, State, handle_text};

/// Bind the socket at `path`: private directory, socket 0600, replacing a stale socket from a crash.
pub fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    let dir = path
        .parent()
        .context("socket path has no parent directory")?;
    crate::dirs::ensure_private(dir)?;

    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_socket() => {
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                bail!("forged is already running (socket {})", path.display());
            }
            std::fs::remove_file(path)
                .with_context(|| format!("removing stale socket {}", path.display()))?;
        }
        Ok(_) => bail!(
            "{} exists and is not a socket; move it away and retry",
            path.display()
        ),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).with_context(|| format!("checking {}", path.display())),
    }

    let listener =
        UnixListener::bind(path).with_context(|| format!("binding {}", path.display()))?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("restricting {}", path.display()))?;
    Ok(listener)
}

/// Per-connection limits, so one misbehaving client can't exhaust the daemon.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Messages queued for a client; a client that lets this fill is disconnected.
    pub queue: usize,
    /// Time allowed to complete the WebSocket handshake.
    pub handshake: Duration,
    /// Requests handled at once; beyond this the daemon stops reading until one finishes.
    pub in_flight: usize,
    /// How long a replay waits for a slow client to make room in its queue.
    pub replay_send: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            queue: 1024,
            handshake: Duration::from_secs(10),
            in_flight: 64,
            replay_send: Duration::from_secs(30),
        }
    }
}

/// Accept connections until `shutdown` resolves.
pub async fn serve(
    listener: UnixListener,
    socket: &Path,
    state: Arc<State>,
    limits: Limits,
    shutdown: impl Future<Output = ()>,
) {
    // Only the user who owns the socket may connect; file mode already enforces this, and the
    // peer check backs it up if the mode is ever loosened.
    let owner = std::fs::metadata(socket).map(|m| m.uid()).ok();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            () = &mut shutdown => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let peer = stream.peer_cred().map(|c| c.uid()).ok();
                    if owner.is_none() || peer != owner {
                        tracing::warn!(?peer, ?owner, "rejected connection from another user");
                        continue;
                    }
                    tokio::spawn(connection(stream, state.clone(), limits));
                }
                Err(e) => tracing::warn!(error = %e, "accept failed"),
            },
        }
    }
}

async fn connection(stream: UnixStream, state: Arc<State>, limits: Limits) {
    let ws = match tokio::time::timeout(limits.handshake, tokio_tungstenite::accept_async(stream))
        .await
    {
        Ok(Ok(ws)) => ws,
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "websocket handshake failed");
            return;
        }
        Err(_) => {
            tracing::debug!("websocket handshake timed out");
            return;
        }
    };
    tracing::debug!("client connected");
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::channel::<String>(limits.queue);
    let outbox = Outbox::new(tx, limits.replay_send);
    let dropped = outbox.drop.clone();
    let conn = Arc::new(Conn::new(outbox));

    let writer_dropped = dropped.clone();
    let writer = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = writer_dropped.cancelled() => break,
                text = rx.recv() => match text {
                    Some(text) => {
                        if sink.send(Message::text(text)).await.is_err() {
                            break;
                        }
                    }
                    None => break,
                },
            }
        }
        let _ = sink.close().await;
    });

    let in_flight = Arc::new(Semaphore::new(limits.in_flight));
    loop {
        let msg = tokio::select! {
            () = dropped.cancelled() => break,
            msg = source.next() => msg,
        };
        match msg {
            Some(Ok(Message::Text(text))) => {
                // At the cap, stop reading until a request finishes.
                let permit = tokio::select! {
                    () = dropped.cancelled() => break,
                    permit = in_flight.clone().acquire_owned() => match permit {
                        Ok(permit) => permit,
                        Err(_) => break,
                    },
                };
                let (state, conn) = (state.clone(), conn.clone());
                tokio::spawn(async move {
                    let (response, deferred) = handle_text(&state, &conn, &text).await;
                    let sent =
                        serde_json::to_string(&response).is_ok_and(|text| conn.outbox.send(text));
                    drop(permit);
                    if sent && let Some(deferred) = deferred {
                        deferred.await;
                    }
                });
            }
            Some(Ok(Message::Close(_))) | None => break,
            Some(Ok(Message::Binary(_))) => tracing::debug!("ignoring binary frame"),
            Some(Ok(_)) => {}
            Some(Err(e)) => {
                tracing::debug!(error = %e, "connection error");
                break;
            }
        }
    }
    conn.close(&state);
    dropped.cancel();
    let _ = writer.await;
    tracing::debug!("client disconnected");
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use futures_util::{SinkExt, StreamExt};
    use tokio::io::AsyncReadExt;

    use super::*;
    use crate::rpc::Outbox;

    struct Running {
        _dir: tempfile::TempDir,
        socket: PathBuf,
        state: Arc<State>,
    }

    async fn start(limits: Limits) -> Running {
        let dir = tempfile::tempdir().unwrap();
        let paths = forge_config::Paths {
            data_dir: dir.path().join("data"),
            runtime_dir: dir.path().join("run"),
            config_dir: dir.path().join("config"),
        };
        crate::dirs::ensure_private(&paths.data_dir).unwrap();
        let socket = paths.socket();
        let listener = bind(&socket).unwrap();
        let db = crate::db::Db::open(&paths.database()).unwrap();
        let state = crate::open_state(db, paths).await.unwrap();
        let (served, path) = (state.clone(), socket.clone());
        tokio::spawn(async move {
            serve(listener, &path, served, limits, std::future::pending()).await;
        });
        Running {
            _dir: dir,
            socket,
            state,
        }
    }

    #[tokio::test]
    async fn a_client_that_never_finishes_the_handshake_is_dropped() {
        let limits = Limits {
            handshake: Duration::from_millis(100),
            ..Limits::default()
        };
        let running = start(limits).await;
        let mut stream = UnixStream::connect(&running.socket).await.unwrap();
        let mut buf = [0u8; 16];
        let read = tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf))
            .await
            .expect("the daemon closes the connection");
        assert_eq!(read.unwrap(), 0);
    }

    #[tokio::test]
    async fn a_client_that_stops_reading_is_dropped() {
        let limits = Limits {
            queue: 4,
            in_flight: 8,
            ..Limits::default()
        };
        let running = start(limits).await;
        let stream = UnixStream::connect(&running.socket).await.unwrap();
        let (ws, _) = tokio_tungstenite::client_async("ws://localhost/", stream)
            .await
            .unwrap();
        let (mut tx, mut rx) = ws.split();
        const SENT: usize = 20_000;
        // Send without reading any responses, until the daemon hangs up.
        let sender = tokio::spawn(async move {
            for id in 0..SENT {
                let text = format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"method":"server.hello","params":{{"client":"t","apiVersion":"0"}}}}"#
                );
                if tx.send(Message::text(text)).await.is_err() {
                    return;
                }
            }
        });
        let _ = tokio::time::timeout(Duration::from_secs(20), sender).await;
        let mut responses = 0;
        let drained = tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(Ok(msg)) = rx.next().await {
                if msg.is_text() {
                    responses += 1;
                }
            }
        })
        .await;
        assert!(drained.is_ok(), "the daemon closed the connection");
        assert!(responses < SENT, "got all {responses} responses");
    }

    #[tokio::test]
    async fn requests_beyond_the_in_flight_cap_wait_to_be_read() {
        let limits = Limits {
            in_flight: 1,
            ..Limits::default()
        };
        let running = start(limits).await;
        let mut client = forge_api::client::Client::connect(&running.socket)
            .await
            .unwrap();
        // With one request at a time, responses still all arrive.
        for _ in 0..20 {
            client
                .call::<forge_api::methods::ProjectList>(&forge_api::methods::Empty {})
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn a_subscribe_that_finishes_after_disconnect_is_removed() {
        let running = start(Limits::default()).await;
        let (tx, _rx) = mpsc::channel(4);
        let conn = Conn::new(Outbox::new(tx, Duration::from_secs(1)));
        conn.close(&running.state);
        let request =
            r#"{"jsonrpc":"2.0","id":1,"method":"events.subscribe","params":{"topics":["board"]}}"#;
        let (response, _) = handle_text(&running.state, &conn, request).await;
        assert!(response.error.is_none(), "{response:?}");
        assert!(running.state.log.hub().is_empty());
    }
}
