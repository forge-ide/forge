//! Regression tests for F-061 / M3: byte-size caps on forge-fs read/write/edit.
//! Uses small explicit `Limits` so tests stay fast and don't write 10 MiB files.

use forge_fs::{edit, read_file, write, FsError, Limits};
use std::io::Write;
use tempfile::{tempdir, NamedTempFile};

fn make_temp_file(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().unwrap();
    f.write_all(content.as_bytes()).unwrap();
    f
}

fn parent_glob(f: &NamedTempFile) -> String {
    let canonical = std::fs::canonicalize(f.path()).unwrap();
    let parent = canonical.parent().unwrap().to_str().unwrap();
    format!("{}/**", parent)
}

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
fn read_file_rejects_file_larger_than_cap_with_typed_error() {
    let f = make_temp_file(&"x".repeat(1024));
    let allowed = vec![parent_glob(&f)];
    let limits = Limits {
        max_read_bytes: 64,
        max_write_bytes: 64,
    };

    let err = read_file(f.path().to_str().unwrap(), &allowed, &limits).unwrap_err();

    match err {
        FsError::TooLarge { actual, limit, .. } => {
            assert_eq!(actual, 1024);
            assert_eq!(limit, 64);
        }
        other => panic!("expected FsError::TooLarge, got {other:?}"),
    }
}

#[test]
fn read_file_accepts_file_at_or_below_cap() {
    let f = make_temp_file("small");
    let allowed = vec![parent_glob(&f)];
    let limits = Limits {
        max_read_bytes: 64,
        max_write_bytes: 64,
    };

    let result = read_file(f.path().to_str().unwrap(), &allowed, &limits).unwrap();
    assert_eq!(result.content, "small");
}

#[test]
fn read_file_default_limits_accept_small_files() {
    let f = make_temp_file("hello");
    let allowed = vec![parent_glob(&f)];

    let result = read_file(f.path().to_str().unwrap(), &allowed, &Limits::default()).unwrap();
    assert_eq!(result.content, "hello");
}

#[test]
fn write_rejects_content_larger_than_cap_with_typed_error() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("out.txt");
    let allowed = vec![canonical_glob(dir.path())];
    let limits = Limits {
        max_read_bytes: 64,
        max_write_bytes: 64,
    };
    let payload = "y".repeat(1024);

    let err = write(target.to_str().unwrap(), &payload, &allowed, &limits).unwrap_err();

    assert!(
        matches!(
            err,
            FsError::TooLarge {
                actual: 1024,
                limit: 64,
                ..
            }
        ),
        "got: {err:?}"
    );
    assert!(!target.exists(), "oversized write must not touch disk");
}

#[test]
fn write_accepts_content_at_or_below_cap() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("out.txt");
    let allowed = vec![canonical_glob(dir.path())];
    let limits = Limits {
        max_read_bytes: 64,
        max_write_bytes: 64,
    };

    write(target.to_str().unwrap(), "small", &allowed, &limits).unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "small");
}

#[test]
fn edit_rejects_source_file_larger_than_read_cap() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("src.txt");
    std::fs::write(&target, "x".repeat(1024)).unwrap();
    let allowed = vec![canonical_glob(dir.path())];
    let limits = Limits {
        max_read_bytes: 64,
        max_write_bytes: 10 * 1024 * 1024,
    };
    let patch = "@@ -1,1 +1,1 @@\n-x\n+y\n";

    let err = edit(target.to_str().unwrap(), patch, &allowed, &limits).unwrap_err();

    assert!(
        matches!(
            err,
            FsError::TooLarge {
                actual: 1024,
                limit: 64,
                ..
            }
        ),
        "got: {err:?}"
    );
}

#[test]
fn edit_rejects_post_patch_output_larger_than_write_cap() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("src.txt");
    let original = "alpha\n";
    std::fs::write(&target, original).unwrap();
    let allowed = vec![canonical_glob(dir.path())];
    // Read cap is large so we get past stat; write cap is tiny so post-patch fails.
    let limits = Limits {
        max_read_bytes: 10 * 1024 * 1024,
        max_write_bytes: 4,
    };
    // Diff that expands one short line into a much larger line.
    let expanded = "alpha plus much more content here\n";
    let patch = unified_diff(original, expanded);

    let err = edit(target.to_str().unwrap(), &patch, &allowed, &limits).unwrap_err();

    assert!(matches!(err, FsError::TooLarge { .. }), "got: {err:?}");
    // File must be unchanged.
    assert_eq!(std::fs::read_to_string(&target).unwrap(), original);
}

#[test]
fn edit_accepts_when_both_caps_honored() {
    let dir = tempdir().unwrap();
    let target = dir.path().join("src.txt");
    let original = "alpha\nbeta\ngamma\n";
    std::fs::write(&target, original).unwrap();
    let allowed = vec![canonical_glob(dir.path())];
    let limits = Limits {
        max_read_bytes: 1024,
        max_write_bytes: 1024,
    };
    let updated = "alpha\nBETA\ngamma\n";
    let patch = unified_diff(original, updated);

    edit(target.to_str().unwrap(), &patch, &allowed, &limits).unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), updated);
}

#[test]
fn limits_default_is_ten_mebibytes_each() {
    let d = Limits::default();
    assert_eq!(d.max_read_bytes, 10 * 1024 * 1024);
    assert_eq!(d.max_write_bytes, 10 * 1024 * 1024);
}
