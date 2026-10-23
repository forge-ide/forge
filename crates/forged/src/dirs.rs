//! Private directories for the daemon's data and socket.

use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

use anyhow::{Context, bail};

/// Create `path` (and any missing parents) with mode 0700, or check that an existing directory
/// is closed to other users. Never changes the mode of a directory forged didn't create, since
/// `FORGE_DATA_DIR` or `FORGE_RUNTIME_DIR` may name a shared one.
pub fn ensure_private(path: &Path) -> anyhow::Result<()> {
    match std::fs::metadata(path) {
        Ok(meta) => {
            if !meta.is_dir() {
                bail!("{} is not a directory", path.display());
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                bail!(
                    "{} is open to other users (mode {mode:o}); run `chmod 700 {}` or choose another directory",
                    path.display(),
                    path.display()
                );
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .with_context(|| format!("creating {}", path.display())),
        Err(e) => Err(e).with_context(|| format!("checking {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn creates_missing_directories_private() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/b");
        ensure_private(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        ensure_private(&dir).unwrap();
    }

    #[test]
    fn refuses_shared_directories_and_leaves_them_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("shared");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = ensure_private(&dir).unwrap_err();
        assert!(err.to_string().contains("other users"), "{err}");
        assert_eq!(mode(&dir), 0o755);
    }

    #[test]
    fn refuses_files() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("f");
        std::fs::write(&file, "").unwrap();
        assert!(ensure_private(&file).is_err());
    }
}
