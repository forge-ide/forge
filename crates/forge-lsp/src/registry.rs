//! Bundled language-server registry. 16 entries per the scope of F-123 /
//! docs/architecture/overview.md ("LSP: ... 16 bundled servers").
//!
//! The architecture docs do not enumerate the 16 servers. This module
//! picks widely-used, freely-distributable servers that cover the languages
//! Forge's DESIGN.md targets for Phase 1. Download URLs point to each
//! project's official release channel; **checksums are intentionally
//! [`Checksum::Pending`]** — curating pinned archive hashes (and the per-OS
//! asset matrix) is a separate release-engineering task. `Bootstrap::ensure`
//! refuses to download a server whose checksum is still `Pending`, so the
//! registry is compile-time safe today even without the pins: a caller
//! that tries to fetch the binary fails loudly instead of silently pulling
//! an unpinned archive.

use serde::{Deserialize, Serialize};

/// Stable identifier for a server in the bundled [`Registry`]. Matches the
/// `binary_name` for most servers; `&'static str` so registry lookups stay
/// allocation-free.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ServerId(pub &'static str);

impl std::fmt::Display for ServerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl From<&'static str> for ServerId {
    fn from(s: &'static str) -> Self {
        ServerId(s)
    }
}

/// Checksum policy for a [`ServerSpec`]'s downloaded archive.
///
/// `Pending` is the safe default for entries whose pinned hash hasn't been
/// curated yet — `Bootstrap::ensure` rejects downloads against a `Pending`
/// checksum with [`crate::BootstrapError::ChecksumPending`], so an unpinned
/// registry can't silently fetch an unverified binary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Checksum {
    /// SHA-256 hex digest expected on the downloaded archive. 64 lowercase
    /// hex chars, verified against the download body bytes.
    Sha256(String),
    /// No pinned digest yet. Release engineering will promote these to
    /// `Sha256` as each server's archive URL is curated per-OS.
    Pending,
}

/// A single bundled language-server entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSpec {
    /// Stable identifier used as the cache-dir name and as the `server`
    /// argument to `lsp_start` / `lsp_send` / `lsp_stop`.
    pub id: ServerId,
    /// Human-readable language identifier. Matches the Monaco languageId
    /// the iframe uses so the frontend can look up the right server.
    pub language_id: &'static str,
    /// Filename of the executable inside the cache dir (after extraction).
    pub binary_name: &'static str,
    /// Canonical download URL for the server's release archive.
    pub download_url: &'static str,
    /// Expected checksum of the downloaded archive. [`Checksum::Pending`]
    /// defers to release engineering; [`crate::Bootstrap::ensure`] refuses
    /// to install an unpinned server.
    pub checksum: Checksum,
}

/// Static registry of bundled servers.
#[derive(Debug, Clone)]
pub struct Registry {
    entries: &'static [ServerSpec],
}

impl Registry {
    /// The 16 bundled servers.
    pub fn bundled() -> Self {
        Self {
            entries: BUNDLED_SERVERS,
        }
    }

    /// All registered specs in declaration order.
    pub fn all(&self) -> &'static [ServerSpec] {
        self.entries
    }

    /// Look up a spec by [`ServerId`].
    pub fn get(&self, id: ServerId) -> Option<&'static ServerSpec> {
        self.entries.iter().find(|s| s.id == id)
    }

    /// Look up a spec by Monaco language id (e.g. `"rust"`, `"typescript"`).
    /// Returns the first match; multiple servers may claim a language but
    /// the bundled set is currently one-per-language.
    pub fn by_language(&self, language_id: &str) -> Option<&'static ServerSpec> {
        self.entries.iter().find(|s| s.language_id == language_id)
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::bundled()
    }
}

/// The 16 bundled server specs. Ordering is stable and part of the test
/// surface (tests assert `Registry::bundled().all().len() == 16`).
///
/// URLs target each project's upstream release; checksums will be pinned
/// per-OS in a follow-up release-engineering task. `Bootstrap::ensure`
/// guards against `Pending` at install time, so this file can land today
/// without risk of an unpinned download.
const BUNDLED_SERVERS: &[ServerSpec] = &[
    ServerSpec {
        id: ServerId("rust-analyzer"),
        language_id: "rust",
        binary_name: "rust-analyzer",
        download_url: "https://github.com/rust-lang/rust-analyzer/releases/latest",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("typescript-language-server"),
        language_id: "typescript",
        binary_name: "typescript-language-server",
        download_url: "https://registry.npmjs.org/typescript-language-server",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("pyright"),
        language_id: "python",
        binary_name: "pyright-langserver",
        download_url: "https://registry.npmjs.org/pyright",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("gopls"),
        language_id: "go",
        binary_name: "gopls",
        download_url: "https://go.googlesource.com/tools/+/refs/tags/gopls",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("clangd"),
        language_id: "cpp",
        binary_name: "clangd",
        download_url: "https://github.com/clangd/clangd/releases/latest",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("jdtls"),
        language_id: "java",
        binary_name: "jdtls",
        download_url: "https://download.eclipse.org/jdtls/milestones/",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("vscode-json-languageserver"),
        language_id: "json",
        binary_name: "vscode-json-languageserver",
        download_url: "https://registry.npmjs.org/vscode-json-languageserver",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("vscode-html-languageserver"),
        language_id: "html",
        binary_name: "vscode-html-languageserver",
        download_url: "https://registry.npmjs.org/vscode-html-languageserver-bin",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("vscode-css-languageserver"),
        language_id: "css",
        binary_name: "vscode-css-languageserver",
        download_url: "https://registry.npmjs.org/vscode-css-languageserver-bin",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("yaml-language-server"),
        language_id: "yaml",
        binary_name: "yaml-language-server",
        download_url: "https://registry.npmjs.org/yaml-language-server",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("bash-language-server"),
        language_id: "shellscript",
        binary_name: "bash-language-server",
        download_url: "https://registry.npmjs.org/bash-language-server",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("lua-language-server"),
        language_id: "lua",
        binary_name: "lua-language-server",
        download_url: "https://github.com/LuaLS/lua-language-server/releases/latest",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("solargraph"),
        language_id: "ruby",
        binary_name: "solargraph",
        download_url: "https://rubygems.org/gems/solargraph",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("intelephense"),
        language_id: "php",
        binary_name: "intelephense",
        download_url: "https://registry.npmjs.org/intelephense",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("elixir-ls"),
        language_id: "elixir",
        binary_name: "elixir-ls",
        download_url: "https://github.com/elixir-lsp/elixir-ls/releases/latest",
        checksum: Checksum::Pending,
    },
    ServerSpec {
        id: ServerId("haskell-language-server"),
        language_id: "haskell",
        binary_name: "haskell-language-server",
        download_url: "https://github.com/haskell/haskell-language-server/releases/latest",
        checksum: Checksum::Pending,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_registry_has_exactly_sixteen_entries() {
        // DoD: "`forge-lsp::Registry` with 16 bundled server definitions"
        let r = Registry::bundled();
        assert_eq!(r.all().len(), 16, "bundled registry must list 16 servers");
    }

    #[test]
    fn every_bundled_entry_has_non_empty_language_binary_and_url() {
        // Each entry needs a language, binary, and URL to be usable. A
        // zero-length field would make `by_language` ambiguous and
        // `Bootstrap::ensure` would attempt an empty download.
        for spec in Registry::bundled().all() {
            assert!(!spec.id.0.is_empty(), "empty id");
            assert!(
                !spec.language_id.is_empty(),
                "{} has empty language_id",
                spec.id
            );
            assert!(
                !spec.binary_name.is_empty(),
                "{} has empty binary_name",
                spec.id
            );
            assert!(
                !spec.download_url.is_empty(),
                "{} has empty download_url",
                spec.id
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        // Dup ids would make `get(id)` ambiguous — the registry lookup
        // surface relies on one entry per id.
        let ids: std::collections::HashSet<_> =
            Registry::bundled().all().iter().map(|s| s.id).collect();
        assert_eq!(
            ids.len(),
            Registry::bundled().all().len(),
            "duplicate ServerId in bundled registry"
        );
    }

    #[test]
    fn get_by_id_returns_the_matching_spec() {
        let r = Registry::bundled();
        let spec = r
            .get(ServerId("rust-analyzer"))
            .expect("rust-analyzer present");
        assert_eq!(spec.language_id, "rust");
    }

    #[test]
    fn by_language_returns_the_matching_spec() {
        let r = Registry::bundled();
        let spec = r.by_language("rust").expect("rust registered");
        assert_eq!(spec.id, ServerId("rust-analyzer"));
    }

    #[test]
    fn checksums_default_to_pending() {
        // Every bundled entry starts at `Pending` — see module docs. This
        // test pins that choice so a future PR that adds a real Sha256 has
        // to update the test too, keeping the divergence visible.
        for spec in Registry::bundled().all() {
            assert!(
                matches!(spec.checksum, Checksum::Pending),
                "{}: expected Pending checksum, got {:?}",
                spec.id,
                spec.checksum
            );
        }
    }
}
