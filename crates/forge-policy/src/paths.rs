//! Edit paths as policy sees them, and gitignore-style patterns over them.

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};

/// A repo-relative path with `.` segments removed and `..` resolved, or `None` if it leaves
/// the repo or is absolute. Purely lexical: it doesn't follow symlinks, so callers resolve
/// them before asking for a decision (see [`crate::Request`]).
pub fn normalize(path: &str) -> Option<String> {
    let path = path.replace('\\', "/");
    if path.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            seg => parts.push(seg),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// A set of gitignore-style patterns over repo-relative paths, compiled once.
///
/// As in gitignore: a pattern without a `/` (other than a trailing one) matches at any depth; a
/// pattern with one is anchored at the repo root, as is one starting with `/`; `*` and `?`
/// stop at `/` while `**` crosses directories; and a pattern that matches a directory matches
/// everything under it. Negation (`!`) isn't supported. Matching ignores case on macOS, whose
/// filesystems usually do.
#[derive(Clone, Debug)]
pub struct PathPatterns {
    set: GlobSet,
    /// For each glob in `set`, the index of the pattern it came from.
    owner: Vec<usize>,
}

/// A pattern that doesn't compile.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("edit pattern {pattern:?}: {message}")]
pub struct PatternError {
    pub pattern: String,
    pub message: String,
}

impl PathPatterns {
    pub fn new<S: AsRef<str>>(patterns: &[S]) -> Result<Self, PatternError> {
        let mut builder = GlobSetBuilder::new();
        let mut owner = Vec::new();
        for (i, pattern) in patterns.iter().enumerate() {
            let pattern = pattern.as_ref();
            let fail = |message: String| PatternError {
                pattern: pattern.to_owned(),
                message,
            };
            if pattern.starts_with('!') {
                return Err(fail("negation ('!') is not supported".into()));
            }
            for glob in expand(pattern) {
                let compiled = GlobBuilder::new(&glob)
                    .literal_separator(true)
                    .backslash_escape(true)
                    .case_insensitive(cfg!(target_os = "macos"))
                    .build()
                    .map_err(|e| fail(e.kind().to_string()))?;
                builder.add(compiled);
                owner.push(i);
            }
        }
        let set = builder.build().map_err(|e| PatternError {
            pattern: String::new(),
            message: e.to_string(),
        })?;
        Ok(Self { set, owner })
    }

    /// The index of the first pattern matching `path` (already normalized).
    pub fn first_match(&self, path: &str) -> Option<usize> {
        self.set
            .matches(path)
            .into_iter()
            .map(|g| self.owner[g])
            .min()
    }
}

/// The globs that implement one gitignore-style pattern.
fn expand(pattern: &str) -> Vec<String> {
    let dir_only = pattern.ends_with('/');
    let trimmed = pattern.trim_end_matches('/');
    let anchored = trimmed.starts_with('/') || trimmed.contains('/');
    let body = trimmed.trim_start_matches('/');
    let base = if anchored {
        body.to_owned()
    } else {
        format!("**/{body}")
    };
    let under = format!("{base}/**");
    if dir_only {
        // `build/` matches only a directory, so only paths under it.
        vec![under]
    } else {
        vec![base, under]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, path: &str) -> bool {
        PathPatterns::new(&[pattern])
            .unwrap()
            .first_match(path)
            .is_some()
    }

    #[test]
    fn normalizes_or_refuses_paths() {
        assert_eq!(normalize("./src//a.rs").as_deref(), Some("src/a.rs"));
        assert_eq!(normalize("src/../.env").as_deref(), Some(".env"));
        assert_eq!(normalize(r"src\a.rs").as_deref(), Some("src/a.rs"));
        assert_eq!(normalize("../outside"), None);
        assert_eq!(normalize("src/../../x"), None);
        assert_eq!(normalize("/etc/passwd"), None);
        assert_eq!(normalize("."), None);
        assert_eq!(normalize(""), None);
    }

    #[test]
    fn unanchored_patterns_match_at_any_depth() {
        assert!(matches(".env*", ".env"));
        assert!(matches(".env*", ".env.local"));
        assert!(matches(".env*", "sub/.env"));
        assert!(!matches(".env*", "src/env.rs"));
        assert!(matches("*.pem", "certs/a/b.pem"));
        assert!(matches("Cargo.lock", "crates/x/Cargo.lock"));
    }

    #[test]
    fn slashes_anchor_at_the_root() {
        assert!(matches("/Cargo.lock", "Cargo.lock"));
        assert!(!matches("/Cargo.lock", "crates/x/Cargo.lock"));
        assert!(matches(".github/workflows/**", ".github/workflows/ci.yml"));
        assert!(!matches(
            ".github/workflows/**",
            "sub/.github/workflows/ci.yml"
        ));
        assert!(matches(
            "**/migrations/**",
            "crates/forged/migrations/0003.sql"
        ));
        assert!(matches("**/migrations/**", "migrations/0001.sql"));
    }

    #[test]
    fn stars_stop_at_slashes() {
        assert!(matches("src/*.rs", "src/a.rs"));
        assert!(!matches("src/*.rs", "src/x/a.rs"));
    }

    #[test]
    fn directories_cover_their_contents() {
        assert!(matches(".forge", ".forge/policy.toml"));
        assert!(matches(".forge/", ".forge/hooks/x.sh"));
        assert!(!matches(".forge/", ".forge"));
        assert!(matches("node_modules", "a/node_modules/x/index.js"));
    }

    #[test]
    fn reports_the_first_matching_pattern() {
        let set = PathPatterns::new(&["*.md", "docs/**", "*.toml"]).unwrap();
        assert_eq!(set.first_match("docs/a.md"), Some(0));
        assert_eq!(set.first_match("docs/a.toml"), Some(1));
        assert_eq!(set.first_match("src/a.rs"), None);
    }

    #[test]
    fn rejects_negation_and_bad_globs() {
        assert!(PathPatterns::new(&["!keep.txt"]).is_err());
        assert!(PathPatterns::new(&["src/[a"]).is_err());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn ignores_case_on_macos() {
        assert!(matches(".env*", ".ENV"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn keeps_case_on_linux() {
        assert!(!matches(".env*", ".ENV"));
    }
}
