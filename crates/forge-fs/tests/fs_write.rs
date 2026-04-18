use forge_fs::{write, write_preview, FsError};
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

    write(target.to_str().unwrap(), "hello world", &allowed).unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    assert_eq!(body, "hello world");
}

#[test]
fn write_overwrites_existing_file_atomically() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("exists.txt");
    std::fs::write(&target, "old").unwrap();
    let allowed = vec![canonical_glob(dir.path())];

    write(target.to_str().unwrap(), "new", &allowed).unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    assert_eq!(body, "new");
}

#[test]
fn write_rejects_path_outside_allowed_paths_with_path_denied() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("blocked.txt");
    let allowed = vec!["/nonexistent/allow/**".to_string()];

    let err = write(target.to_str().unwrap(), "nope", &allowed).unwrap_err();

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

    let err = write(target.to_str().unwrap(), "payload", &allowed).unwrap_err();

    assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
}

#[test]
fn write_refuses_to_create_parent_directories() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("missing_parent").join("file.txt");
    let allowed = vec![canonical_glob(dir.path())];

    let err = write(target.to_str().unwrap(), "x", &allowed).unwrap_err();

    assert!(matches!(err, FsError::ParentMissing { .. }), "got: {err:?}");
}

#[test]
fn write_preview_returns_unified_diff_for_new_file() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("new.txt");

    let preview = write_preview(target.to_str().unwrap(), "line1\nline2\n");

    assert!(
        preview.description.contains("new.txt"),
        "description missing path: {}",
        preview.description
    );
    assert!(
        preview.description.contains("+line1"),
        "expected added line in diff: {}",
        preview.description
    );
}

#[test]
fn write_preview_returns_unified_diff_for_overwrite() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("existing.txt");
    let mut f = std::fs::File::create(&target).unwrap();
    f.write_all(b"old_line\n").unwrap();

    let preview = write_preview(target.to_str().unwrap(), "new_line\n");

    assert!(
        preview.description.contains("-old_line"),
        "expected removed line: {}",
        preview.description
    );
    assert!(
        preview.description.contains("+new_line"),
        "expected added line: {}",
        preview.description
    );
}
