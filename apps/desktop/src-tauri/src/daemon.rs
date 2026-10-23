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
    outgoing: Mutex<Option<mpsc::UnboundedSender<String>>>,
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

    // Dropping the old sender ends the old writer, which closes that socket.
    *bridge.outgoing.lock().await = Some(tx.clone());

    tauri::async_runtime::spawn(async move {
        while let Some(text) = rx.recv().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    tauri::async_runtime::spawn(async move {
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
        let current = app.state::<Bridge>();
        let mut outgoing = current.outgoing.lock().await;
        if outgoing.as_ref().is_some_and(|s| s.same_channel(&tx)) {
            *outgoing = None;
            drop(outgoing);
            let _ = app.emit(CLOSED_EVENT, reason);
        }
    });
    Ok(())
}

/// Send one JSON-RPC frame to the daemon.
#[tauri::command]
pub async fn daemon_send(bridge: State<'_, Bridge>, text: String) -> Result<(), String> {
    match bridge.outgoing.lock().await.as_ref() {
        Some(tx) => tx
            .send(text)
            .map_err(|_| "not connected to forged".to_owned()),
        None => Err("not connected to forged".to_owned()),
    }
}

/// Start `forged` detached from the app, logging to the data dir.
fn start_daemon(paths: &Paths) -> Result<(), String> {
    use std::os::unix::process::CommandExt;

    std::fs::create_dir_all(&paths.data_dir)
        .map_err(|e| format!("creating {}: {e}", paths.data_dir.display()))?;
    let log_path = paths.daemon_log();
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
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
