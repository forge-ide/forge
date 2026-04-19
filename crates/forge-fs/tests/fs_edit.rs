use forge_fs::{edit, edit_preview, FsError, Limits};
use std::io::Write;
use tempfile::tempdir;

fn canonical_glob(dir: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(dir).unwrap();
    format!("{}/**", canonical.to_str().unwrap())
}

fn unified_diff(old: &str, new: &str) -> String {
    similar::TextDiff::from_lines(old, new)
        .unified_diff()
        .to_string()
}

#[test]
fn edit_applies_unified_diff_patch() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("src.txt");
    let original = "alpha\nbeta\ngamma\n";
    std::fs::write(&target, original).unwrap();
    let allowed = vec![canonical_glob(dir.path())];
    let updated = "alpha\nBETA\ngamma\n";
    let patch = unified_diff(original, updated);

    edit(
        target.to_str().unwrap(),
        &patch,
        &allowed,
        &Limits::default(),
    )
    .unwrap();

    let body = std::fs::read_to_string(&target).unwrap();
    assert_eq!(body, updated);
}

#[test]
fn edit_rejects_malformed_patch() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("src.txt");
    std::fs::write(&target, "alpha\nbeta\n").unwrap();
    let allowed = vec![canonical_glob(dir.path())];
    let bogus = "this is not a unified diff at all\n";

    let err = edit(
        target.to_str().unwrap(),
        bogus,
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(
        matches!(err, FsError::MalformedPatch { .. }),
        "got: {err:?}"
    );
}

#[test]
fn edit_rejects_path_outside_allowed_paths_with_path_denied() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("src.txt");
    std::fs::write(&target, "a\n").unwrap();
    let allowed = vec!["/nonexistent/allow/**".to_string()];
    let patch = unified_diff("a\n", "b\n");

    let err = edit(
        target.to_str().unwrap(),
        &patch,
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(matches!(err, FsError::PathDenied { .. }), "got: {err:?}");
}

#[test]
fn edit_rejects_symlink_target() {
    let dir = tempdir().unwrap();
    let real = dir.path().join("real.txt");
    std::fs::write(&real, "a\n").unwrap();
    let link = dir.path().join("link.txt");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real, &link).unwrap();
    #[cfg(not(unix))]
    return;

    let allowed = vec![canonical_glob(dir.path())];
    let patch = unified_diff("a\n", "b\n");

    let err = edit(link.to_str().unwrap(), &patch, &allowed, &Limits::default()).unwrap_err();

    assert!(matches!(err, FsError::SymlinkDenied { .. }), "got: {err:?}");
}

#[test]
fn edit_requires_target_file_to_exist() {
    let dir = tempdir().unwrap();
    let missing = dir.path().join("nope.txt");
    let allowed = vec![canonical_glob(dir.path())];
    let patch = unified_diff("a\n", "b\n");

    let err = edit(
        missing.to_str().unwrap(),
        &patch,
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();

    assert!(matches!(err, FsError::TargetMissing { .. }), "got: {err:?}");
}

#[test]
fn edit_preview_returns_unified_diff_description() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("src.txt");
    let mut f = std::fs::File::create(&target).unwrap();
    f.write_all(b"alpha\nbeta\ngamma\n").unwrap();
    let patch = unified_diff("alpha\nbeta\ngamma\n", "alpha\nBETA\ngamma\n");

    let preview = edit_preview(target.to_str().unwrap(), &patch);

    assert!(
        preview.description.contains("src.txt"),
        "description missing path: {}",
        preview.description
    );
    assert!(
        preview.description.contains("-beta"),
        "expected removed line: {}",
        preview.description
    );
    assert!(
        preview.description.contains("+BETA"),
        "expected added line: {}",
        preview.description
    );
}
