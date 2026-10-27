//! The daemon's confined git: what it may read and write, and that a symlink swapped in while
//! it works can't lead it anywhere else. Run with `just test-sandbox`; a machine that can't
//! confine git fails the suite instead of skipping.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use forge_git::commit::{self, CommitError};
use forge_git::{GitCli, GitDirs, Launch, Worktrees, confine};
use forge_sandbox::GitConfinement;
use tempfile::TempDir;

const HELPER: &str = env!("CARGO_BIN_EXE_forge-sandbox-exec");
const SECRET: &str = "PRIVATE KEY that only the user may read\n";
const CREDENTIALS: &str = "https://user:tok-credentials@example.com\n";
const INCLUDED: &str = "[http]\n\textraHeader = Authorization: tok-include\n";
/// A race runs at least this many rounds and this long, however slow the host's confinement.
const ROUNDS: u32 = 20;
const RACE: Duration = Duration::from_secs(3);
/// Past this, a race that still hasn't reached `ROUNDS` fails instead of spinning on.
const RACE_CAP: Duration = Duration::from_secs(60);

fn racing(start: Instant, rounds: u32) -> bool {
    (rounds < ROUNDS || start.elapsed() < RACE) && start.elapsed() < RACE_CAP
}

/// A home with a key, a repo inside it with a session worktree, a dir for the daemon's index,
/// and the confinement.
struct Fixture {
    _dir: TempDir,
    home: PathBuf,
    repo: PathBuf,
    wt: PathBuf,
    dirs: GitDirs,
    index_dir: PathBuf,
    git: Arc<GitConfinement>,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(home.join(".ssh")).unwrap();
        std::fs::write(home.join(".ssh/id_test"), SECRET).unwrap();
        let index_dir = root.join("sessions/ses_1/git");
        std::fs::create_dir_all(&index_dir).unwrap();
        // The user's git config: tokens in it, beside it, and in a file it includes, next to a
        // global ignore file and a filter, which confined git still honors.
        std::fs::create_dir_all(home.join(".config/git")).unwrap();
        std::fs::write(home.join(".config/git/credentials"), CREDENTIALS).unwrap();
        std::fs::write(home.join("inc"), INCLUDED).unwrap();
        std::fs::write(home.join("global-ignore"), "*.log\n").unwrap();
        std::fs::write(
            home.join(".gitconfig"),
            format!(
                "[url \"https://tok-gitconfig@example.com/\"]\n\tinsteadOf = https://example.com/\n\
                 [include]\n\tpath = ~/inc\n[core]\n\texcludesFile = ~/global-ignore\n\
                 [filter \"probe\"]\n\tclean = touch {}; cat\n",
                index_dir.join("global-filter-ran").display()
            ),
        )
        .unwrap();
        let repo = home.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "--quiet", "--initial-branch=main"]);
        // The repo's own identity, so commits don't depend on the machine's git config.
        git(&repo, &["config", "user.name", "T"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        std::fs::write(repo.join("a.txt"), "a\n").unwrap();
        git(&repo, &["add", "a.txt"]);
        git(&repo, &["commit", "--quiet", "-m", "initial"]);
        let wt = GitCli
            .create(&repo, "feat/x", "main", forge_git::CreateMode::New)
            .unwrap();
        let git_confinement = Arc::new(
            GitConfinement::probe(HELPER.into())
                .unwrap_or_else(|why| panic!("this machine can't confine git: {why}")),
        );
        let mut dirs = GitCli.git_dirs(&repo, &wt).unwrap();
        let snap = confine::snapshot(&dirs, &home, &home.join(".config"), &|_| false).unwrap();
        let config = root.join("sessions/ses_1/git-config");
        confine::write_snapshot(&snap, &config).unwrap();
        dirs.launch = Launch::Confined(confine::Confined {
            confine: git_confinement.clone(),
            config,
            read: snap.read,
        });
        Fixture {
            _dir: dir,
            home,
            repo,
            wt,
            dirs,
            index_dir,
            git: git_confinement,
        }
    }

    /// Git confined to `read` and `write`, on the repo's shared git dir.
    fn confined(&self, read: &[&Path], write: &[&Path], args: &[&str]) -> std::process::Output {
        let read: Vec<PathBuf> = read.iter().map(|p| p.to_path_buf()).collect();
        let write: Vec<PathBuf> = write.iter().map(|p| p.to_path_buf()).collect();
        let mut cmd = self.git.command(&read, &write);
        cmd.args(args)
            .current_dir(&self.wt)
            .env("GIT_DIR", self.repo.join(".git"))
            .env("GIT_WORK_TREE", &self.wt)
            .env("GIT_INDEX_FILE", self.index_dir.join("t.index"))
            // Confined git can't read the user's config; the daemon gives it a snapshot.
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .stdin(Stdio::null());
        cmd.output().unwrap()
    }
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "T")
        .env("GIT_AUTHOR_EMAIL", "t@example.com")
        .env("GIT_COMMITTER_NAME", "T")
        .env("GIT_COMMITTER_EMAIL", "t@example.com")
        .stdout(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

#[test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
fn confined_git_reads_only_what_it_was_given() {
    let f = Fixture::new();
    let git_dir = f.repo.join(".git");
    let (read, write) = ([git_dir.as_path(), &f.wt], [f.index_dir.as_path()]);
    let inside = f.confined(&read, &write, &["hash-object", "a.txt"]);
    assert!(inside.status.success(), "{inside:?}");
    let key = f.home.join(".ssh/id_test");
    let outside = f.confined(&read, &write, &["hash-object", key.to_str().unwrap()]);
    assert!(!outside.status.success(), "read the key: {outside:?}");
    // Control: unconfined, the same command reads it.
    let plain = Command::new("git")
        .args(["hash-object", key.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(plain.status.success());
}

#[test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
fn confined_git_writes_only_what_it_was_given() {
    let f = Fixture::new();
    let git_dir = f.repo.join(".git");
    let read = [git_dir.as_path(), &f.wt];
    git(
        &f.repo,
        &[
            "read-tree",
            "--index-output",
            f.index_dir.join("t.index").to_str().unwrap(),
            "HEAD",
        ],
    );
    let out = f.home.join("planted");
    let prefix = |p: &Path| format!("--prefix={}/", p.display());
    let ok = f.confined(
        &read,
        &[&f.wt, &f.index_dir],
        &["checkout-index", "-a", &prefix(&f.wt.join("copy"))],
    );
    assert!(ok.status.success(), "{ok:?}");
    assert!(f.wt.join("copy/a.txt").exists());
    let escaped = f.confined(
        &read,
        &[&f.wt, &f.index_dir],
        &["checkout-index", "-a", &prefix(&out)],
    );
    assert!(!escaped.status.success(), "{escaped:?}");
    assert!(!out.exists(), "wrote outside its paths");
    // Nor can it move a branch or plant a hook in the repo's git dir it may only read.
    let main = std::fs::read_to_string(git_dir.join("refs/heads/main")).unwrap();
    let moved = f.confined(
        &read,
        &[&f.wt, &f.index_dir],
        &["update-ref", "refs/heads/main", "refs/heads/feat/x"],
    );
    assert!(!moved.status.success(), "{moved:?}");
    assert_eq!(
        std::fs::read_to_string(git_dir.join("refs/heads/main")).unwrap(),
        main
    );
    let branch = f.confined(&read, &[&f.wt, &f.index_dir], &["branch", "planted"]);
    assert!(!branch.status.success(), "{branch:?}");
    assert!(!git_dir.join("refs/heads/planted").exists());
    // Control: unconfined, the same checkout writes there.
    let plain = Command::new("git")
        .args(["checkout-index", "-a", &prefix(&out)])
        .current_dir(&f.wt)
        .env("GIT_DIR", &git_dir)
        .env("GIT_WORK_TREE", &f.wt)
        .env("GIT_INDEX_FILE", f.index_dir.join("t.index"))
        .output()
        .unwrap();
    assert!(
        plain.status.success() && out.join("a.txt").exists(),
        "{plain:?}"
    );
}

#[test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
fn git_commit_works_confined() {
    let f = Fixture::new();
    let index = f.index_dir.join("git_commit.index");
    std::fs::write(f.wt.join("a.txt"), "changed\n").unwrap();
    std::fs::write(f.wt.join("new.rs"), "fn f() {}\n").unwrap();
    let staged = commit::stage(&f.dirs, &f.wt, "feat/x", None, &index).unwrap();
    assert_eq!(staged.changed, ["a.txt", "new.rs"]);
    commit::commit(&f.dirs, &f.wt, "feat/x", &staged, "m", &index).unwrap();
    let status = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&f.wt)
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&status.stdout), "");
}

#[test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
fn confined_git_still_honors_the_users_ignores_and_filters() {
    let f = Fixture::new();
    let index = f.index_dir.join("git_commit.index");
    std::fs::write(f.wt.join(".gitattributes"), "*.txt filter=probe\n").unwrap();
    std::fs::write(f.wt.join("a.txt"), "changed\n").unwrap();
    std::fs::write(f.wt.join("build.log"), "noise\n").unwrap();
    let staged = commit::stage(&f.dirs, &f.wt, "feat/x", None, &index).unwrap();
    assert_eq!(staged.changed, [".gitattributes", "a.txt"]);
    assert!(f.index_dir.join("global-filter-ran").exists());
}

/// A process the agent left running keeps swapping a dir in the worktree for a link to `dir`
/// while the daemon stages: `name` there, holding `secret`, never lands in the repo's objects.
fn race(f: &Fixture, dir: &Path, name: &str, secret: &str) {
    let index = f.index_dir.join("git_commit.index");
    let d = f.wt.join("d");
    let real = f.wt.join("d.real");
    std::fs::create_dir(&real).unwrap();
    // Same name and size as the secret's file, so a stat taken before the swap still matches.
    std::fs::write(real.join(name), "x".repeat(secret.len())).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let swapper = {
        let (stop, d, real, dir) = (stop.clone(), d.clone(), real.clone(), dir.to_path_buf());
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = std::fs::rename(&real, &d);
                let _ = std::fs::rename(&d, &real);
                let _ = std::os::unix::fs::symlink(&dir, &d);
                let _ = std::fs::remove_file(&d);
            }
        })
    };
    let secret_blob = {
        let out = Command::new("git")
            .args(["hash-object", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin.take().unwrap().write_all(secret.as_bytes())?;
                c.wait_with_output()
            })
            .unwrap();
        String::from_utf8(out.stdout).unwrap().trim().to_owned()
    };
    let start = Instant::now();
    let mut rounds = 0;
    while racing(start, rounds) {
        std::fs::write(f.wt.join("a.txt"), format!("round {rounds}\n")).unwrap();
        match commit::stage(&f.dirs, &f.wt, "feat/x", None, &index) {
            Ok(_) | Err(CommitError::NothingToCommit) | Err(CommitError::Git(_)) => {}
            Err(e) => panic!("{e}"),
        }
        rounds += 1;
    }
    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();
    let _ = std::fs::remove_file(&d);
    let _ = std::fs::remove_dir_all(&real);
    let _ = std::fs::remove_dir_all(&d);
    assert!(rounds >= ROUNDS, "only {rounds} rounds");
    let found = Command::new("git")
        .args(["cat-file", "-e", &secret_blob])
        .current_dir(&f.repo)
        .status()
        .unwrap();
    assert!(!found.success(), "{name} reached the repo's objects");
}

#[test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
fn a_link_swapped_in_while_staging_never_reaches_the_users_secrets() {
    let f = Fixture::new();
    let gitconfig = std::fs::read_to_string(f.home.join(".gitconfig")).unwrap();
    race(&f, &f.home.join(".ssh"), "id_test", SECRET);
    race(&f, &f.home.join(".config/git"), "credentials", CREDENTIALS);
    race(&f, &f.home, ".gitconfig", &gitconfig);
    race(&f, &f.home, "inc", INCLUDED);
}

/// A process the agent left running keeps swapping a worktree dir for a link to a dir outside
/// while the daemon rewinds back and forth: nothing outside is written or removed. Unconfined,
/// the swap wins only now and then; `confined_git_writes_only_what_it_was_given` checks the
/// same boundary every time.
#[test]
#[ignore = "needs a sandbox-capable host; run `just test-sandbox`"]
fn a_link_swapped_in_while_rewinding_never_writes_outside() {
    use forge_git::checkpoint;
    let f = Fixture::new();
    let index = checkpoint::index_path(&f.index_dir, "repo");
    let outside = f.home.join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("keep.txt"), "the user's\n").unwrap();
    let d = f.wt.join("d");
    std::fs::create_dir(&d).unwrap();
    std::fs::write(d.join("keep.txt"), "one\n").unwrap();
    let take = |n: u32, previous: Option<&str>| {
        checkpoint::take(
            &f.dirs,
            &f.wt,
            "feat/x",
            previous,
            &index,
            &checkpoint::checkpoint_ref("ses_1", n),
            "cp",
        )
    };
    let one = take(1, None).unwrap();
    std::fs::write(d.join("keep.txt"), "two\n").unwrap();
    std::fs::write(d.join("new.txt"), "two\n").unwrap();
    let two = take(2, Some(&one)).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let swapper = {
        let (stop, d, outside) = (stop.clone(), d.clone(), outside.clone());
        let aside = f.wt.join("d.aside");
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if std::fs::rename(&d, &aside).is_ok() {
                    let _ = std::os::unix::fs::symlink(&outside, &d);
                    let _ = std::fs::remove_file(&d);
                    let _ = std::fs::rename(&aside, &d);
                }
            }
        })
    };
    let start = Instant::now();
    let (mut n, mut previous, mut rounds) = (3, two.clone(), 0);
    while racing(start, rounds) {
        // Snapshot what's there now, then rewind to whichever checkpoint it isn't.
        let Ok(now) = take(n, Some(&previous)) else {
            continue;
        };
        n += 1;
        let to = if rounds % 2 == 0 { &one } else { &two };
        if checkpoint::rewind(&f.dirs, &f.wt, "feat/x", &now, to, &index).is_ok() {
            previous = to.clone();
        } else {
            previous = now;
        }
        rounds += 1;
    }
    stop.store(true, Ordering::Relaxed);
    swapper.join().unwrap();
    assert!(rounds >= ROUNDS, "only {rounds} rounds");
    let mut left: Vec<String> = std::fs::read_dir(&outside)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    left.sort();
    assert_eq!(left, ["keep.txt"], "wrote outside the worktree");
    assert_eq!(
        std::fs::read_to_string(outside.join("keep.txt")).unwrap(),
        "the user's\n"
    );
}
