//! F-044 (H8) + F-339 regression: `resolve_socket_path` refuses the
//! world-connectable `/tmp` fallback, but does fall back to a per-user
//! `0o700` directory under `~/Library/...` on macOS when
//! `XDG_RUNTIME_DIR` is unset.
//!
//! Before F-044, `resolve_socket_path` fell back to `/tmp/forge-{uid}` when
//! `XDG_RUNTIME_DIR` was unset, and read the UID from `std::env::var("UID")`
//! (a shell variable not generally exported to child processes) with a
//! fallback of `"0"`. The result: multiple local users all binding under
//! `/tmp/forge-0/forge/sessions/`, world-connectable.
//!
//! F-044 remediation (option 4 from H8): require a per-user runtime dir and
//! refuse to start otherwise.
//!
//! F-339 refinement: on macOS (where launchd does not export
//! `XDG_RUNTIME_DIR`), fall back to `$HOME/Library/Application Support/
//! Forge/run` at `0o700` — the F-044 invariant is preserved, just via a
//! platform-native path.
//!
//! Tests cover:
//!   - success when XDG_RUNTIME_DIR is set (all platforms)
//!   - Linux-only error when XDG_RUNTIME_DIR is unset (no silent /tmp fallback)
//!   - the error message names `XDG_RUNTIME_DIR` so operators see the fix
//!
//! Both cases run in a single `#[test]` so env mutation can't race another
//! parallel reader in this same test binary. The macOS fallback branch has
//! dedicated DI-based tests in `forge_core::runtime_dir` — we do not
//! mutate `$HOME` here to avoid dragging fragile env state into this file.

use forge_session::socket_path::resolve_socket_path;

#[test]
fn resolve_socket_path_requires_xdg_runtime_dir() {
    let prev_xdg = std::env::var("XDG_RUNTIME_DIR").ok();
    let prev_uid = std::env::var("UID").ok();

    // --- Case 1: XDG_RUNTIME_DIR set -> Ok(path under that dir) ---
    // SAFETY: single-threaded test binary; only this test file exists in
    // this crate target, so no other tokio task concurrently reads env.
    unsafe {
        std::env::set_var("XDG_RUNTIME_DIR", "/run/user/1234");
    }
    let path = resolve_socket_path("abc123").expect("should resolve");
    assert_eq!(
        path.to_str().unwrap(),
        "/run/user/1234/forge/sessions/abc123.sock"
    );

    // --- Case 2 (Linux-only): XDG_RUNTIME_DIR unset -> Err ---
    // On macOS the F-339 fallback path kicks in and uses $HOME/Library/...
    // at 0o700; the dedicated unit tests in `forge_core::runtime_dir`
    // cover that branch with tempdir-injected HOME so this binary does
    // not need to mutate $HOME at all.
    #[cfg(target_os = "linux")]
    {
        // Clear UID too so no caller can smuggle a uid through the old fallback.
        unsafe {
            std::env::remove_var("XDG_RUNTIME_DIR");
            std::env::remove_var("UID");
        }
        let err = resolve_socket_path("abc123").expect_err("must refuse without XDG_RUNTIME_DIR");
        let msg = err.to_string();
        assert!(
            msg.contains("XDG_RUNTIME_DIR"),
            "error must name XDG_RUNTIME_DIR so operators see the fix, got: {msg}"
        );
        // Defense-in-depth: never silently resolve to /tmp/forge-<uid>.
        assert!(
            !msg.contains("/tmp/forge-"),
            "error must not advertise an actual /tmp/forge- path, got: {msg}"
        );
    }

    // Restore
    unsafe {
        match prev_xdg {
            Some(v) => std::env::set_var("XDG_RUNTIME_DIR", v),
            None => std::env::remove_var("XDG_RUNTIME_DIR"),
        }
        match prev_uid {
            Some(v) => std::env::set_var("UID", v),
            None => std::env::remove_var("UID"),
        }
    }
}
