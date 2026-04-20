use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use anyhow::{anyhow, Result};
use chrono::Utc;
use forge_core::{
    meta::{read_meta, write_meta},
    SessionPersistence, SessionState,
};

pub async fn archive_or_purge(
    session_dir: &Path,
    persistence: SessionPersistence,
    socket_path: &Path,
) -> Result<()> {
    match persistence {
        SessionPersistence::Ephemeral => {
            if session_dir.exists() {
                tokio::fs::remove_dir_all(session_dir).await?;
            }
        }
        SessionPersistence::Persist => {
            let archived_dir = archived_destination(session_dir)?;
            if let Some(parent) = archived_dir.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            move_dir(session_dir, &archived_dir).await?;
            update_meta_to_archived(&archived_dir.join("meta.toml")).await?;
        }
    }

    remove_socket(socket_path).await?;
    Ok(())
}

fn archived_destination(session_dir: &Path) -> Result<PathBuf> {
    let id = session_dir.file_name().ok_or_else(|| {
        anyhow!(
            "session_dir has no final component: {}",
            session_dir.display()
        )
    })?;
    let parent = session_dir
        .parent()
        .ok_or_else(|| anyhow!("session_dir has no parent: {}", session_dir.display()))?;
    Ok(parent.join("archived").join(id))
}

async fn move_dir(src: &Path, dst: &Path) -> Result<()> {
    // Async rename keeps the tokio worker unblocked; large session directories
    // on cross-device renames can otherwise stall for 50-500 ms (F-110).
    handle_rename_result(tokio::fs::rename(src, dst).await, src, dst).await
}

async fn move_dir_with_rename<F>(src: &Path, dst: &Path, rename: F) -> Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    handle_rename_result(rename(src, dst), src, dst).await
}

async fn handle_rename_result(rename_result: io::Result<()>, src: &Path, dst: &Path) -> Result<()> {
    match rename_result {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) => {
            copy_dir_all(src, dst).await?;
            tokio::fs::remove_dir_all(src).await?;
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

fn is_cross_device(err: &io::Error) -> bool {
    err.raw_os_error() == Some(libc::EXDEV)
}

async fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    copy_dir_all_boxed(src.to_path_buf(), dst.to_path_buf()).await
}

fn copy_dir_all_boxed(
    src: PathBuf,
    dst: PathBuf,
) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
    Box::pin(async move {
        tokio::fs::create_dir_all(&dst).await?;
        let mut entries = tokio::fs::read_dir(&src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let file_type = entry.file_type().await?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if file_type.is_dir() {
                copy_dir_all_boxed(src_path, dst_path).await?;
            } else {
                tokio::fs::copy(&src_path, &dst_path).await?;
            }
        }
        Ok(())
    })
}

async fn update_meta_to_archived(meta_path: &Path) -> Result<()> {
    if !meta_path.exists() {
        return Ok(());
    }
    let mut meta = read_meta(meta_path).await?;
    meta.state = SessionState::Archived;
    meta.ended_at = Some(Utc::now());
    write_meta(meta_path, &meta).await?;
    Ok(())
}

async fn remove_socket(socket_path: &Path) -> Result<()> {
    match tokio::fs::remove_file(socket_path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

#[doc(hidden)]
pub async fn copy_dir_all_for_test(src: &Path, dst: &Path) -> Result<()> {
    copy_dir_all(src, dst).await
}

#[doc(hidden)]
pub async fn move_dir_with_rename_for_test<F>(src: &Path, dst: &Path, rename: F) -> Result<()>
where
    F: FnOnce(&Path, &Path) -> io::Result<()>,
{
    move_dir_with_rename(src, dst, rename).await
}
