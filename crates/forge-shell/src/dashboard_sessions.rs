//! Dashboard Tauri commands and their pure helpers.
//!
//! See `docs/architecture/persistence.md` §7.1, §7.5 and
//! `docs/architecture/window-hierarchy.md` §3.1.
//!
//! The core logic (`SessionSummary`, `collect_sessions`, `Pinger`, `UdsPinger`)
//! is compiled unconditionally so it can be unit-tested under
//! `--no-default-features`. The `#[tauri::command]` wrappers and their
//! `invoke_handler` registration live behind the `webview` feature.

use std::path::{Path, PathBuf};

use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use forge_core::meta::read_meta;
use forge_core::workspaces::read_workspaces;
use forge_core::SessionPersistence;
use serde::Serialize;

/// Wire shape consumed by the Dashboard's sessions panel.
///
/// `state` is a lowercase wire-string (`"active" | "archived" | "stopped"`)
/// and is intentionally decoupled from `forge_core::SessionState` — the UI
/// surfaces a "stopped" state for active sessions whose UDS socket fails to
/// respond, which has no equivalent in the core enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub subject: String,
    pub state: String,
    pub persistence: String,
    /// ISO-8601 UTC timestamp.
    pub created_at: String,
    /// ISO-8601 UTC timestamp.
    pub last_event_at: String,
}

/// Liveness probe for an active session's UDS socket. Injected so
/// `collect_sessions` can be unit-tested without a live server.
#[async_trait]
pub trait Pinger: Send + Sync {
    async fn ping(&self, socket: &Path) -> bool;
}

/// Scan every workspace listed in `workspaces_toml` for sessions and return
/// their summaries. A missing registry yields an empty list.
pub async fn collect_sessions(
    workspaces_toml: &Path,
    pinger: &dyn Pinger,
) -> Result<Vec<SessionSummary>> {
    if !workspaces_toml.exists() {
        return Ok(Vec::new());
    }

    let workspaces = read_workspaces(workspaces_toml).await?;
    let mut summaries = Vec::new();

    for workspace in &workspaces {
        let sessions_root = workspace.path.join(".forge").join("sessions");
        scan_active(&sessions_root, pinger, &mut summaries).await?;
        scan_archived(&sessions_root.join("archived"), &mut summaries).await?;
    }

    Ok(summaries)
}

/// Iterate direct children of `sessions_root` (skipping `archived/`), treat
/// each as an active session dir, and push its summary.
async fn scan_active(
    sessions_root: &Path,
    pinger: &dyn Pinger,
    out: &mut Vec<SessionSummary>,
) -> Result<()> {
    let Some(entries) = read_dir_opt(sessions_root).await? else {
        return Ok(());
    };
    for entry in entries {
        if entry.file_name() == std::ffi::OsStr::new("archived") {
            continue;
        }
        if !entry.is_dir {
            continue;
        }
        if let Some(summary) = summarize_active(&entry.path, pinger).await? {
            out.push(summary);
        }
    }
    Ok(())
}

/// Iterate direct children of `archived_root`, treat each as an archived
/// session dir, and push its summary.
async fn scan_archived(archived_root: &Path, out: &mut Vec<SessionSummary>) -> Result<()> {
    let Some(entries) = read_dir_opt(archived_root).await? else {
        return Ok(());
    };
    for entry in entries {
        if !entry.is_dir {
            continue;
        }
        if let Some(summary) = summarize_archived(&entry.path).await? {
            out.push(summary);
        }
    }
    Ok(())
}

struct DirEntry {
    path: PathBuf,
    is_dir: bool,
}

impl DirEntry {
    fn file_name(&self) -> &std::ffi::OsStr {
        self.path
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new(""))
    }
}

async fn read_dir_opt(dir: &Path) -> Result<Option<Vec<DirEntry>>> {
    let mut rd = match tokio::fs::read_dir(dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut out = Vec::new();
    while let Some(entry) = rd.next_entry().await? {
        let ft = entry.file_type().await?;
        out.push(DirEntry {
            path: entry.path(),
            is_dir: ft.is_dir(),
        });
    }
    Ok(Some(out))
}

async fn summarize_active(
    session_dir: &Path,
    pinger: &dyn Pinger,
) -> Result<Option<SessionSummary>> {
    let Some(meta) = load_meta(session_dir).await? else {
        return Ok(None);
    };
    let alive = pinger.ping(&meta.socket_path).await;
    let state = if alive { "active" } else { "stopped" };
    Ok(Some(make_summary(state, &meta, session_dir).await))
}

async fn summarize_archived(session_dir: &Path) -> Result<Option<SessionSummary>> {
    let Some(meta) = load_meta(session_dir).await? else {
        return Ok(None);
    };
    Ok(Some(make_summary("archived", &meta, session_dir).await))
}

async fn load_meta(session_dir: &Path) -> Result<Option<forge_core::meta::SessionMeta>> {
    let meta_path = session_dir.join("meta.toml");
    if !meta_path.exists() {
        return Ok(None);
    }
    Ok(Some(read_meta(&meta_path).await?))
}

async fn make_summary(
    wire_state: &str,
    meta: &forge_core::meta::SessionMeta,
    session_dir: &Path,
) -> SessionSummary {
    let last_event_at = last_event_at(session_dir, meta.started_at).await;
    SessionSummary {
        id: meta.id.to_string(),
        subject: meta.name.clone(),
        state: wire_state.to_string(),
        persistence: match meta.persistence {
            SessionPersistence::Persist => "persist".to_string(),
            SessionPersistence::Ephemeral => "ephemeral".to_string(),
        },
        created_at: meta.started_at.to_rfc3339(),
        last_event_at: last_event_at.to_rfc3339(),
    }
}

/// Last event wall-clock, taken from `events.jsonl` mtime. Falls back to
/// `started_at` if the log is absent or mtime lookup fails.
async fn last_event_at(session_dir: &Path, fallback: DateTime<Utc>) -> DateTime<Utc> {
    let log = session_dir.join("events.jsonl");
    let Ok(metadata) = tokio::fs::metadata(&log).await else {
        return fallback;
    };
    metadata
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
        .unwrap_or(fallback)
}

/// Production `Pinger` that talks real UDS. Connects with a 250ms cap and
/// completes a `Hello`→`HelloAck` round-trip within a 500ms total budget,
/// so a stalled dashboard ping never blocks the UI.
pub struct UdsPinger;

const PING_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);
const PING_TOTAL_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(500);

#[async_trait]
impl Pinger for UdsPinger {
    async fn ping(&self, socket: &Path) -> bool {
        use forge_ipc::{ClientInfo, FramedStream, Hello, IpcMessage, PROTO_VERSION};
        use tokio::net::UnixStream;

        let connect = tokio::time::timeout(PING_CONNECT_TIMEOUT, UnixStream::connect(socket));
        let stream = match connect.await {
            Ok(Ok(s)) => s,
            _ => return false,
        };

        let handshake = async {
            let mut framed = FramedStream::new(stream);
            framed
                .send(&IpcMessage::Hello(Hello {
                    proto: PROTO_VERSION,
                    client: ClientInfo {
                        kind: "forge-shell".into(),
                        pid: std::process::id(),
                        user: whoami(),
                    },
                }))
                .await
                .ok()?;
            match framed.recv::<IpcMessage>().await.ok()? {
                Some(IpcMessage::HelloAck(_)) => Some(()),
                _ => None,
            }
        };

        matches!(
            tokio::time::timeout(PING_TOTAL_TIMEOUT, handshake).await,
            Ok(Some(()))
        )
    }
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

/// Resolve the path to the user's global `workspaces.toml`
/// (`~/.config/forge/workspaces.toml`).
pub fn default_workspaces_toml() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    base.join("forge").join("workspaces.toml")
}

/// Tauri command: return the sessions panel's data for the Dashboard.
#[cfg(feature = "webview")]
#[tauri::command]
pub async fn session_list<R: tauri::Runtime>(
    webview: tauri::Webview<R>,
) -> Result<Vec<SessionSummary>, String> {
    crate::ipc::require_window_label(&webview, "dashboard")?;
    collect_sessions(&default_workspaces_toml(), &UdsPinger)
        .await
        .map_err(|e| e.to_string())
}

/// Tauri command: open (or focus) the Session window for `id`. Delegates to
/// the F-019 `WindowManager`.
#[cfg(feature = "webview")]
#[tauri::command]
pub async fn open_session<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
    id: String,
) -> Result<(), String> {
    crate::ipc::require_window_label(&webview, "dashboard")?;
    crate::window_manager::WindowManager::new(app)
        .open_session(&id)
        .map(|_| ())
        .map_err(|e| e.to_string())
}
