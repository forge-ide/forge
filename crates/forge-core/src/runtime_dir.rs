//! Per-user runtime directory resolution for the `forged` daemon, CLI, and
//! Tauri shell.
//!
//! # F-044 (H8) invariant, preserved
//!
//! Prior to F-044 the various resolvers fell back to `/tmp/forge-{uid}` when
//! `XDG_RUNTIME_DIR` was unset. The socket under `/tmp` is world-connectable,
//! so any local user could drive another user's session. F-044 closed that
//! fallback — the resolvers now refuse to return a path when the runtime dir
//! cannot be established as a per-user, mode-`0o700` directory.
//!
//! # F-339 (macOS fallback)
//!
//! Apple's launchd does not export `XDG_RUNTIME_DIR`, so a fresh `cargo run
//! -p forge-shell` on macOS errored out at startup. Rather than relax the
//! F-044 invariant, this module resolves a natively-appropriate per-user
//! directory on macOS: `$HOME/Library/Application Support/Forge/run`. The
//! directory is created with mode `0o700` when missing; an existing
//! directory whose mode is not `0o700` is rejected (to catch either a
//! downgraded-perm state or a shared-dir misconfiguration).
//!
//! Linux behavior is unchanged: `XDG_RUNTIME_DIR` must be set (systemd
//! provides `/run/user/<uid>` automatically), and no `/tmp` fallback is
//! ever attempted. Windows is not supported yet; see the TODO in
//! [`runtime_dir_with`].

use std::path::PathBuf;

use anyhow::{anyhow, Result};

/// Resolve the per-user runtime directory used by `forged` and its clients.
///
/// Reads `XDG_RUNTIME_DIR` from the environment and `$HOME` via
/// [`dirs::home_dir`]; delegates the actual policy to [`runtime_dir_with`]
/// so tests can exercise every branch without mutating process-global env.
pub fn runtime_dir() -> Result<PathBuf> {
    let xdg = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|s| !s.is_empty());
    let home = dirs::home_dir();
    runtime_dir_with(xdg.as_deref(), home.as_deref())
}

/// DI variant of [`runtime_dir`] — takes the env/home values explicitly so
/// tests can drive the macOS and Linux branches with tempdirs instead of
/// unsafe env mutation.
///
/// Policy:
///   - If `XDG_RUNTIME_DIR` is set (non-empty), use it. This preserves the
///     Linux priority ordering on every platform, so operators can still
///     override the macOS fallback by exporting `XDG_RUNTIME_DIR` explicitly.
///   - Else, on macOS, return `$HOME/Library/Application Support/Forge/run`,
///     creating it with mode `0o700` if missing; reject a pre-existing
///     directory whose mode is not `0o700`.
///   - Else, on Linux, error with a message naming `XDG_RUNTIME_DIR`.
///   - Else, on Windows, error (tracked as a follow-up; Windows daemon/CLI
///     is out of scope for F-339).
pub fn runtime_dir_with(xdg: Option<&str>, home: Option<&std::path::Path>) -> Result<PathBuf> {
    if let Some(xdg) = xdg.filter(|s| !s.is_empty()) {
        return Ok(PathBuf::from(xdg));
    }
    platform_fallback(home)
}

#[cfg(target_os = "macos")]
fn platform_fallback(home: Option<&std::path::Path>) -> Result<PathBuf> {
    let home = home.ok_or_else(|| {
        anyhow!(
            "XDG_RUNTIME_DIR is unset and $HOME is unavailable; \
             forge cannot resolve a per-user runtime directory. \
             Set XDG_RUNTIME_DIR to a 0o700 directory or ensure \
             $HOME is set. (F-044 / F-339)"
        )
    })?;
    let dir = home.join("Library/Application Support/Forge/run");
    ensure_macos_runtime_dir(&dir)?;
    Ok(dir)
}

#[cfg(target_os = "linux")]
fn platform_fallback(_home: Option<&std::path::Path>) -> Result<PathBuf> {
    // F-044 / H8: Linux must never fall back to a shared directory. The
    // socket there would be world-connectable; any local user could drive
    // another user's session. systemd provides a per-user 0o700 tmpfs at
    // `/run/user/<uid>` — any non-systemd deployment must set the env var
    // explicitly before starting forge.
    Err(anyhow!(
        "XDG_RUNTIME_DIR is unset: forge refuses to fall back to a \
         shared /tmp directory because the socket there would be \
         world-connectable. Set XDG_RUNTIME_DIR to a per-user 0o700 \
         directory (systemd sets it to /run/user/<uid> automatically). \
         (F-044 / H8)"
    ))
}

// TODO(F-339 follow-up): Windows daemon/CLI/shell is out of scope for
// this task. When Windows support lands, pick a per-user directory
// under `%LOCALAPPDATA%` (or equivalent) and enforce the same
// single-user ACL invariant as macOS/Linux.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn platform_fallback(_home: Option<&std::path::Path>) -> Result<PathBuf> {
    Err(anyhow!(
        "XDG_RUNTIME_DIR is unset and no platform-native fallback is \
         implemented for this target; set XDG_RUNTIME_DIR explicitly. \
         (F-044 / F-339)"
    ))
}

/// macOS runtime-dir creation + permission enforcement.
///
/// The whole F-044 rationale is that the socket's parent must be per-user
/// (`0o700`). We enforce that on both sides of the creation:
///   - If the directory is missing, create it (and its parents) with mode
///     `0o700` via [`std::os::unix::fs::DirBuilderExt::mode`]. This applies
///     the mode to any intermediate component we create, which is the
///     conservative choice (`~/Library` and `~/Library/Application Support`
///     already exist on every macOS install; the only component this
///     typically creates is the final `Forge/run` leaf).
///   - If the directory exists, read its mode. Refuse if `mode & 0o777`
///     isn't exactly `0o700`. We deliberately do not auto-chmod — a
///     pre-existing relaxed directory may be adversarial (another user's
///     directory we inherited through a user-switch), and silently
///     downgrading its perms would hide the problem.
#[cfg(target_os = "macos")]
fn ensure_macos_runtime_dir(dir: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    match std::fs::metadata(dir) {
        Ok(meta) => {
            if !meta.is_dir() {
                return Err(anyhow!(
                    "{} exists but is not a directory; refusing to use as a \
                     per-user runtime dir. (F-044 / F-339)",
                    dir.display()
                ));
            }
            let mode = meta.permissions().mode() & 0o777;
            if mode != 0o700 {
                return Err(anyhow!(
                    "{} exists with mode 0o{:o}; expected 0o700 \
                     (F-044 per-user runtime invariant). Remove or chmod \
                     the directory and retry. (F-339)",
                    dir.display(),
                    mode
                ));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(|e| {
                    anyhow!(
                        "failed to create per-user runtime dir {} \
                         with mode 0o700: {e} (F-339)",
                        dir.display()
                    )
                })?;
            Ok(())
        }
        Err(e) => Err(anyhow!(
            "failed to stat runtime dir {}: {e} (F-339)",
            dir.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xdg_set_wins_on_all_platforms() {
        // Priority: XDG_RUNTIME_DIR always beats the platform fallback,
        // so Linux operators' existing muscle memory (and ops playbooks)
        // keep working on macOS too.
        let got = runtime_dir_with(
            Some("/run/user/4242"),
            Some(std::path::Path::new("/home/x")),
        )
        .expect("xdg set -> Ok");
        assert_eq!(got, PathBuf::from("/run/user/4242"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn empty_xdg_is_treated_as_unset_on_linux() {
        // Empty XDG_RUNTIME_DIR must fall through to the same branch as
        // unset — matches the filter in `runtime_dir()`. Linux: error.
        let home = std::path::Path::new("/home/x");
        let result = runtime_dir_with(Some(""), Some(home));
        assert!(result.is_err(), "empty xdg on Linux -> err");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn empty_xdg_is_treated_as_unset_on_macos() {
        // Empty XDG_RUNTIME_DIR must fall through to the same branch as
        // unset — matches the filter in `runtime_dir()`. macOS with a
        // tempdir HOME: drives the fallback successfully.
        let tmp = tempfile::tempdir().expect("tempdir");
        let got = runtime_dir_with(Some(""), Some(tmp.path())).expect("empty xdg -> fallback");
        assert!(got.ends_with("Library/Application Support/Forge/run"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_errors_when_xdg_unset_regardless_of_home() {
        // F-044 regression guard: Linux must never resolve without
        // XDG_RUNTIME_DIR. The macOS fallback is cfg-gated out.
        let err = runtime_dir_with(None, Some(std::path::Path::new("/home/x")))
            .expect_err("linux + no xdg -> err");
        let msg = err.to_string();
        assert!(
            msg.contains("XDG_RUNTIME_DIR"),
            "error must name XDG_RUNTIME_DIR so operators see the fix, got: {msg}"
        );
        assert!(
            !msg.contains("/tmp/forge-"),
            "error must not advertise an actual /tmp/forge- resolved path, \
             got: {msg}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_creates_runtime_dir_with_mode_700_when_missing() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = tempfile::tempdir().expect("tempdir");
        // Simulate $HOME under tempdir so we can inspect what the helper
        // creates without touching the real user profile.
        let got = runtime_dir_with(None, Some(tmp.path())).expect("macos fallback -> Ok");
        let expected = tmp.path().join("Library/Application Support/Forge/run");
        assert_eq!(got, expected);
        assert!(expected.is_dir(), "helper must create the dir");
        let mode = std::fs::metadata(&expected)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o700,
            "leaf dir must be 0o700 per F-044 invariant, got 0o{mode:o}"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_accepts_preexisting_0700_dir() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let tmp = tempfile::tempdir().expect("tempdir");
        let expected = tmp.path().join("Library/Application Support/Forge/run");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&expected)
            .expect("pre-create 0o700");

        let got = runtime_dir_with(None, Some(tmp.path())).expect("preexisting 0o700 -> Ok");
        assert_eq!(got, expected);
        let mode = std::fs::metadata(&expected)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_rejects_preexisting_dir_with_loose_perms() {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let tmp = tempfile::tempdir().expect("tempdir");
        let expected = tmp.path().join("Library/Application Support/Forge/run");
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o755)
            .create(&expected)
            .expect("pre-create 0o755");

        let err = runtime_dir_with(None, Some(tmp.path())).expect_err("loose perms -> err");
        let msg = err.to_string();
        assert!(
            msg.contains("0o755"),
            "error must name the observed mode, got: {msg}"
        );
        assert!(
            msg.contains("0o700"),
            "error must name the required mode, got: {msg}"
        );
        // The helper must not auto-chmod — the existing relaxed state
        // could be adversarial and silently repairing it would hide the
        // problem.
        let mode = std::fs::metadata(&expected)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o755, "helper must not silently chmod");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_honors_xdg_when_set() {
        // When XDG_RUNTIME_DIR IS set on macOS, we MUST use it verbatim —
        // not the Library fallback — so the priority ordering matches
        // Linux. An operator who exports XDG_RUNTIME_DIR on macOS (e.g.
        // for testing / custom sandboxes) deserves the same contract.
        let tmp = tempfile::tempdir().expect("tempdir");
        let got = runtime_dir_with(Some("/run/user/4242"), Some(tmp.path()))
            .expect("xdg set on macos -> Ok");
        assert_eq!(got, PathBuf::from("/run/user/4242"));
        // And the fallback dir must NOT have been created as a side effect.
        assert!(
            !tmp.path().join("Library").exists(),
            "XDG-set path must not create the macOS fallback tree"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_errors_when_home_missing() {
        // $HOME unavailable is a hard error on macOS when XDG is also unset
        // — we cannot invent a per-user directory out of thin air.
        let err = runtime_dir_with(None, None).expect_err("no xdg + no home -> err");
        let msg = err.to_string();
        assert!(
            msg.contains("HOME") || msg.contains("home"),
            "error must mention $HOME, got: {msg}"
        );
    }
}
