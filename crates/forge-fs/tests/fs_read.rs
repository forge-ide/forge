use forge_fs::{read_file, Limits};
use std::io::Write;
use tempfile::NamedTempFile;

fn make_temp_file(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f
}

/// Returns a glob that matches any file under the canonicalized parent of `f`.
fn parent_glob(f: &NamedTempFile) -> String {
    let canonical = std::fs::canonicalize(f.path()).unwrap();
    let parent = canonical.parent().unwrap().to_str().unwrap();
    format!("{}/**", parent)
}

#[test]
fn read_file_returns_content_bytes_sha256() {
    let f = make_temp_file("hello world");
    let path = f.path().to_str().unwrap();
    let allowed = vec![parent_glob(&f)];

    let result = read_file(path, &allowed, &Limits::default()).unwrap();

    assert_eq!(result.content, "hello world");
    assert_eq!(result.bytes, 11);
    // sha256 must be a 64-char lowercase hex string
    assert_eq!(result.sha256.len(), 64);
    assert!(result.sha256.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn read_file_sha256_differs_for_different_content() {
    let f1 = make_temp_file("hello world");
    let f2 = make_temp_file("different content");

    let r1 = read_file(
        f1.path().to_str().unwrap(),
        &[parent_glob(&f1)],
        &Limits::default(),
    )
    .unwrap();
    let r2 = read_file(
        f2.path().to_str().unwrap(),
        &[parent_glob(&f2)],
        &Limits::default(),
    )
    .unwrap();

    assert_ne!(r1.sha256, r2.sha256);
}

#[test]
fn read_file_rejects_path_not_in_allowed_globs() {
    let f = make_temp_file("secret");
    let path = f.path().to_str().unwrap();

    // No matching glob
    let allowed = vec!["/nonexistent/**".to_string()];

    let err = read_file(path, &allowed, &Limits::default()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("not allowed") || msg.contains("denied"),
        "unexpected error: {msg}"
    );
}

#[test]
fn read_file_rejects_empty_allowed_paths() {
    let f = make_temp_file("data");
    let path = f.path().to_str().unwrap();

    let err = read_file(path, &[], &Limits::default()).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("not allowed") || msg.contains("denied"),
        "unexpected error: {msg}"
    );
}

#[test]
fn read_file_errors_on_missing_file() {
    let allowed = vec!["/**".to_string()];
    let err = read_file(
        "/nonexistent/path/to/missing.txt",
        &allowed,
        &Limits::default(),
    )
    .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("No such file")
            || msg.contains("os error")
            || msg.contains("not found")
            || msg.contains("cannot resolve"),
        "unexpected error: {msg}"
    );
}

#[test]
fn read_file_allows_exact_path_glob() {
    let f = make_temp_file("exact");
    // Use the canonical path as the pattern so it matches after canonicalization.
    let canonical_path = std::fs::canonicalize(f.path()).unwrap();
    let canonical_str = canonical_path.to_str().unwrap();
    let allowed = vec![canonical_str.to_string()];

    let result = read_file(f.path().to_str().unwrap(), &allowed, &Limits::default()).unwrap();
    assert_eq!(result.content, "exact");
}

#[test]
fn read_file_blocks_path_traversal() {
    // Create a real file in a temp dir
    let f = make_temp_file("sensitive");
    let canonical_parent = std::fs::canonicalize(f.path().parent().unwrap()).unwrap();

    // Build a traversal path: /allowed/subdir/../../<tmp_dir>/file
    // (Only allow paths under "/allowed/**" — never the actual temp location)
    let allowed = vec!["/allowed/**".to_string()];

    let err = read_file(f.path().to_str().unwrap(), &allowed, &Limits::default()).unwrap_err();
    let msg = err.to_string();
    // After canonicalization, the real path (not under /allowed) must be rejected.
    assert!(
        msg.contains("not allowed") || msg.contains("denied"),
        "traversal bypass: path under {canonical_parent:?} was not rejected; error: {msg}"
    );
}
