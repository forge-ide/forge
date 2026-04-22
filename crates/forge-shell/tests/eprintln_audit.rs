//! F-371: shipped forge-shell code must not `eprintln!`.
//!
//! Source-level audit so a post-F-371 regression (someone reintroduces
//! ad-hoc stderr in `ipc.rs`, `bridge.rs`, etc.) fails CI instead of
//! silently routing around the structured-logging contract. `#[cfg(test)]
//! mod …` blocks are excluded.

use std::fs;
use std::path::PathBuf;

#[test]
fn no_eprintln_in_shipped_shell_sources() {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let offenders = collect_eprintln_offenders(&src_dir);
    assert!(
        offenders.is_empty(),
        "eprintln! found in shipped forge-shell surface:\n{}",
        offenders.join("\n")
    );
}

/// `main.rs` hosts a single no-webview `std::process::exit(1)` banner that
/// fires before a `tracing` subscriber could ever run; keep it excluded so
/// the audit targets the runtime surface, not fail-fast CLI bootstrap.
fn is_bin_main(path: &std::path::Path) -> bool {
    path.file_name().and_then(|n| n.to_str()) == Some("main.rs")
}

fn collect_eprintln_offenders(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let entries = fs::read_dir(dir).expect("read src dir");
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_eprintln_offenders(&path));
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        if is_bin_main(&path) {
            continue;
        }
        let src = fs::read_to_string(&path).expect("read rs file");
        let mut in_cfg_test = false;
        let mut depth: i32 = 0;
        for (idx, line) in src.lines().enumerate() {
            let trimmed = line.trim_start();
            if !in_cfg_test
                && (trimmed.starts_with("#[cfg(test)]")
                    || (trimmed.starts_with("#[cfg(all(") && trimmed.contains("test")))
            {
                in_cfg_test = true;
                depth = 0;
                continue;
            }
            if in_cfg_test {
                let opens = line.matches('{').count() as i32;
                let closes = line.matches('}').count() as i32;
                depth += opens;
                depth -= closes;
                if depth <= 0 && opens > 0 {
                    in_cfg_test = false;
                }
                continue;
            }
            if line.contains("eprintln!") {
                out.push(format!("{}:{}: {}", path.display(), idx + 1, line.trim()));
            }
        }
    }
    out
}
