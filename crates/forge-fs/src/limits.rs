//! Byte-size caps for filesystem operations. Enforcement lives in `forge-fs`
//! so callers cannot forget to apply it; see F-061 / M3 audit finding.

/// Maximum bytes to read or write in a single `forge-fs` operation.
///
/// Defaults to 10 MiB for each direction. Bump via a custom `Limits` if your
/// workload is known-bounded; tighten it in tests to exercise the cap without
/// generating large fixtures.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_read_bytes: u64,
    pub max_write_bytes: u64,
}

const DEFAULT_LIMIT: u64 = 10 * 1024 * 1024;

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_read_bytes: DEFAULT_LIMIT,
            max_write_bytes: DEFAULT_LIMIT,
        }
    }
}
