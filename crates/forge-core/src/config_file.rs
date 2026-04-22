//! Shared atomic-write plumbing for workspace/user config files (F-372).
//!
//! Three prior call sites — `approvals.rs`, `settings.rs`, and the layouts
//! write path in `forge_shell::ipc` — each carried their own copy of the same
//! four-step pattern:
//!
//! 1. Serialize to a body.
//! 2. `create_dir_all` the parent.
//! 3. Write to `<path>.tmp`.
//! 4. `rename` the tmp into place (atomic on POSIX for same-filesystem targets;
//!    same directory here by construction).
//!
//! Plus matching load paths that degrade a missing file to `T::default()`.
//! F-363 showed the layouts copy had already drifted (it dropped the atomic
//! rename). This module is the single source of truth so any future drift
//! touches one file and one test set.
//!
//! ## Degradation policy split (deliberate)
//!
//! TOML configs ([`load_toml_or_default`]) treat a missing file as default
//! but *malformed contents* as an error — approvals and settings both want
//! the IPC layer to surface "your settings file has a syntax error" rather
//! than silently wipe the user's prefs.
//!
//! The layouts JSON ([`load_json_or_default`]) takes the opposite stance:
//! missing *and* malformed both degrade to default. A crash mid-write that
//! leaves a truncated JSON payload must not brick the next session open with
//! a blank window. Losing the prior layout is recoverable; losing the ability
//! to mount any layout is not.
//!
//! ## IPC error mapping
//!
//! Tauri command handlers want `Result<T, String>` with a human-prefix. The
//! `_ipc` variants ([`save_json_atomic_ipc`]) collapse the error to a string
//! with a caller-supplied prefix so the handler body is one line and the
//! prefix convention stays consistent across commands.

use std::path::{Path, PathBuf};

use serde::{de::DeserializeOwned, Serialize};

use crate::Result;

/// Sibling `<path>.tmp` for the atomic tmp+rename write. Same directory by
/// construction so the rename is same-filesystem (POSIX-atomic).
pub fn tmp_path_for(path: &Path) -> PathBuf {
    let mut file_name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    file_name.push(".tmp");
    match path.parent() {
        Some(parent) => parent.join(file_name),
        None => PathBuf::from(file_name),
    }
}

/// Read `path` and deserialize as TOML. Returns `T::default()` if the file is
/// absent; surfaces any other read or parse error.
pub async fn load_toml_or_default<T>(path: &Path) -> Result<T>
where
    T: DeserializeOwned + Default,
{
    match tokio::fs::read_to_string(path).await {
        Ok(contents) => {
            let value: T = toml::from_str(&contents).map_err(|e| anyhow::anyhow!(e))?;
            Ok(value)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e.into()),
    }
}

/// Serialize `value` to TOML and atomically write to `path` via tmp+rename.
/// Creates `path.parent()` if absent.
pub async fn save_toml_atomic<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    let body = toml::to_string(value).map_err(|e| anyhow::anyhow!(e))?;
    save_raw_atomic(path, body.as_bytes()).await
}

/// Read `path` and deserialize as JSON. Missing *or* malformed files degrade
/// to `T::default()` — see module docs on why layouts' failure mode differs
/// from the TOML configs. Read errors other than `NotFound` (e.g. permissions
/// anomalies) also degrade, matching the prior inline behavior.
pub async fn load_json_or_default<T>(path: &Path) -> T
where
    T: DeserializeOwned + Default,
{
    let Ok(bytes) = tokio::fs::read(path).await else {
        return T::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Serialize `value` to pretty JSON and atomically write to `path`.
pub async fn save_json_atomic<T>(path: &Path, value: &T) -> Result<()>
where
    T: Serialize,
{
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| anyhow::anyhow!(e))?;
    save_raw_atomic(path, &bytes).await
}

/// IPC-surface variant of [`save_json_atomic`] that collapses errors to a
/// `String` prefixed with `error_prefix`. Used by Tauri command handlers that
/// return `Result<T, String>`.
pub async fn save_json_atomic_ipc<T>(
    path: &Path,
    value: &T,
    error_prefix: &str,
) -> std::result::Result<(), String>
where
    T: Serialize,
{
    save_json_atomic(path, value)
        .await
        .map_err(|e| format!("{error_prefix}: {e}"))
}

/// Atomically write `body` verbatim to `path`. Creates `path.parent()` if
/// absent. Used when the body is already a serialized payload (e.g. the output
/// of `settings::apply_setting_update`, which must bypass struct-roundtrip —
/// see `settings` module docs).
pub async fn save_raw_atomic(path: &Path, body: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let tmp = tmp_path_for(path);
    tokio::fs::write(&tmp, body).await?;
    tokio::fs::rename(&tmp, path).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use tempfile::TempDir;

    #[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
    struct Sample {
        #[serde(default)]
        name: String,
        #[serde(default)]
        count: u32,
    }

    // ------------------------------------------------------------------
    // missing-file-defaults
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn load_toml_missing_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.toml");
        let v: Sample = load_toml_or_default(&path).await.unwrap();
        assert_eq!(v, Sample::default());
    }

    #[tokio::test]
    async fn load_json_missing_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("missing.json");
        let v: Sample = load_json_or_default(&path).await;
        assert_eq!(v, Sample::default());
    }

    #[tokio::test]
    async fn load_json_corrupt_returns_default() {
        // Distinct from TOML — malformed JSON must degrade silently (the
        // layouts fallback policy).
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.json");
        tokio::fs::write(&path, b"{not valid json").await.unwrap();
        let v: Sample = load_json_or_default(&path).await;
        assert_eq!(v, Sample::default());
    }

    #[tokio::test]
    async fn load_toml_malformed_returns_error() {
        // Opposite policy from JSON: a TOML parse error must surface so the
        // IPC layer can flag it, not silently wipe.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.toml");
        tokio::fs::write(&path, b"this is not = valid [ toml")
            .await
            .unwrap();
        let res: Result<Sample> = load_toml_or_default(&path).await;
        assert!(res.is_err(), "malformed TOML must surface an error");
    }

    // ------------------------------------------------------------------
    // tmp-rename atomicity (simulate crash)
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn save_toml_leaves_no_tmp_residue() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("cfg.toml");
        let sample = Sample {
            name: "n".into(),
            count: 3,
        };
        save_toml_atomic(&path, &sample).await.unwrap();
        assert!(path.exists(), "final file must exist after save");
        assert!(
            !tmp_path_for(&path).exists(),
            "tmp must be renamed, not left behind"
        );
    }

    #[tokio::test]
    async fn save_json_leaves_no_tmp_residue() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nested").join("cfg.json");
        let sample = Sample {
            name: "n".into(),
            count: 3,
        };
        save_json_atomic(&path, &sample).await.unwrap();
        assert!(path.exists());
        assert!(!tmp_path_for(&path).exists());
    }

    #[tokio::test]
    async fn save_creates_missing_parent_dir() {
        let dir = TempDir::new().unwrap();
        let path = dir
            .path()
            .join("does")
            .join("not")
            .join("exist")
            .join("c.toml");
        save_toml_atomic(&path, &Sample::default()).await.unwrap();
        assert!(path.exists());
    }

    #[tokio::test]
    async fn crash_mid_write_leaves_last_valid_on_disk() {
        // Simulate: a previous write crashed between writing the tmp and
        // renaming it. The next successful save must overwrite that stale
        // `.tmp` rather than be confused by it, and the final file must hold
        // the new body. Readers see either the pre-crash file or the new one
        // — never a partial.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cfg.toml");

        // Pre-populate with "previous valid" contents.
        let v1 = Sample {
            name: "first".into(),
            count: 1,
        };
        save_toml_atomic(&path, &v1).await.unwrap();

        // Drop a half-written `.tmp` on disk (the crash residue).
        let tmp = tmp_path_for(&path);
        tokio::fs::write(&tmp, b"this would be a partial payload")
            .await
            .unwrap();
        assert!(tmp.exists());

        // Recovery write.
        let v2 = Sample {
            name: "second".into(),
            count: 2,
        };
        save_toml_atomic(&path, &v2).await.unwrap();

        // The stale tmp is overwritten + renamed into place — no residue.
        assert!(!tmp.exists(), "recovery write must consume the stale tmp");
        // And the on-disk file is the new valid body, never the partial.
        let loaded: Sample = load_toml_or_default(&path).await.unwrap();
        assert_eq!(loaded, v2);
    }

    #[tokio::test]
    async fn tmp_path_is_sibling_with_tmp_suffix() {
        let p = Path::new("/a/b/c.toml");
        assert_eq!(tmp_path_for(p), PathBuf::from("/a/b/c.toml.tmp"));

        // Edge: no parent.
        let p = Path::new("c.toml");
        assert_eq!(tmp_path_for(p), PathBuf::from("c.toml.tmp"));
    }

    // ------------------------------------------------------------------
    // IPC error mapping
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn save_json_atomic_ipc_maps_success_to_ok() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ok.json");
        let res = save_json_atomic_ipc(&path, &Sample::default(), "write cfg").await;
        assert!(res.is_ok());
    }

    #[tokio::test]
    async fn save_json_atomic_ipc_maps_io_error_to_string_with_prefix() {
        // Point at a path whose parent creation is blocked — writing under a
        // path whose ancestor is an existing *file* (not a dir) fails at
        // `create_dir_all`. This exercises the error surface without needing
        // permissions tricks.
        let dir = TempDir::new().unwrap();
        let blocker = dir.path().join("blocker");
        tokio::fs::write(&blocker, b"i am a file, not a dir")
            .await
            .unwrap();
        // Now ask to save *inside* `blocker`, which is a regular file.
        let path = blocker.join("child").join("cfg.json");

        let err = save_json_atomic_ipc(&path, &Sample::default(), "write cfg")
            .await
            .expect_err("expected write to fail under a file-shaped blocker");
        assert!(
            err.starts_with("write cfg: "),
            "expected caller-supplied prefix, got {err:?}"
        );
    }

    // ------------------------------------------------------------------
    // round-trip
    // ------------------------------------------------------------------

    #[tokio::test]
    async fn save_then_load_toml_roundtrips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rt.toml");
        let sample = Sample {
            name: "hello".into(),
            count: 9,
        };
        save_toml_atomic(&path, &sample).await.unwrap();
        let loaded: Sample = load_toml_or_default(&path).await.unwrap();
        assert_eq!(loaded, sample);
    }

    #[tokio::test]
    async fn save_then_load_json_roundtrips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("rt.json");
        let sample = Sample {
            name: "hello".into(),
            count: 9,
        };
        save_json_atomic(&path, &sample).await.unwrap();
        let loaded: Sample = load_json_or_default(&path).await;
        assert_eq!(loaded, sample);
    }

    #[tokio::test]
    async fn save_raw_atomic_bypasses_serde() {
        // `save_raw_atomic` is the seam for `settings::apply_setting_update`,
        // which hands in pre-formatted TOML text. Writing it verbatim must
        // preserve byte-for-byte what the caller supplied.
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("raw.toml");
        let body = b"[section]\nkey = \"value\"\n# comment line\n";
        save_raw_atomic(&path, body).await.unwrap();
        let on_disk = tokio::fs::read(&path).await.unwrap();
        assert_eq!(on_disk, body);
    }
}
