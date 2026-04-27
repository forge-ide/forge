//! F-601: cross-session, per-agent memory.
//!
//! [`MemoryStore`] reads/writes one Markdown-with-frontmatter file per agent
//! at `~/.config/forge/memory/<agent>.md`. The file body is appended to the
//! agent's system prompt under a `## Memory` heading after `AGENTS.md` when
//! the agent's per-agent memory flag is enabled — see
//! [`assemble_system_prompt`].
//!
//! ## Format
//!
//! ```text
//! ---
//! updated_at: 2026-04-26T12:00:00Z
//! version: 1
//! ---
//! free-form markdown body the agent has accumulated
//! ```
//!
//! ## Security model
//!
//! - Memory is plain Markdown — no executable content, no template
//!   evaluation. Bytes round-trip verbatim.
//! - Files are written with mode `0600` on Unix so only the owning user can
//!   read them. The parent directory is created with mode `0700`.
//! - **Secrets must NEVER be written to memory.** There is no encryption,
//!   no redaction. The body is appended verbatim into the system prompt of
//!   every subsequent agent turn, so anything in memory is visible to the
//!   model and to every transport that carries the prompt.
//! - Reads are best-effort: a corrupt frontmatter or a permission denial is
//!   logged at WARN and skipped. The session continues without memory
//!   injection — never crash on a bad file.
//!
//! See `docs/architecture/memory.md` for the full security and operational
//! contract.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use gray_matter::{engine::YAML, Matter, ParsedEntity};
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// Per-file YAML frontmatter for a memory file.
///
/// Bumped by [`MemoryStore::write`] on every write — `version` increments
/// monotonically and `updated_at` snaps to the current `Utc::now()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryFrontmatter {
    /// ISO 8601 / RFC 3339 timestamp of the last write.
    pub updated_at: DateTime<Utc>,
    /// Monotonic version counter — starts at 1, increments on every write.
    pub version: u64,
}

impl Default for MemoryFrontmatter {
    fn default() -> Self {
        Self {
            updated_at: Utc::now(),
            version: 1,
        }
    }
}

/// Parsed memory file: frontmatter + free-form body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Memory {
    /// Versioning metadata.
    pub frontmatter: MemoryFrontmatter,
    /// Markdown body — appended verbatim to the system prompt under a
    /// `## Memory` heading when injection is enabled.
    pub body: String,
}

/// Mode flag for [`MemoryStore::write`].
///
/// `Append` joins the new content to the existing body with a single
/// newline separator. `Replace` discards the existing body in full.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// Concatenate `\n` + new content to the existing body.
    Append,
    /// Discard the existing body and write `content` as the entire new body.
    Replace,
}

impl WriteMode {
    /// Parse the wire-level string ("append" / "replace") into the typed
    /// enum; the IPC tool surface uses these strings verbatim.
    pub fn parse(s: &str) -> std::result::Result<Self, anyhow::Error> {
        match s {
            "append" => Ok(Self::Append),
            "replace" => Ok(Self::Replace),
            other => Err(anyhow::anyhow!(
                "unknown memory.write mode '{other}': expected 'append' or 'replace'"
            )),
        }
    }
}

/// Filesystem-backed per-agent memory store rooted at
/// `<config_root>/forge/memory/<agent>.md`.
///
/// `config_root` is normally `~/.config` on Unix; tests inject a tempdir.
/// The store creates the `forge/memory/` directory on first write with
/// mode `0700` on Unix.
#[derive(Debug, Clone)]
pub struct MemoryStore {
    root: PathBuf,
}

impl MemoryStore {
    /// Build a store rooted at `<config_root>/forge/memory/`. The directory
    /// is *not* created here — creation is deferred to the first
    /// [`MemoryStore::save`] / [`MemoryStore::write`] so a read-only
    /// session never touches the filesystem.
    pub fn new(config_root: impl Into<PathBuf>) -> Self {
        Self {
            root: config_root.into().join("forge").join("memory"),
        }
    }

    /// Build a store anchored at the user's home `~/.config` directory.
    ///
    /// Returns `None` when the home directory cannot be resolved — callers
    /// should treat that as "memory disabled for this session" rather than
    /// failing the session.
    pub fn from_home() -> Option<Self> {
        let home = dirs::home_dir()?;
        Some(Self::new(home.join(".config")))
    }

    /// Path the store would read/write for the named agent.
    pub fn path_for(&self, agent_id: &str) -> PathBuf {
        self.root.join(format!("{agent_id}.md"))
    }

    /// Load the agent's memory file.
    ///
    /// Returns `Ok(None)` when the file is absent. Returns
    /// [`Error::Other`] only on hard IO errors (permission denied, etc.) —
    /// a corrupt frontmatter is logged and surfaces as `Ok(None)` so a
    /// session can never crash on a malformed memory file.
    pub fn load(&self, agent_id: &str) -> Result<Option<Memory>> {
        let path = self.path_for(agent_id);
        if !path.exists() {
            return Ok(None);
        }
        let raw = match fs::read_to_string(&path)
            .with_context(|| format!("reading memory file {}", path.display()))
        {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(
                    target: "forge_agents::memory",
                    path = %path.display(),
                    error = %err,
                    "failed to read memory file; treating as absent",
                );
                return Ok(None);
            }
        };

        let matter = Matter::<YAML>::new();
        let parsed: ParsedEntity<MemoryFrontmatter> = match matter.parse(&raw) {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(
                    target: "forge_agents::memory",
                    path = %path.display(),
                    error = %err,
                    "memory frontmatter parse failed; skipping injection",
                );
                return Ok(None);
            }
        };

        let Some(fm) = parsed.data else {
            tracing::warn!(
                target: "forge_agents::memory",
                path = %path.display(),
                "memory file missing YAML frontmatter; skipping injection",
            );
            return Ok(None);
        };
        if fm.version == 0 {
            tracing::warn!(
                target: "forge_agents::memory",
                path = %path.display(),
                "memory frontmatter version must be a positive integer; skipping injection",
            );
            return Ok(None);
        }

        Ok(Some(Memory {
            frontmatter: fm,
            body: parsed.content,
        }))
    }

    /// Persist `memory` to the agent's file with an atomic temp + rename.
    ///
    /// On Unix the parent directory is enforced at mode `0700` and the
    /// file at `0600`. On Windows the platform default ACL is used.
    pub fn save(&self, agent_id: &str, memory: &Memory) -> Result<()> {
        ensure_dir_secure(&self.root)?;
        let path = self.path_for(agent_id);

        let serialized = serialize(memory);

        let tmp = path.with_extension("md.tmp");
        let mut file = open_secure_temp(&tmp)?;
        file.write_all(serialized.as_bytes())
            .map_err(|e| Error::Other(anyhow::Error::from(e)))?;
        file.sync_all()
            .map_err(|e| Error::Other(anyhow::Error::from(e)))?;
        // Drop the handle before rename — Windows requires it; on Unix it is
        // a harmless tightening of the lifetime.
        drop(file);

        fs::rename(&tmp, &path).map_err(|e| {
            // Clean up the temp file on failure — best-effort, ignore the
            // unlink error since we are already returning the rename error.
            let _ = fs::remove_file(&tmp);
            Error::Other(anyhow::Error::from(e))
        })?;

        // Idempotent re-tighten — covers the case where rename(2) preserved
        // a looser pre-existing destination mode. Best-effort.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
        }

        Ok(())
    }

    /// Append or replace the agent's memory body.
    ///
    /// On `Append`: the existing body and `content` are joined with a single
    /// `'\n'` separator. On `Replace`: `content` becomes the entire new body.
    /// Either way, `version` is incremented (starting at `1` if no prior
    /// file existed) and `updated_at` is set to the current `Utc::now()`.
    pub fn write(&self, agent_id: &str, content: &str, mode: WriteMode) -> Result<Memory> {
        let prior = self.load(agent_id)?;
        let next_version = prior
            .as_ref()
            .map(|m| m.frontmatter.version + 1)
            .unwrap_or(1);

        let body = match (mode, prior.as_ref()) {
            (WriteMode::Replace, _) => content.to_string(),
            (WriteMode::Append, Some(p)) if p.body.is_empty() => content.to_string(),
            (WriteMode::Append, Some(p)) => format!("{}\n{}", p.body, content),
            (WriteMode::Append, None) => content.to_string(),
        };

        let memory = Memory {
            frontmatter: MemoryFrontmatter {
                updated_at: Utc::now(),
                version: next_version,
            },
            body,
        };
        self.save(agent_id, &memory)?;
        Ok(memory)
    }
}

/// Serialize a [`Memory`] into the canonical on-disk shape:
/// `---\n<yaml>\n---\n<body>`.
///
/// The frontmatter has exactly two scalar fields and both serialize safely
/// without quoting (`updated_at` is ISO 8601, `version` is a positive
/// integer), so we emit YAML by hand rather than depending on a generic
/// YAML serializer for two lines.
fn serialize(memory: &Memory) -> String {
    let mut out = String::with_capacity(memory.body.len() + 96);
    out.push_str("---\n");
    out.push_str(&format!(
        "updated_at: {}\nversion: {}\n",
        memory.frontmatter.updated_at.to_rfc3339(),
        memory.frontmatter.version,
    ));
    out.push_str("---\n");
    out.push_str(&memory.body);
    out
}

#[cfg(unix)]
fn ensure_dir_secure(dir: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(dir).map_err(|e| Error::Other(anyhow::Error::from(e)))?;
    // Idempotent — tightens the dir even if create_dir_all observed it
    // pre-existing under a more permissive mode.
    let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    Ok(())
}

#[cfg(not(unix))]
fn ensure_dir_secure(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir).map_err(|e| Error::Other(anyhow::Error::from(e)))?;
    Ok(())
}

#[cfg(unix)]
fn open_secure_temp(path: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| Error::Other(anyhow::Error::from(e)))
}

#[cfg(not(unix))]
fn open_secure_temp(path: &Path) -> Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| Error::Other(anyhow::Error::from(e)))
}

/// Heading the memory body is injected under in the assembled system prompt.
///
/// Public so consumers (and tests) can assert the exact label without string
/// duplication.
pub const MEMORY_HEADING: &str = "\n\n---\n## Memory\n";

/// Build the final system prompt for an agent turn.
///
/// Order: optional `AGENTS.md` (already labeled by the caller) followed by
/// optional memory body under a `## Memory` heading. Returns `None` when
/// both inputs are absent so the caller can leave `ChatRequest.system`
/// unset rather than send an empty string.
///
/// Pure / side-effect-free so tests can drive every shape directly without
/// touching the filesystem. Memory injection at the call site is gated on
/// the per-agent flag — this helper does *not* re-check it.
pub fn assemble_system_prompt(
    agents_md_prefix: Option<&str>,
    memory_body: Option<&str>,
) -> Option<String> {
    match (agents_md_prefix, memory_body) {
        (None, None) => None,
        (Some(a), None) => Some(a.to_string()),
        (None, Some(m)) => Some(format!("{MEMORY_HEADING}{m}")),
        (Some(a), Some(m)) => Some(format!("{a}{MEMORY_HEADING}{m}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn store(dir: &Path) -> MemoryStore {
        MemoryStore::new(dir)
    }

    #[test]
    fn load_returns_none_when_file_absent() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        assert!(s.load("ghost").unwrap().is_none());
    }

    #[test]
    fn save_then_load_roundtrips_frontmatter_and_body() {
        // Note: gray_matter trims one trailing newline from the content
        // section. Memory injection happens under a `## Memory` heading
        // and the body is concatenated, so a single trailing newline
        // on either side is cosmetic — we tolerate the trim.
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        let original = Memory {
            frontmatter: MemoryFrontmatter {
                updated_at: Utc::now(),
                version: 7,
            },
            body: "remember the milk".to_string(),
        };
        s.save("scribe", &original).unwrap();
        let loaded = s.load("scribe").unwrap().unwrap();
        assert_eq!(loaded.frontmatter.version, 7);
        assert_eq!(loaded.body, "remember the milk");
    }

    #[test]
    fn write_append_creates_file_and_starts_at_version_one() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        let result = s.write("scribe", "first note", WriteMode::Append).unwrap();
        assert_eq!(result.frontmatter.version, 1);
        assert_eq!(result.body, "first note");
        let loaded = s.load("scribe").unwrap().unwrap();
        assert_eq!(loaded.body, "first note");
    }

    #[test]
    fn write_append_concatenates_with_newline() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        s.write("scribe", "alpha", WriteMode::Append).unwrap();
        let second = s.write("scribe", "beta", WriteMode::Append).unwrap();
        assert_eq!(second.frontmatter.version, 2);
        assert_eq!(second.body, "alpha\nbeta");
    }

    #[test]
    fn write_replace_discards_prior_body() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        s.write("scribe", "old", WriteMode::Append).unwrap();
        let replaced = s.write("scribe", "new", WriteMode::Replace).unwrap();
        assert_eq!(replaced.frontmatter.version, 2);
        assert_eq!(replaced.body, "new");
    }

    #[test]
    fn write_increments_version_monotonically() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        for expected in 1..=5u64 {
            let m = s.write("scribe", "tick", WriteMode::Append).unwrap();
            assert_eq!(m.frontmatter.version, expected);
        }
    }

    #[test]
    fn write_advances_updated_at() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        let first = s.write("scribe", "a", WriteMode::Append).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let second = s.write("scribe", "b", WriteMode::Append).unwrap();
        assert!(
            second.frontmatter.updated_at > first.frontmatter.updated_at,
            "updated_at must advance on each write"
        );
    }

    #[test]
    fn corrupt_frontmatter_yields_none_without_panicking() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        ensure_dir_secure(&s.root).unwrap();
        fs::write(
            s.path_for("broken"),
            "---\nthis: is\n: not [valid yaml\n---\nbody",
        )
        .unwrap();
        assert!(s.load("broken").unwrap().is_none());
    }

    #[test]
    fn missing_frontmatter_yields_none() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        ensure_dir_secure(&s.root).unwrap();
        fs::write(s.path_for("plain"), "no frontmatter here").unwrap();
        assert!(s.load("plain").unwrap().is_none());
    }

    #[test]
    fn version_zero_is_rejected() {
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        ensure_dir_secure(&s.root).unwrap();
        fs::write(
            s.path_for("zerover"),
            "---\nupdated_at: 2026-04-26T12:00:00Z\nversion: 0\n---\nbody",
        )
        .unwrap();
        assert!(s.load("zerover").unwrap().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn save_writes_file_at_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let s = store(dir.path());
        let memory = Memory {
            frontmatter: MemoryFrontmatter {
                updated_at: Utc::now(),
                version: 1,
            },
            body: "secrets-MUST-NOT-go-here-but-perms-still-tight".to_string(),
        };
        s.save("locked", &memory).unwrap();
        let mode = fs::metadata(s.path_for("locked"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "memory file mode must be 0600, was {:o}",
            mode & 0o777
        );

        let dir_mode = fs::metadata(&s.root).unwrap().permissions().mode();
        assert_eq!(
            dir_mode & 0o777,
            0o700,
            "memory parent dir mode must be 0700, was {:o}",
            dir_mode & 0o777
        );
    }

    #[test]
    fn assemble_returns_none_when_both_inputs_absent() {
        assert_eq!(assemble_system_prompt(None, None), None);
    }

    #[test]
    fn assemble_passes_through_agents_md_only() {
        assert_eq!(
            assemble_system_prompt(Some("AGENTS prefix"), None).as_deref(),
            Some("AGENTS prefix"),
        );
    }

    #[test]
    fn assemble_appends_memory_after_agents_md() {
        let s = assemble_system_prompt(Some("AGENTS prefix"), Some("memo body")).unwrap();
        assert!(s.starts_with("AGENTS prefix"));
        assert!(
            s.ends_with("memo body"),
            "memory body must come last; got: {s:?}"
        );
        assert!(
            s.contains("## Memory"),
            "memory heading must be present; got: {s:?}"
        );
        let agents_idx = s.find("AGENTS prefix").unwrap();
        let mem_idx = s.find("## Memory").unwrap();
        assert!(
            agents_idx < mem_idx,
            "AGENTS.md must precede Memory in the assembled prompt"
        );
    }

    #[test]
    fn assemble_uses_memory_alone_when_agents_md_absent() {
        let s = assemble_system_prompt(None, Some("memo body")).unwrap();
        assert!(s.contains("## Memory"));
        assert!(s.ends_with("memo body"));
    }

    #[test]
    fn write_mode_parse_accepts_documented_strings() {
        assert_eq!(WriteMode::parse("append").unwrap(), WriteMode::Append);
        assert_eq!(WriteMode::parse("replace").unwrap(), WriteMode::Replace);
        assert!(WriteMode::parse("clobber").is_err());
    }
}
