use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufWriter};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio::time::Duration;

use crate::event::Event;
use crate::workspace;
use crate::{ForgeError, Result};

const SCHEMA_HEADER: &str = r#"{"schema_version":1}"#;
const FLUSH_INTERVAL: Duration = Duration::from_millis(50);

pub struct EventLog {
    writer: Arc<Mutex<BufWriter<tokio::fs::File>>>,
    flush_task: JoinHandle<()>,
}

impl EventLog {
    pub async fn create(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if let Some(root) = forge_dir_parent(path) {
            workspace::ensure_gitignore(root).await?;
        }
        let file = tokio::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .await?;
        let mut writer = BufWriter::new(file);
        writer.write_all(SCHEMA_HEADER.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
        Ok(Self::with_writer(writer))
    }

    pub async fn open(path: &Path) -> Result<Self> {
        let mut file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .await?;

        // Validate schema header using a single file handle to avoid TOCTOU.
        let expected = format!("{SCHEMA_HEADER}\n");
        let mut buf = vec![0u8; expected.len()];
        file.read_exact(&mut buf).await.map_err(|_| {
            ForgeError::Other(anyhow::anyhow!(
                "events.jsonl too short or missing schema header"
            ))
        })?;
        if buf != expected.as_bytes() {
            return Err(ForgeError::Other(anyhow::anyhow!(
                "events.jsonl schema header mismatch: expected {SCHEMA_HEADER:?}"
            )));
        }

        // Seek to end on the same handle before wrapping in BufWriter.
        file.seek(std::io::SeekFrom::End(0)).await?;
        Ok(Self::with_writer(BufWriter::new(file)))
    }

    fn with_writer(writer: BufWriter<tokio::fs::File>) -> Self {
        let shared = Arc::new(Mutex::new(writer));
        let flush_handle = {
            let shared = Arc::clone(&shared);
            tokio::spawn(async move {
                let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
                loop {
                    ticker.tick().await;
                    let _ = shared.lock().await.flush().await;
                }
            })
        };
        Self {
            writer: shared,
            flush_task: flush_handle,
        }
    }

    pub async fn append(&mut self, event: &Event) -> Result<()> {
        let line = serde_json::to_string(event)?;
        let mut w = self.writer.lock().await;
        w.write_all(line.as_bytes()).await?;
        w.write_all(b"\n").await?;
        Ok(())
    }

    pub async fn flush(&mut self) -> Result<()> {
        self.writer.lock().await.flush().await?;
        Ok(())
    }

    pub async fn close(mut self) -> Result<()> {
        self.flush_task.abort();
        self.flush().await
    }
}

/// Returns the workspace root if `path` is nested under a `.forge` directory.
/// Returns `None` (and silently skips gitignore creation) for paths outside `.forge/`.
/// Production callers are expected to always nest EventLog files under `.forge/`.
fn forge_dir_parent(path: &Path) -> Option<&Path> {
    let mut cur = path.parent()?;
    loop {
        if cur.file_name()? == ".forge" {
            return cur.parent();
        }
        cur = cur.parent()?;
    }
}

impl Drop for EventLog {
    fn drop(&mut self) {
        self.flush_task.abort();
    }
}
