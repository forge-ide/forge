use forge_fs::{write, write_preview, FsError, Limits};
use std::io::Write;
use tempfile::tempdir;

fn canonical_glob(dir: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(dir).unwrap();
    format!("{}/**", canonical.to_str().unwrap())
}

#[test]
fn write_creates_file_atomically_when_path_allowed() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("hello.txt");
    let allowed = vec![canonical_glob(dir.path())];

    write(
        target.to_str().unwrap(),
        "hello world",
        &allowed,
        &Limits::default(),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    assert_eq!(body, "hello world");
}

#[test]
fn write_overwrites_existing_file_atomically() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("exists.txt");
    std::fs::write(&target, "old").unwrap();
    let allowed = vec![canonical_glob(dir.path())];

    write(
        target.to_str().unwrap(),
        "new",
        &allowed,
        &Limits::default(),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    assert_eq!(body, "new");
}

#[test]
fn write_rejects_path_outside_allowed_paths_with_path_denied() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("blocked.txt");
    let allowed = vec!["/nonexistent/allow/**".to_string()];

    let err = write(
        target.to_str().unwrap(),
        "nope",
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(matches!(err, FsError::PathDenied { .. }), "got: {err:?}");
}

#[test]
fn write_rejects_symlink_parent() {
    let dir = tempdir().unwrap();
    let real = dir.path().join("real_dir");
    std::fs::create_dir(&real).unwrap();
    let link = dir.path().join("link_dir");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(not(unix))]
    return;

    let target = link.join("file.txt");
    // allow the real directory; symlinked path must still be rejected
    let allowed = vec![canonical_glob(dir.path())];

    let err = write(
        target.to_str().unwrap(),
        "payload",
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
}

#[test]
#[cfg(unix)]
fn write_rejects_symlink_in_the_middle_combined_with_parent_dir() {
    // F-356: previously, `..` inflated `path.components().len()` past the
    // canonical count, which skipped the symlink-component guard entirely and
    // silently let a user-space symlink through when mixed with `..`.
    let dir = tempdir().unwrap();
    let real = dir.path().join("real_dir");
    std::fs::create_dir_all(real.join("leaf")).unwrap();
    std::fs::create_dir_all(real.join("sibling")).unwrap();
    let link = dir.path().join("link_dir");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    // link/sibling/../leaf/file.txt — normalizes to link/leaf/file.txt, which
    // resolves to real/leaf/file.txt. The link is still in the middle.
    let target = link
        .join("sibling")
        .join("..")
        .join("leaf")
        .join("file.txt");
    let allowed = vec![canonical_glob(dir.path())];

    let err = write(
        target.to_str().unwrap(),
        "payload",
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
}

#[test]
#[cfg(unix)]
fn write_rejects_symlink_at_leaf_pointing_outside_allowlist() {
    // Leaf symlink escapes the allowlist. Either the symlink check fires at
    // the helper (same-tree rename) or `enforce_allowed` fires at the
    // public-API layer (different ancestor tree). Either rejection satisfies
    // the sandbox contract.
    let inside = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_file = outside.path().canonicalize().unwrap().join("secret.txt");
    std::fs::File::create(&outside_file).unwrap();

    let link = inside.path().canonicalize().unwrap().join("leak.txt");
    std::os::unix::fs::symlink(&outside_file, &link).unwrap();
    let allowed = vec![canonical_glob(inside.path())];

    let err = write(
        link.to_str().unwrap(),
        "payload",
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(
        matches!(
            err,
            FsError::SymlinkDenied { .. } | FsError::PathDenied { .. }
        ),
        "got: {err:?}"
    );
}

#[test]
#[cfg(unix)]
fn write_rejects_symlink_in_the_middle_pointing_outside_allowlist() {
    let inside = tempdir().unwrap();
    let outside = tempdir().unwrap();
    std::fs::create_dir_all(outside.path().join("leaf")).unwrap();

    let outside_real = outside.path().canonicalize().unwrap();
    let inside_real = inside.path().canonicalize().unwrap();
    let link = inside_real.join("escape");
    std::os::unix::fs::symlink(&outside_real, &link).unwrap();
    let allowed = vec![canonical_glob(inside.path())];

    let target = link.join("leaf").join("file.txt");
    let err = write(
        target.to_str().unwrap(),
        "payload",
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(
        matches!(
            err,
            FsError::SymlinkDenied { .. } | FsError::PathDenied { .. }
        ),
        "got: {err:?}"
    );
}

#[test]
fn write_accepts_path_with_parent_dir_inside_allowlist() {
    // F-356: `..` in a caller path must round-trip cleanly to its canonical
    // form inside the allowlist. Before the normalization fix this still
    // happened (the length mismatch silently skipped the symlink guard),
    // but the behavior is now load-bearing — pin it.
    let dir = tempdir().unwrap();
    let a_b = dir.path().join("a").join("b");
    std::fs::create_dir_all(&a_b).unwrap();
    std::fs::create_dir_all(dir.path().join("a").join("x")).unwrap();
    let target = dir
        .path()
        .join("a")
        .join("x")
        .join("..")
        .join("b")
        .join("file.txt");
    let allowed = vec![canonical_glob(dir.path())];

    write(target.to_str().unwrap(), "ok", &allowed, &Limits::default()).unwrap();

    let landed = a_b.join("file.txt");
    let body = std::fs::read_to_string(&landed).unwrap();
    assert_eq!(body, "ok");
}

#[test]
fn write_refuses_to_create_parent_directories() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("missing_parent").join("file.txt");
    let allowed = vec![canonical_glob(dir.path())];

    let err = write(target.to_str().unwrap(), "x", &allowed, &Limits::default()).unwrap_err();

    assert!(matches!(err, FsError::ParentMissing { .. }), "got: {err:?}");
}

#[test]
fn write_preview_does_not_leak_existing_file_contents() {
    // Regression for F-042 (H3): write_preview used to read the target file
    // from disk and embed its contents in the approval description, leaking
    // arbitrary files (e.g. /etc/passwd, ~/.ssh/id_rsa) onto the event bus
    // before the user could approve or reject.
    let dir = tempdir().unwrap();
    let target = dir.path().join("secret.txt");
    let secret = "SECRETMARKER-f042-do-not-leak";
    let mut f = std::fs::File::create(&target).unwrap();
    f.write_all(secret.as_bytes()).unwrap();

    let preview: String = write_preview(target.to_str().unwrap(), "replacement\n");

    assert!(
        !preview.contains(secret),
        "preview leaked existing file contents: {preview}"
    );
}

#[test]
fn write_preview_describes_path_byte_count_and_proposed_content() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("new.txt");
    let content = "line1\nline2\n";

    let preview: String = write_preview(target.to_str().unwrap(), content);

    assert!(preview.contains("new.txt"), "missing path: {preview}");
    assert!(
        preview.contains(&format!("({} bytes)", content.len())),
        "missing byte count: {preview}"
    );
    assert!(
        preview.contains(content),
        "missing proposed content: {preview}"
    );
}
