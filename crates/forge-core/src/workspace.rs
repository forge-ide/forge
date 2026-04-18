use std::path::Path;

use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;

use crate::Result;

/// Creates `.forge/.gitignore` containing `*` under `workspace_root` if it does not exist.
pub async fn ensure_gitignore(workspace_root: &Path) -> Result<()> {
    let forge_dir = workspace_root.join(".forge");
    tokio::fs::create_dir_all(&forge_dir).await?;
    let gi = forge_dir.join(".gitignore");
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&gi)
        .await
    {
        Ok(mut f) => {
            f.write_all(b"*\n").await?;
            f.flush().await?;
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    Ok(())
}
