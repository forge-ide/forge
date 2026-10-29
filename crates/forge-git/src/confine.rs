//! Confining the daemon's own git when it opens a worktree's files.
//!
//! Whatever runs in a worktree can still be running while the daemon's git reads or writes it,
//! and git checks a path for symlinks before it opens it, not as it opens it. So a file or dir
//! swapped for a symlink at the right moment can make the daemon, which runs as the user, read
//! a file the sandbox hides or write through the link. Git commands that open the worktree's
//! files therefore run confined to the paths they need, so a link can only lead to those. Ones
//! that never open them (reading and writing objects, refs, and the daemon's index) don't need
//! to be.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::worktree::{GitDirs, GitError};

/// Starts git confined to some paths, on top of what any git needs (the system, git itself,
/// and the user's git config, read-only). Implemented by the sandbox.
pub trait Confine: Send + Sync {
    /// A command that runs git, able to read `read` and to read and write `write`, and
    /// nothing else of the user's; or why it can't.
    fn git(&self, read: &[PathBuf], write: &[PathBuf]) -> Result<std::process::Command, String>;
}

/// A confined git, with the config it reads in place of the user's own.
#[derive(Clone)]
pub struct Confined {
    pub confine: Arc<dyn Confine>,
    /// The snapshot of the user's git config it reads (see [`snapshot`]).
    pub config: PathBuf,
    /// The files that snapshot names for it to read: ignore and attributes files.
    pub read: Vec<PathBuf>,
}

/// How the daemon runs git commands that open a worktree's files.
#[derive(Clone, Default)]
pub enum Launch {
    /// Plainly, as the daemon, with its whole environment: for tests.
    Unconfined,
    /// Unconfined, through a [`Confine`] that ignores the paths but still sets git's
    /// environment: for sessions that run unsandboxed, whose agent can already reach
    /// everything the daemon's git could.
    Plain(Arc<dyn Confine>),
    /// Confined, through a [`Confine`].
    Confined(Confined),
    /// Not at all, for this reason. The default, so nothing runs unconfined by omission.
    Refused(String),
    #[default]
    Unset,
}

impl std::fmt::Debug for Launch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Launch::Unconfined => f.write_str("Unconfined"),
            Launch::Plain(_) => f.write_str("Plain"),
            Launch::Confined(_) => f.write_str("Confined"),
            Launch::Refused(why) => f.debug_tuple("Refused").field(why).finish(),
            Launch::Unset => f.write_str("Unset"),
        }
    }
}

impl PartialEq for Launch {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Launch::Confined(a), Launch::Confined(b)) => {
                Arc::ptr_eq(&a.confine, &b.confine) && a.config == b.config && a.read == b.read
            }
            (Launch::Plain(a), Launch::Plain(b)) => Arc::ptr_eq(a, b),
            (Launch::Refused(a), Launch::Refused(b)) => a == b,
            (a, b) => std::mem::discriminant(a) == std::mem::discriminant(b),
        }
    }
}

impl Eq for Launch {}

/// What a git command opens in the worktree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reach {
    /// Nothing: it only reads objects, refs, or an index, and writes objects or refs.
    Repo,
    /// Reads its files, and may write objects and the index.
    Scan,
    /// Writes its files from the index.
    Checkout,
}

/// What `args` (a git command line, without `git`) opens in the worktree. Anything not known
/// to stay out of it counts as reading it. That includes every command that writes an index:
/// git re-reads, through its filters, any entry too recent to trust its timestamps for
/// ("racily clean") whenever it writes the index, so `write-tree`, `read-tree`, and
/// `update-index` can all open the worktree's files.
pub(crate) fn reach(args: &[&str]) -> Reach {
    let has = |flag: &str| args.contains(&flag);
    match args.first().copied() {
        Some(
            "rev-parse" | "ls-tree" | "commit-tree" | "update-ref" | "cat-file" | "diff-tree"
            | "merge-base",
        ) => Reach::Repo,
        Some("ls-files") if has("-s") && !has("--others") && !has("-o") => Reach::Repo,
        // Objects from stdin, as they are: no filter, so no file of the worktree's is read.
        Some("hash-object") if has("--stdin") && has("--no-filters") => Reach::Repo,
        // Merges trees into objects, never touching an index or a worktree; attributes come
        // from the tree `GIT_ATTR_SOURCE` names.
        Some("merge-tree") if has("--write-tree") => Reach::Repo,
        Some("config") if has("--get") => Reach::Repo,
        Some("worktree") if args.get(1) == Some(&"list") => Reach::Repo,
        // Sends objects and updates the remote's refs; local refs only as tracking refs.
        Some("push") => Reach::Repo,
        Some("read-tree") if has("-u") => Reach::Checkout,
        _ => Reach::Scan,
    }
}

/// The paths a command of `reach` may read and write, on `worktree`, with the index at
/// `index` (the daemon's own, or the worktree's in its own git dir): never the repo's refs,
/// hooks, or config, nor the rest of the daemon's files.
pub(crate) fn paths(
    reach: Reach,
    dirs: &GitDirs,
    worktree: &Path,
    index: &Path,
) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let index_dir = index.parent().unwrap_or(index).to_path_buf();
    let read = vec![dirs.common.clone(), worktree.to_path_buf()];
    let mut write = vec![index_dir];
    match reach {
        Reach::Repo => {}
        Reach::Scan => {
            write.push(dirs.common.join("objects"));
            // Git LFS's clean filter stores the files it replaces with pointers here.
            let lfs = dirs.common.join("lfs");
            if lfs.is_dir() {
                write.push(lfs);
            }
        }
        Reach::Checkout => write.push(worktree.to_path_buf()),
    }
    (read, write)
}

/// The daemon's `PATH` with only the entries that are absolute and exist (see
/// [`absolute_path_of`]).
pub fn absolute_path() -> std::ffi::OsString {
    absolute_path_of(&std::env::var_os("PATH").unwrap_or_default())
}

/// `path` with only the entries that are absolute, exist, and aren't under `/proc`, `/dev`, or
/// `/sys`. A relative one (`.`, an empty one) would find a program by its name in whatever
/// directory git or a filter runs in, such as a worktree.
pub fn absolute_path_of(path: &std::ffi::OsStr) -> std::ffi::OsString {
    std::env::join_paths(std::env::split_paths(path).filter(|dir| {
        dir.is_absolute()
            && dir.is_dir()
            && !["/proc", "/dev", "/sys"]
                .iter()
                .any(|special| dir.starts_with(special))
    }))
    .unwrap_or_default()
}

/// The git the daemon runs, found once through the absolute `PATH` entries only, as it
/// resolves.
pub fn git_program() -> Result<&'static Path, GitError> {
    static GIT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    GIT.get_or_init(|| {
        std::env::split_paths(&absolute_path())
            .map(|dir| dir.join("git"))
            .find(|candidate| candidate.is_file())
            .and_then(|git| std::fs::canonicalize(git).ok())
    })
    .as_deref()
    .ok_or_else(|| GitError::NotConfined("git is not on the daemon's PATH".into()))
}

/// The daemon's git, never by its bare name: [`git_program`], with the daemon's environment
/// but for a `PATH` of only absolute entries, and no prompts. Every git the daemon starts
/// begins here.
pub fn plain() -> Result<std::process::Command, GitError> {
    let mut cmd = std::process::Command::new(git_program()?);
    cmd.env("PATH", absolute_path())
        .env("GIT_TERMINAL_PROMPT", "0");
    Ok(cmd)
}

/// The user's own git config keys a confined git keeps: where their ignore and attributes
/// files are, line-ending and path-safety settings, and filter drivers (Git LFS's among them),
/// so it records what the user's git would. Everything else, where tokens tend to live
/// (`url.*`, `http.*`, `credential.*`, `remote.*`, `lfs.*`), is left out.
const KEPT_CORE: &[&str] = &[
    "core.excludesfile",
    "core.attributesfile",
    "core.autocrlf",
    "core.eol",
    "core.safecrlf",
    "core.checkroundtripencoding",
    "core.symlinks",
    "core.ignorecase",
    "core.precomposeunicode",
    "core.protecthfs",
    "core.protectntfs",
    "core.bigfilethreshold",
    "core.compression",
    "core.loosecompression",
];

fn kept(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    KEPT_CORE.contains(&key.as_str())
        || (key.starts_with("filter.")
            && key.matches('.').count() >= 2
            && [".clean", ".smudge", ".process", ".required"]
                .iter()
                .any(|var| key.ends_with(var)))
}

/// What a confined git reads of the user's own git config.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    /// Keys and values, in the order git listed them.
    pub entries: Vec<(String, String)>,
    /// The ignore and attributes files they name, as they resolve, for it to read.
    pub read: Vec<PathBuf>,
    /// Keys left out because their value looked like a secret.
    pub dropped: Vec<String>,
}

/// The settings of the user's global git config (and what it includes, for `repo`) that a
/// confined git keeps, as git sees them with `home` and `xdg_config` as the user's
/// directories. A value `looks_secret` says holds a secret is left out. Confined git never
/// reads the user's config itself: a symlink swapped in while it works could otherwise
/// lead `git add` to stage a token from it where the agent can read it. Blocking: runs git.
pub fn snapshot(
    repo: &GitDirs,
    home: &Path,
    xdg_config: &Path,
    looks_secret: &dyn Fn(&str) -> bool,
) -> Result<Snapshot, GitError> {
    let out = plain_git(home, xdg_config)?
        .args(["config", "--list", "-z", "--show-origin", "--show-scope"])
        .env("GIT_DIR", &repo.common)
        .current_dir(&repo.common)
        .output()
        .map_err(|source| GitError::Io {
            context: "listing the user's git config".into(),
            source,
        })?;
    if !out.status.success() {
        return Err(GitError::Command {
            args: "config --list".into(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    let listed = String::from_utf8_lossy(&out.stdout);
    let mut fields = listed.split('\0');
    let mut snap = Snapshot::default();
    while let (Some(scope), Some(_origin), Some(entry)) =
        (fields.next(), fields.next(), fields.next())
    {
        // The repo's own config and the system's, git reads anyway.
        if scope != "global" {
            continue;
        }
        // A key listed with no value is a bare boolean, which means true; written back
        // empty, it would mean false.
        let (key, value) = entry.split_once('\n').unwrap_or((entry, "true"));
        if !kept(key) {
            continue;
        }
        if looks_secret(value) {
            snap.dropped.push(key.to_owned());
            continue;
        }
        let value = if is_path_key(key) {
            let Some(path) = expand(value, home) else {
                continue;
            };
            snap.read.extend(std::fs::canonicalize(&path));
            path.display().to_string()
        } else {
            value.to_owned()
        };
        snap.entries.push((key.to_owned(), value));
    }
    // Git's default ignore and attributes files, named outright.
    for (key, file) in [
        ("core.excludesFile", "ignore"),
        ("core.attributesFile", "attributes"),
    ] {
        let set = snap
            .entries
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case(key));
        let path = xdg_config.join("git").join(file);
        if !set && path.is_file() {
            snap.read.extend(std::fs::canonicalize(&path));
            snap.entries
                .push((key.to_owned(), path.display().to_string()));
        }
    }
    Ok(snap)
}

/// Write `snap` as a git config file at `path`, readable only by the user.
pub fn write_snapshot(snap: &Snapshot, path: &Path) -> Result<(), GitError> {
    let io = |source| GitError::Io {
        context: format!("writing {}", path.display()),
        source,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    let _ = std::fs::remove_file(path);
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(io)?;
    }
    for (key, value) in &snap.entries {
        let out = plain_git(Path::new("/nonexistent"), Path::new("/nonexistent"))?
            .args(["config", "--file"])
            .arg(path)
            .args(["--add", "--", key, value])
            .output()
            .map_err(io)?;
        if !out.status.success() {
            return Err(GitError::Command {
                args: format!("config --file {} --add {key}", path.display()),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            });
        }
    }
    Ok(())
}

fn is_path_key(key: &str) -> bool {
    key.eq_ignore_ascii_case("core.excludesFile") || key.eq_ignore_ascii_case("core.attributesFile")
}

/// `value` as an absolute path, with a leading `~/` meaning `home`; `None` for anything else.
fn expand(value: &str, home: &Path) -> Option<PathBuf> {
    let path = match value.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(value),
    };
    path.is_absolute().then_some(path)
}

/// Git with no environment of the daemon's that changes where config comes from.
fn plain_git(home: &Path, xdg_config: &Path) -> Result<std::process::Command, GitError> {
    let mut cmd = plain()?;
    cmd.env("HOME", home)
        .env("XDG_CONFIG_HOME", xdg_config)
        .stdin(std::process::Stdio::null());
    for var in [
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_COUNT",
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
    ] {
        cmd.env_remove(var);
    }
    Ok(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_commands_known_to_stay_out_of_the_worktree_run_unconfined() {
        for repo in [
            &["rev-parse", "--verify", "x"][..],
            &["ls-tree", "-r", "-z", "x"],
            &["commit-tree", "t", "-p", "p", "-F", "-"],
            &["update-ref", "-m", "m", "r", "n", "o"],
            &["cat-file", "-s", "x"],
            &["diff-tree", "-r", "a", "b"],
            &["diff-tree", "--stdin", "-r", "-z", "-M", "--raw"],
            &["cat-file", "--batch"],
            &["merge-base", "a", "b"],
            &["ls-files", "-s", "-z"],
            &["hash-object", "-w", "--stdin", "--no-filters"],
            &["merge-tree", "--write-tree", "-z", "a", "b"],
            &["config", "--get", "branch.main.remote"],
            &["worktree", "list", "--porcelain", "-z"],
            &["push", "--porcelain", "origin", "a:refs/heads/b"],
        ] {
            assert_eq!(reach(repo), Reach::Repo, "{repo:?}");
        }
        for scan in [
            &["add", "--all", "--", "."][..],
            &["status", "--porcelain=v1"],
            &["diff-files", "--name-only"],
            &["ls-files", "-z", "--others"],
            &["ls-files", "-s", "--others"],
            &["update-index", "-q", "--refresh"],
            // Writing an index can re-read racily clean entries from the worktree.
            &["write-tree"],
            &["read-tree", "x"],
            &["update-index", "-z", "--index-info"],
            &["update-index", "-z", "--force-remove", "--stdin"],
            &["hash-object", "x"],
            &["hash-object", "-w", "--stdin"],
            &["merge-tree", "a", "b"],
            &["config", "x"],
            &["worktree", "add", "x"],
            &["something-new"],
            &[],
        ] {
            assert_eq!(reach(scan), Reach::Scan, "{scan:?}");
        }
        assert_eq!(reach(&["read-tree", "-m", "-u", "a", "b"]), Reach::Checkout);
        assert_eq!(reach(&["read-tree", "--reset", "-u", "a"]), Reach::Checkout);
    }

    #[test]
    fn confined_commands_never_write_refs_hooks_or_config() {
        let dirs = GitDirs {
            common: "/r/.git".into(),
            admin: "/r/.git/worktrees/x".into(),
            launch: Launch::Unset,
        };
        let (wt, index) = (
            Path::new("/r/.worktrees/x"),
            Path::new("/s/ses/git/i.index"),
        );
        let (read, scan) = paths(Reach::Scan, &dirs, wt, index);
        assert_eq!(read, [PathBuf::from("/r/.git"), wt.to_path_buf()]);
        assert_eq!(
            scan,
            [PathBuf::from("/s/ses/git"), "/r/.git/objects".into()]
        );
        let (_, checkout) = paths(Reach::Checkout, &dirs, wt, index);
        assert_eq!(checkout, [PathBuf::from("/s/ses/git"), wt.to_path_buf()]);
        // The worktree's own index is in its own git dir, which is then what may be written.
        let (_, own) = paths(Reach::Scan, &dirs, wt, &dirs.admin.join("index"));
        assert_eq!(
            own,
            [
                PathBuf::from("/r/.git/worktrees/x"),
                "/r/.git/objects".into()
            ]
        );
        for write in scan.iter().chain(&checkout) {
            assert_ne!(write, &dirs.common, "the whole git dir");
            assert!(!write.starts_with("/r/.git/refs") && !write.starts_with("/r/.git/hooks"));
        }
    }

    #[test]
    fn the_config_snapshot_keeps_only_ignores_line_endings_and_filters() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let (home, xdg) = (root.join("home"), root.join("home/.config"));
        std::fs::create_dir_all(xdg.join("git")).unwrap();
        std::fs::write(xdg.join("git/attributes"), "*.png binary\n").unwrap();
        std::fs::write(xdg.join("git/credentials"), "https://u:tok@host\n").unwrap();
        std::fs::write(home.join("ignore"), ".DS_Store\n").unwrap();
        std::fs::write(
            home.join("inc"),
            "[http]\n\textraHeader = Authorization: tok\n[filter \"y\"]\n\tclean = keep-me\n",
        )
        .unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            "[core]\n\tautocrlf = input\n\texcludesFile = ~/ignore\n\tsshCommand = ssh -i k\n\
             [url \"https://tok@host/\"]\n\tinsteadOf = https://host/\n\
             [lfs]\n\turl = https://u:tok@lfs\n[credential]\n\thelper = store\n\
             [include]\n\tpath = ~/inc\n\
             [filter \"lfs\"]\n\tclean = git-lfs clean -- %f\n\trequired = true\n\
             [filter \"x\"]\n\tclean = run --token SECRET\n[filter \"bare\"]\n\trequired\n\
             [user]\n\tname = U\n",
        )
        .unwrap();
        let repo = root.join("repo");
        std::fs::create_dir(&repo).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(&repo)
                .status()
                .unwrap()
                .success()
        );
        let dirs = GitDirs {
            common: repo.join(".git"),
            admin: repo.join(".git"),
            launch: Launch::Unset,
        };
        let snap = snapshot(&dirs, &home, &xdg, &|v| v.contains("SECRET")).unwrap();
        let keys: Vec<&str> = snap.entries.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            [
                "core.autocrlf",
                "core.excludesfile",
                "filter.y.clean",
                "filter.lfs.clean",
                "filter.lfs.required",
                "filter.bare.required",
                "core.attributesFile",
            ]
        );
        assert_eq!(snap.dropped, ["filter.x.clean"]);
        assert_eq!(
            snap.entries[1].1,
            home.join("ignore").display().to_string(),
            "~ is expanded"
        );
        // Single files to read, never the dirs that hold credentials beside them.
        assert_eq!(snap.read, [home.join("ignore"), xdg.join("git/attributes")]);
        // Written, it reads back the same, values starting with dashes included.
        let path = root.join("snapshot");
        write_snapshot(&snap, &path).unwrap();
        let read = std::process::Command::new("git")
            .args(["config", "--file"])
            .arg(&path)
            .args(["--list"])
            .output()
            .unwrap();
        let listed = String::from_utf8(read.stdout).unwrap();
        assert!(
            listed.contains("filter.lfs.clean=git-lfs clean -- %f"),
            "{listed}"
        );
        // A bare boolean stays true.
        let bare = std::process::Command::new("git")
            .args(["config", "--file"])
            .arg(&path)
            .args(["--bool", "filter.bare.required"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&bare.stdout).trim(), "true");
        assert!(!listed.contains("tok"), "{listed}");
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn only_absolute_existing_path_entries_are_kept() {
        let kept = absolute_path_of(std::ffi::OsStr::new(
            ".::bin:/usr/bin:/nonexistent-forge-dir:/proc/self:/dev",
        ));
        assert_eq!(kept, "/usr/bin");
        assert!(git_program().unwrap().is_absolute());
    }
}
