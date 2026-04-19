//! F-044 (H8) regression: `resolve_socket_path` rejects the `/tmp` fallback.
//!
//! Before F-044, `resolve_socket_path` fell back to `/tmp/forge-{uid}` when
//! `XDG_RUNTIME_DIR` was unset, and read the UID from `std::env::var("UID")`
//! (a shell variable not generally exported to child processes) with a
//! fallback of `"0"`. The result: multiple local users all binding under
//! `/tmp/forge-0/forge/sessions/`, world-connectable.
//!
//! Remediation (option 4 from H8): require `XDG_RUNTIME_DIR` and refuse to
//! start otherwise. Tests cover:
//!   - success when XDG_RUNTIME_DIR is set
//!   - error when XDG_RUNTIME_DIR is unset (no silent /tmp fallback)
//!   - the error message names `XDG_RUNTIME_DIR` so operators see the fix
//!
//! Both cases run in a single `#[test]` so env mutation can't race another
//! parallel reader in this same test binary.

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

    // --- Case 2: XDG_RUNTIME_DIR unset (and UID unset too) -> Err ---
    // Clear UID so no caller can smuggle a uid through the old fallback.
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
    // Defense-in-depth: never silently resolve to /tmp.
    assert!(
        !msg.contains("/tmp"),
        "error must not advertise a /tmp fallback, got: {msg}"
    );

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
