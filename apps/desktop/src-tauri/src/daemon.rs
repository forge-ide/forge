//! Bridges the webview to the daemon socket so the webview never opens a network port.
//!
//! This is a pipe, not a client: it forwards JSON-RPC text frames both ways and starts
//! `forged` if nothing is listening. The UI owns the protocol.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use forge_config::Paths;
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::net::UnixStream;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::tungstenite::Message;

/// Event carrying one text frame from the daemon.
const MESSAGE_EVENT: &str = "forge://daemon-message";
/// Event emitted once when the connection ends, with the reason.
const CLOSED_EVENT: &str = "forge://daemon-closed";

/// How long to wait for a freshly started daemon to accept connections.
const START_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct Bridge {
    current: Mutex<Slot<Connection>>,
    /// Serializes `daemon_connect` so two calls can't both start a daemon.
    connecting: Mutex<()>,
}

/// One open daemon connection. The writer task ends, closing the socket, once `outgoing` is
/// dropped; the reader holds no sender, so it can't keep the socket alive.
struct Connection {
    outgoing: mpsc::UnboundedSender<String>,
    reader: tauri::async_runtime::JoinHandle<()>,
}

impl Connection {
    /// Stop forwarding frames from this connection and let its socket close.
    fn close(self) {
        self.reader.abort();
    }
}

/// Holds the current value with an ID, so a task can tell whether it still owns the slot.
struct Slot<T> {
    next_id: u64,
    current: Option<(u64, T)>,
}

impl<T> Default for Slot<T> {
    fn default() -> Self {
        Self {
            next_id: 1,
            current: None,
        }
    }
}

impl<T> Slot<T> {
    /// Reserve the ID the next `put` will use.
    fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Store `value` under the reserved ID, returning the value it replaces.
    fn put(&mut self, value: T) -> Option<T> {
        let id = self.next_id;
        self.next_id += 1;
        self.current.replace((id, value)).map(|(_, old)| old)
    }

    fn get(&self) -> Option<&T> {
        self.current.as_ref().map(|(_, v)| v)
    }

    /// Remove the value if it is still the one stored under `id`.
    fn take_if(&mut self, id: u64) -> Option<T> {
        match self.current {
            Some((current, _)) if current == id => self.current.take().map(|(_, v)| v),
            _ => None,
        }
    }

    fn take(&mut self) -> Option<T> {
        self.current.take().map(|(_, v)| v)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonInfo {
    socket: String,
    log: String,
}

fn paths() -> Result<Paths, String> {
    Paths::from_env().map_err(|e| e.to_string())
}

/// Where the daemon listens and logs, for error messages.
#[tauri::command]
pub fn daemon_info() -> Result<DaemonInfo, String> {
    let paths = paths()?;
    Ok(DaemonInfo {
        socket: paths.socket().display().to_string(),
        log: paths.daemon_log().display().to_string(),
    })
}

/// Connect to the daemon, starting it first if nothing is listening. Replaces any open connection.
#[tauri::command]
pub async fn daemon_connect(app: AppHandle, bridge: State<'_, Bridge>) -> Result<(), String> {
    let _connecting = bridge.connecting.lock().await;
    // Close the old connection first so its frames can't reach the new one.
    if let Some(old) = bridge.current.lock().await.take() {
        old.close();
    }

    let paths = paths()?;
    let socket = paths.socket();
    let stream = match UnixStream::connect(&socket).await {
        Ok(stream) => stream,
        Err(_) => {
            start_daemon(&paths)?;
            wait_for_socket(&socket).await?
        }
    };
    let (ws, _) = tokio_tungstenite::client_async("ws://localhost/", stream)
        .await
        .map_err(|e| format!("handshake with forged failed: {e}"))?;
    let (mut sink, mut source) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    tauri::async_runtime::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    let mut slot = bridge.current.lock().await;
    let id = slot.next_id();
    let reader = tauri::async_runtime::spawn(async move {
        let reason = loop {
            match source.next().await {
                Some(Ok(Message::Text(text))) => {
                    let _ = app.emit(MESSAGE_EVENT, text.as_str());
                }
                Some(Ok(Message::Close(_))) | None => {
                    break "daemon closed the connection".to_owned();
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => break e.to_string(),
            }
        };
        // Only report the close if this is still the current connection.
        let bridge = app.state::<Bridge>();
        let ended = bridge.current.lock().await.take_if(id);
        // Dropping the connection ends its writer; this task is finishing on its own.
        if ended.is_some() {
            let _ = app.emit(CLOSED_EVENT, reason);
        }
    });
    slot.put(Connection {
        outgoing: tx,
        reader,
    });
    Ok(())
}

/// Send one JSON-RPC frame to the daemon.
#[tauri::command]
pub async fn daemon_send(bridge: State<'_, Bridge>, text: String) -> Result<(), String> {
    match bridge.current.lock().await.get() {
        Some(conn) => conn
            .outgoing
            .send(text)
            .map_err(|_| "not connected to forged".to_owned()),
        None => Err("not connected to forged".to_owned()),
    }
}

/// Start `forged` detached from the app, logging to the data dir.
fn start_daemon(paths: &Paths) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    use std::os::unix::process::CommandExt;

    // Private from the start; forged refuses a data dir other users can read.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&paths.data_dir)
        .map_err(|e| format!("creating {}: {e}", paths.data_dir.display()))?;
    let log_path = paths.daemon_log();
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(&log_path)
        .map_err(|e| format!("opening {}: {e}", log_path.display()))?;
    let log_err = log.try_clone().map_err(|e| e.to_string())?;
    let binary = daemon_binary();
    eprintln!("forge-desktop: starting {}", binary.display());
    let mut child = Command::new(&binary)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(log_err)
        // Its own process group, so it outlives the app and ignores the terminal's Ctrl-C.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("starting {}: {e}", binary.display()))?;
    // Reap it if it exits while the app is still running.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// `forged` next to this executable (dev builds and bundles), else from `PATH`.
fn daemon_binary() -> PathBuf {
    std::env::current_exe()
        .ok()
        .map(|exe| exe.with_file_name("forged"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("forged"))
}

async fn wait_for_socket(socket: &Path) -> Result<UnixStream, String> {
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => return Ok(stream),
            Err(e) if Instant::now() >= deadline => {
                return Err(format!(
                    "forged did not start listening on {} within {}s: {e}",
                    socket.display(),
                    START_TIMEOUT.as_secs()
                ));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Slot;

    #[test]
    fn a_replaced_owner_cannot_clear_the_slot() {
        let mut slot = Slot::default();
        let first = slot.next_id();
        assert_eq!(slot.put("a"), None);
        let second = slot.next_id();
        assert_eq!(slot.put("b"), Some("a"));

        // The first connection's reader ends late: it must not clear the second.
        assert_eq!(slot.take_if(first), None);
        assert_eq!(slot.get(), Some(&"b"));
        assert_eq!(slot.take_if(second), Some("b"));
        assert_eq!(slot.get(), None);
    }

    #[test]
    fn take_empties_the_slot() {
        let mut slot = Slot::default();
        slot.put(1);
        assert_eq!(slot.take(), Some(1));
        assert_eq!(slot.take(), None);
    }
}
