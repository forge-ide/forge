use anyhow::{bail, Context, Result};
use glob::Pattern;
use sha2::{Digest, Sha256};
use std::path::Path;

/// Result of a successful `fs.read` operation.
#[derive(Debug)]
pub struct ReadResult {
    pub content: String,
    pub bytes: usize,
    pub sha256: String,
}

/// Read a file at `path`, validating it against the `allowed_paths` glob patterns.
///
/// The path is canonicalized before glob matching to prevent `..` traversal attacks.
/// Returns `Err` if no pattern matches or the file cannot be read.
pub fn read_file(path: &str, allowed_paths: &[String]) -> Result<ReadResult> {
    // Canonicalize first so `..` components can't bypass glob patterns.
    let canonical =
        std::fs::canonicalize(path).with_context(|| format!("cannot resolve path '{path}'"))?;
    validate_path(&canonical, allowed_paths)?;
    let raw = std::fs::read(&canonical)?;
    let bytes = raw.len();
    let content = String::from_utf8_lossy(&raw).into_owned();
    let sha256 = hex::encode(Sha256::digest(&raw));
    Ok(ReadResult {
        content,
        bytes,
        sha256,
    })
}

fn validate_path(path: &Path, allowed_paths: &[String]) -> Result<()> {
    for pattern in allowed_paths {
        if let Ok(pat) = Pattern::new(pattern) {
            if pat.matches_path(path) {
                return Ok(());
            }
        }
    }
    bail!("path '{}' is not allowed by allowed_paths", path.display());
}
