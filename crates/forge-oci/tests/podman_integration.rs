//! Integration test: drives a real `podman` against a real image.
//!
//! Auto-skips on:
//! - non-Linux hosts (rootless podman semantics differ; F-595 is Linux-first),
//! - hosts where `podman` is not on `PATH`,
//! - hosts where `PodmanRuntime::detect()` fails (rootless mode unavailable).
//!
//! When skipped, the test prints a single line and returns success rather than
//! failing CI. Real coverage requires `cargo test -p forge-oci` on a Linux
//! host with rootless podman configured.

#![cfg(target_os = "linux")]

use forge_oci::{ContainerRuntime, ImageRef, PodmanRuntime};

fn podman_on_path() -> bool {
    std::process::Command::new("podman")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tokio::test]
async fn podman_full_lifecycle_against_alpine() {
    if !podman_on_path() {
        eprintln!("[forge-oci] skipping: podman not on PATH");
        return;
    }

    let runtime = PodmanRuntime::new();

    if let Err(e) = runtime.detect().await {
        eprintln!("[forge-oci] skipping: podman detect failed: {e}");
        return;
    }

    let image = ImageRef::parse("docker.io/library/alpine:3.19").expect("valid image ref");

    runtime.pull(&image).await.expect("pull alpine");

    // Long-lived foreground process so `exec` has something to attach to.
    // `sleep 60` is plenty for the test to do its work and tear down.
    let handle = runtime
        .create(&image, &["sleep".into(), "60".into()])
        .await
        .expect("create container");

    runtime.start(&handle).await.expect("start container");

    let result = runtime
        .exec(&handle, &["echo".into(), "hello".into()])
        .await
        .expect("exec echo");
    assert_eq!(result.stdout, "hello\n");
    assert_eq!(result.exit_code, Some(0));

    let stats = runtime.stats(&handle).await.expect("stats");
    // Alpine `sleep` is tiny; just assert we got *some* signal back.
    assert!(
        stats.pids.unwrap_or(0) >= 1,
        "expected at least one PID, got {stats:?}"
    );

    runtime.remove(&handle).await.expect("remove container");

    // After remove, `inspect` must fail — proving cleanup actually happened.
    let inspect = std::process::Command::new("podman")
        .args(["inspect", &handle.id])
        .output()
        .expect("podman inspect spawn");
    assert!(
        !inspect.status.success(),
        "expected inspect to fail after remove; stdout={:?}",
        String::from_utf8_lossy(&inspect.stdout)
    );
}
