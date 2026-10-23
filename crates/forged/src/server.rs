//! The Unix socket listener: JSON-RPC over WebSocket, one task per connection.

use std::future::Future;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, bail};
use futures_util::{SinkExt, StreamExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::rpc::{Conn, State, handle_text};

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

/// Accept connections until `shutdown` resolves.
pub async fn serve(
    listener: UnixListener,
    socket: &Path,
    state: Arc<State>,
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
                    tokio::spawn(connection(stream, state.clone()));
                }
                Err(e) => tracing::warn!(error = %e, "accept failed"),
            },
        }
    }
}

async fn connection(stream: UnixStream, state: Arc<State>) {
    let ws = match tokio_tungstenite::accept_async(stream).await {
        Ok(ws) => ws,
        Err(e) => {
            tracing::debug!(error = %e, "websocket handshake failed");
            return;
        }
    };
    tracing::debug!("client connected");
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let conn = Arc::new(Conn::new(tx.clone()));

    tokio::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    while let Some(msg) = source.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let (state, conn, tx) = (state.clone(), conn.clone(), tx.clone());
                tokio::spawn(async move {
                    let response = handle_text(&state, &conn, &text).await;
                    if let Ok(text) = serde_json::to_string(&response) {
                        let _ = tx.send(text);
                    }
                });
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Binary(_)) => tracing::debug!("ignoring binary frame"),
            Ok(_) => {}
            Err(e) => {
                tracing::debug!(error = %e, "connection error");
                break;
            }
        }
    }
    conn.close(&state);
    tracing::debug!("client disconnected");
}
