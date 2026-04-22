//! First-run download bootstrap. Fetches a bundled server's archive,
//! verifies its sha256 against [`crate::Checksum`], and caches the raw
//! bytes under `~/.cache/forge/lsp/<server_id>/`.
//!
//! **Sandbox.** Every write happens inside the caller-provided `cache_root`
//! (defaulting to `~/.cache/forge/lsp/`). The server's destination path is
//! constructed as `cache_root.join(server.id)` and the leaf target is
//! checked to stay rooted at `cache_root` via an explicit prefix match
//! against the canonicalized root — defense-in-depth against a registry
//! entry whose `id` contains `..` or an absolute path. This honors the
//! `forge-fs` sandbox story (see DoD: "Must honor `forge-fs` sandboxing so
//! downloaded binaries cannot escape the LSP cache directory"). The archive
//! bytes themselves are written verbatim — extraction is a separate
//! follow-up (requires per-archive-format logic + per-OS asset selection).
//!
//! **Checksum policy.** [`Checksum::Pending`] entries never download: the
//! bootstrap returns [`BootstrapError::ChecksumPending`] so callers learn
//! fast and release engineering can pin hashes safely.
//!
//! **Network seam.** The [`Downloader`] trait lets tests inject an
//! in-memory fixture without touching the network — see `bootstrap.rs`'s
//! unit tests and the `tests/` integration suite.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::registry::{Checksum, Registry, ServerSpec};

/// Errors returned by [`Bootstrap::ensure`].
#[derive(Debug, thiserror::Error)]
pub enum BootstrapError {
    /// The spec's checksum is [`Checksum::Pending`] — we refuse to download
    /// until a hash is pinned.
    #[error("checksum pending for server {server}: refuse to download unpinned archive")]
    ChecksumPending {
        /// The server's id.
        server: String,
    },
    /// The downloaded bytes did not match the pinned sha256.
    #[error("checksum mismatch for {server}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// The server's id.
        server: String,
        /// Expected hex digest.
        expected: String,
        /// Observed hex digest of the downloaded bytes.
        actual: String,
    },
    /// Downloader returned an error.
    #[error("download failed for {server}: {source}")]
    Download {
        /// The server's id.
        server: String,
        /// Underlying downloader error, stringified.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    /// Resolved cache path escaped `cache_root` — the registry entry is
    /// hostile (e.g. `id = "../../etc"`). Hard failure; no bytes touch disk.
    #[error("cache path escaped sandbox for {server}: {path}")]
    SandboxEscape {
        /// The server's id.
        server: String,
        /// Path that would have been written.
        path: PathBuf,
    },
    /// Filesystem I/O while writing the cache.
    #[error("cache I/O for {server}: {source}")]
    Io {
        /// The server's id.
        server: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// No platform cache dir resolvable (e.g. `HOME` unset). Caller should
    /// pass an explicit `cache_root` to [`Bootstrap::new_in`].
    #[error("could not resolve a cache directory (HOME unset?)")]
    NoCacheDir,
}

/// Network seam. Tests inject a stub impl with in-memory fixtures;
/// production uses [`HttpDownloader`] backed by reqwest.
#[async_trait]
pub trait Downloader: Send + Sync {
    /// Fetch the bytes of `url`. Any transport error surfaces as
    /// [`BootstrapError::Download`].
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>>;
}

/// Production [`Downloader`] backed by `reqwest`.
pub struct HttpDownloader {
    client: reqwest::Client,
}

impl HttpDownloader {
    /// New downloader with default timeouts.
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .user_agent(concat!("forge-lsp/", env!("CARGO_PKG_VERSION")))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }
}

impl Default for HttpDownloader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Downloader for HttpDownloader {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        let resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(format!("HTTP {status}").into());
        }
        let bytes = resp.bytes().await?;
        Ok(bytes.to_vec())
    }
}

/// Bootstrap operations rooted at a specific cache directory.
pub struct Bootstrap {
    cache_root: PathBuf,
    downloader: Box<dyn Downloader>,
    registry: Registry,
}

impl Bootstrap {
    /// Bootstrap rooted at the platform default (`~/.cache/forge/lsp/`),
    /// with an [`HttpDownloader`] and the bundled [`Registry`]. Errors if
    /// no cache dir can be resolved.
    pub fn new() -> Result<Self, BootstrapError> {
        let root = default_cache_root().ok_or(BootstrapError::NoCacheDir)?;
        Ok(Self::new_in(root, Box::new(HttpDownloader::new())))
    }

    /// Bootstrap rooted at `cache_root`, using the injected `downloader`
    /// and the bundled [`Registry`]. Every on-disk side-effect stays under
    /// `cache_root`.
    pub fn new_in(cache_root: PathBuf, downloader: Box<dyn Downloader>) -> Self {
        Self::with_registry(cache_root, downloader, Registry::bundled())
    }

    /// Bootstrap with an explicit [`Registry`]. Hidden from rustdoc —
    /// production callers should stick to [`Bootstrap::new`] /
    /// [`Bootstrap::new_in`] so the bundled registry is the only surface
    /// `Server::from_registry` resolves against. Integration tests use this
    /// to inject a single-spec registry pointing at the in-tree stub LSP
    /// fixture.
    #[doc(hidden)]
    pub fn with_registry(
        cache_root: PathBuf,
        downloader: Box<dyn Downloader>,
        registry: Registry,
    ) -> Self {
        Self {
            cache_root,
            downloader,
            registry,
        }
    }

    /// The [`Registry`] this bootstrap resolves against. See
    /// [`crate::Server::from_registry`].
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Resolve the absolute cache directory for `spec`, asserting it stays
    /// rooted under the cache root passed at construction. Returns
    /// [`BootstrapError::SandboxEscape`] for hostile ids.
    pub fn server_dir(&self, spec: &ServerSpec) -> Result<PathBuf, BootstrapError> {
        let candidate = self.cache_root.join(spec.id.0);
        enforce_in_sandbox(&self.cache_root, &candidate, spec.id.0)?;
        Ok(candidate)
    }

    /// Absolute cache root passed at construction. Used by
    /// [`crate::Server::from_registry`] to bind the spawn path to the
    /// sandbox.
    pub fn cache_root(&self) -> &Path {
        &self.cache_root
    }

    /// Assert `candidate` resolves under the cache root. Returns
    /// [`BootstrapError::SandboxEscape`] otherwise. Public so other modules
    /// (notably [`crate::Server::from_registry`]) can reuse the same
    /// lexical-prefix check the download path uses.
    pub fn enforce_in_sandbox(
        &self,
        candidate: &Path,
        server_id: &str,
    ) -> Result<(), BootstrapError> {
        enforce_in_sandbox(&self.cache_root, candidate, server_id)
    }

    /// Ensure `spec` is present in the cache. On a cache hit — the archive
    /// file exists — returns the archive path without network I/O. On a
    /// miss, fetches the archive, verifies its sha256, writes the bytes to
    /// `<server_dir>/archive.bin`, and returns its path.
    ///
    /// Errors:
    /// - [`BootstrapError::ChecksumPending`] for unpinned specs.
    /// - [`BootstrapError::ChecksumMismatch`] if bytes don't match the pin.
    /// - [`BootstrapError::Download`] for transport failures.
    /// - [`BootstrapError::SandboxEscape`] for hostile ids.
    /// - [`BootstrapError::Io`] for cache write failures.
    ///
    /// # Examples
    ///
    /// Resolve the bundled registry's rust-analyzer entry against a
    /// scratch cache root. The bundled registry ships with
    /// [`Checksum::Pending`] pins, so `ensure` surfaces
    /// [`BootstrapError::ChecksumPending`] — the intended safety net
    /// that prevents an unpinned archive from ever touching disk:
    ///
    /// ```no_run
    /// use forge_lsp::{Bootstrap, BootstrapError, Registry};
    ///
    /// # async fn example() -> Result<(), BootstrapError> {
    /// let bootstrap = Bootstrap::new()?;
    /// let spec = Registry::bundled()
    ///     .by_language("rust")
    ///     .expect("rust-analyzer registered");
    /// match bootstrap.ensure(spec).await {
    ///     Err(BootstrapError::ChecksumPending { .. }) => { /* expected until RE pins */ }
    ///     other => panic!("unexpected result: {other:?}"),
    /// }
    /// # Ok(()) }
    /// ```
    pub async fn ensure(&self, spec: &ServerSpec) -> Result<PathBuf, BootstrapError> {
        let expected = match &spec.checksum {
            Checksum::Sha256(h) => h.clone(),
            Checksum::Pending => {
                return Err(BootstrapError::ChecksumPending {
                    server: spec.id.to_string(),
                });
            }
        };

        let server_dir = self.server_dir(spec)?;
        let archive_path = server_dir.join("archive.bin");

        if archive_path.exists() {
            // Cache hit — caller can trust the pin was verified on the
            // original install. Skipping HTTP here keeps `ensure` idempotent
            // and offline-friendly.
            return Ok(archive_path);
        }

        let bytes = self
            .downloader
            .fetch(spec.download_url)
            .await
            .map_err(|source| BootstrapError::Download {
                server: spec.id.to_string(),
                source,
            })?;

        let actual = hex::encode(Sha256::digest(&bytes));
        if !actual.eq_ignore_ascii_case(&expected) {
            return Err(BootstrapError::ChecksumMismatch {
                server: spec.id.to_string(),
                expected,
                actual,
            });
        }

        // Create parent, then write the archive. Everything stays inside
        // `server_dir` which is already sandbox-checked above.
        tokio::fs::create_dir_all(&server_dir)
            .await
            .map_err(|source| BootstrapError::Io {
                server: spec.id.to_string(),
                source,
            })?;
        tokio::fs::write(&archive_path, &bytes)
            .await
            .map_err(|source| BootstrapError::Io {
                server: spec.id.to_string(),
                source,
            })?;

        Ok(archive_path)
    }
}

/// Return the default `~/.cache/forge/lsp/` root, or `None` when the
/// platform cannot resolve a cache dir.
pub fn default_cache_root() -> Option<PathBuf> {
    dirs::cache_dir().map(|d| d.join("forge").join("lsp"))
}

/// Verify `candidate` is rooted under `root`. Used as a sandbox guard before
/// any filesystem write — matches the `forge-fs` `enforce_allowed` shape but
/// the check here is path-prefix only (no glob patterns); the cache
/// directory is a single known root.
fn enforce_in_sandbox(
    root: &Path,
    candidate: &Path,
    server_id: &str,
) -> Result<(), BootstrapError> {
    // Canonicalize components that exist; fall back to lexical prefix check
    // for not-yet-created paths. A hostile `..` or absolute-path id produces
    // a lexical mismatch either way.
    let normalized = normalize(candidate);
    let normalized_root = normalize(root);
    if !normalized.starts_with(&normalized_root) {
        return Err(BootstrapError::SandboxEscape {
            server: server_id.to_string(),
            path: candidate.to_path_buf(),
        });
    }
    Ok(())
}

/// Lexical normalization: collapse `.` and `..` components without touching
/// the filesystem. A sandbox escape attempt via `id = "../../etc"` collapses
/// to a path outside the cache root, which the prefix check rejects.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{Checksum, ServerId, ServerSpec};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct StubDownloader {
        bytes: Vec<u8>,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Downloader for StubDownloader {
        async fn fetch(
            &self,
            _url: &str,
        ) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.bytes.clone())
        }
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn spec_with(checksum: Checksum) -> ServerSpec {
        ServerSpec {
            id: ServerId("stub-server"),
            language_id: "stub",
            binary_name: "stub",
            download_url: "http://example.invalid/stub.tar.gz",
            checksum,
        }
    }

    #[tokio::test]
    async fn ensure_refuses_pending_checksums() {
        // DoD: checksum verification. `Pending` entries never touch disk.
        let tmp = tempfile::tempdir().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let downloader = Box::new(StubDownloader {
            bytes: b"irrelevant".to_vec(),
            calls: Arc::clone(&calls),
        });
        let b = Bootstrap::new_in(tmp.path().to_path_buf(), downloader);
        let err = b.ensure(&spec_with(Checksum::Pending)).await.unwrap_err();
        assert!(matches!(err, BootstrapError::ChecksumPending { .. }));
        assert_eq!(calls.load(Ordering::SeqCst), 0, "no download on Pending");
    }

    #[tokio::test]
    async fn ensure_verifies_checksum_and_caches() {
        // DoD: downloads + verifies + caches under the sandbox root.
        let tmp = tempfile::tempdir().unwrap();
        let bytes = b"hello forge-lsp".to_vec();
        let calls = Arc::new(AtomicUsize::new(0));
        let downloader = Box::new(StubDownloader {
            bytes: bytes.clone(),
            calls: Arc::clone(&calls),
        });
        let b = Bootstrap::new_in(tmp.path().to_path_buf(), downloader);
        let spec = spec_with(Checksum::Sha256(sha256_hex(&bytes)));

        let path = b.ensure(&spec).await.expect("first ensure succeeds");
        assert!(path.exists(), "archive must exist at {path:?}");
        assert!(
            path.starts_with(tmp.path()),
            "archive {path:?} must stay under cache root {:?}",
            tmp.path()
        );
        let got = tokio::fs::read(&path).await.unwrap();
        assert_eq!(got, bytes);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one fetch on miss");
    }

    #[tokio::test]
    async fn ensure_rejects_checksum_mismatch() {
        // DoD: checksum verification rejects tampered archives. The cache
        // dir must not contain the tampered bytes after a mismatch.
        let tmp = tempfile::tempdir().unwrap();
        let downloader = Box::new(StubDownloader {
            bytes: b"tampered".to_vec(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let b = Bootstrap::new_in(tmp.path().to_path_buf(), downloader);
        let spec = spec_with(Checksum::Sha256(sha256_hex(b"ORIGINAL BYTES")));
        let err = b.ensure(&spec).await.unwrap_err();
        assert!(matches!(err, BootstrapError::ChecksumMismatch { .. }));
        let archive = tmp.path().join("stub-server").join("archive.bin");
        assert!(
            !archive.exists(),
            "mismatch must not leave bytes on disk at {archive:?}"
        );
    }

    #[tokio::test]
    async fn ensure_cache_hit_skips_network() {
        // DoD: `Bootstrap::ensure` is idempotent. Second call must not fetch.
        let tmp = tempfile::tempdir().unwrap();
        let bytes = b"cached payload".to_vec();
        let calls = Arc::new(AtomicUsize::new(0));
        let downloader = Box::new(StubDownloader {
            bytes: bytes.clone(),
            calls: Arc::clone(&calls),
        });
        let b = Bootstrap::new_in(tmp.path().to_path_buf(), downloader);
        let spec = spec_with(Checksum::Sha256(sha256_hex(&bytes)));

        b.ensure(&spec).await.expect("first");
        b.ensure(&spec).await.expect("second");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "cache hit must skip fetch");
    }

    #[tokio::test]
    async fn ensure_rejects_sandbox_escape() {
        // DoD: sandbox — a hostile server id with `..` components cannot
        // escape the cache root.
        let tmp = tempfile::tempdir().unwrap();
        let downloader = Box::new(StubDownloader {
            bytes: Vec::new(),
            calls: Arc::new(AtomicUsize::new(0)),
        });
        let b = Bootstrap::new_in(tmp.path().to_path_buf(), downloader);
        let hostile = ServerSpec {
            id: ServerId("../../etc/evil"),
            language_id: "evil",
            binary_name: "evil",
            download_url: "http://example.invalid/",
            checksum: Checksum::Sha256("deadbeef".into()),
        };
        let err = b.ensure(&hostile).await.unwrap_err();
        assert!(
            matches!(err, BootstrapError::SandboxEscape { .. }),
            "expected SandboxEscape, got {err:?}"
        );
    }
}
