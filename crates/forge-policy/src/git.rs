//! The built-in rule against manual git commands that write the repo's shared state.
//!
//! Inside the sandbox the shared git dir is read-only, so these commands would fail anyway;
//! the rule fails them early with a reason that points the agent at Forge's `git_commit` tool.
//! `commit` and `push` are always refused. `stash` and `branch` are refused except in their
//! read-only forms (`git stash list`, `git branch --list`, ...), since read-only git commands
//! stay allowed. A git alias defined on the command line, or a config file included there,
//! could name any subcommand, so they are refused too.
//!
//! Known gaps, left to the read-only git dir: other commands that write refs or config
//! (`git checkout -b`, `git switch -c`, `git tag`, `git update-ref`, `git worktree add`,
//! `git config`), and git configuration passed through the environment.

/// Why the rule refuses a command.
pub const REASON: &str = "git commit, push, stash, and branch change the repository's shared \
    state, which is read-only to agents; commit with Forge's git_commit tool instead (branches \
    are created and pushed by Forge)";

/// Global options that take a separate value: `git -C dir status`.
const GLOBAL_WITH_VALUE: &[&str] = &[
    "-C",
    "-c",
    "--git-dir",
    "--work-tree",
    "--namespace",
    "--super-prefix",
    "--config-env",
    "--exec-path",
    "--list-cmds",
    "--attr-source",
];

/// `git branch` flags that only list or describe branches.
const BRANCH_READ_ONLY: &[&str] = &[
    "--list",
    "--all",
    "--remotes",
    "--verbose",
    "--show-current",
    "--contains",
    "--no-contains",
    "--merged",
    "--no-merged",
    "--points-at",
    "--format",
    "--sort",
    "--color",
    "--no-color",
    "--column",
    "--no-column",
    "--abbrev",
    "--no-abbrev",
    "--ignore-case",
    "--omit-empty",
];
/// The single-letter forms of read-only `git branch` flags.
const BRANCH_READ_ONLY_SHORT: &[char] = &['l', 'a', 'r', 'v', 'i'];
/// `git branch` flags whose value may be the next argument.
const BRANCH_WITH_VALUE: &[&str] = &[
    "--contains",
    "--no-contains",
    "--points-at",
    "--format",
    "--sort",
];
/// `git branch` flags whose commit argument is optional.
const BRANCH_OPTIONAL_VALUE: &[&str] = &["--merged", "--no-merged"];

/// What the built-in rule says about one simple command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GitWrite {
    /// Not a git command that writes shared state.
    No,
    /// A git command that writes shared state.
    Yes,
    /// A git command whose subcommand, or the arguments that decide, are expansions.
    Maybe,
}

/// Judge `segment`, whose leading `literal` words are literal and the rest start with an
/// expansion.
pub fn check(segment: &crate::exec::Segment) -> GitWrite {
    let words = &segment.words;
    let writes = writes_shared_state(words);
    if segment.is_literal() || segment.literal == 0 {
        return if writes { GitWrite::Yes } else { GitWrite::No };
    }
    let Some((program, args)) = words.split_first() else {
        return GitWrite::No;
    };
    let sub = match program.strip_prefix("git-") {
        Some(sub) => sub,
        None if program == "git" => {
            // The first non-option word is the subcommand; if it, or an option before it, is
            // an expansion, it could be anything.
            let mut i = 0;
            loop {
                let Some(arg) = args.get(i) else {
                    return GitWrite::No;
                };
                if 1 + i >= segment.literal {
                    return GitWrite::Maybe;
                }
                if !arg.starts_with('-') {
                    break arg.as_str();
                }
                i += if GLOBAL_WITH_VALUE.contains(&arg.as_str()) {
                    2
                } else {
                    1
                };
            }
        }
        None => return GitWrite::No,
    };
    match sub {
        "commit" | "push" => GitWrite::Yes,
        // Their arguments decide, and one is an expansion.
        "stash" | "branch" => GitWrite::Maybe,
        _ if writes => GitWrite::Yes,
        _ => GitWrite::No,
    }
}

/// Whether `words` (one simple command, program name already reduced) is a git command that
/// writes shared state.
pub fn writes_shared_state(words: &[String]) -> bool {
    let Some((program, args)) = words.split_first() else {
        return false;
    };
    // `git-commit` and friends, from git's exec path, are the subcommands themselves.
    let (sub, rest) = match program.strip_prefix("git-") {
        Some(sub) => (sub, args),
        None if program == "git" => {
            if defines_alias(args) {
                // An alias can name any subcommand: `git -c alias.ci=commit ci`.
                return true;
            }
            match subcommand(args) {
                Some(found) => found,
                None => return false,
            }
        }
        None => return false,
    };
    match sub {
        "commit" | "push" => true,
        "stash" => !matches!(rest.first().map(String::as_str), Some("list" | "show")),
        "branch" => !branch_is_read_only(rest),
        _ => false,
    }
}

/// The subcommand and its arguments, after git's global options.
fn subcommand(args: &[String]) -> Option<(&str, &[String])> {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if !arg.starts_with('-') {
            return Some((arg.as_str(), &args[i + 1..]));
        }
        // `--opt=value` carries its own value; `--opt value` and `-C dir` take the next one.
        i += if GLOBAL_WITH_VALUE.contains(&arg.as_str()) {
            2
        } else {
            1
        };
    }
    None
}

/// Whether git's global options define an alias (`-c alias.x=...`, `--config-env=alias.x=...`),
/// or include a config file that could (`-c include.path=...`, `-c includeIf.<cond>.path=...`).
fn defines_alias(args: &[String]) -> bool {
    let is_alias = |value: &str| {
        let lower = value.to_ascii_lowercase();
        lower.starts_with("alias.") || lower.starts_with("include")
    };
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if !arg.starts_with('-') {
            return false;
        }
        if let Some(value) = arg.strip_prefix("--config-env=") {
            if is_alias(value) {
                return true;
            }
        } else if (arg == "-c" || arg == "--config-env")
            && args.get(i + 1).is_some_and(|v| is_alias(v))
        {
            return true;
        }
        i += if GLOBAL_WITH_VALUE.contains(&arg.as_str()) {
            2
        } else {
            1
        };
    }
    false
}

/// Whether `git branch <args>` only lists or describes branches.
fn branch_is_read_only(args: &[String]) -> bool {
    let mut listing = false;
    let mut names = false;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        i += 1;
        if arg == "--" {
            names |= i < args.len();
            break;
        }
        if let Some(long) = arg.strip_prefix("--") {
            let (flag, inline_value) = match long.split_once('=') {
                Some((flag, _)) => (format!("--{flag}"), true),
                None => (arg.clone(), false),
            };
            if !BRANCH_READ_ONLY.contains(&flag.as_str()) {
                return false;
            }
            listing |= flag == "--list";
            if !inline_value {
                let next_is_value = args.get(i).is_some_and(|n| !n.starts_with('-'));
                if BRANCH_WITH_VALUE.contains(&flag.as_str())
                    || (BRANCH_OPTIONAL_VALUE.contains(&flag.as_str()) && next_is_value)
                {
                    i += 1;
                }
            }
        } else if let Some(short) = arg.strip_prefix('-').filter(|s| !s.is_empty()) {
            if !short.chars().all(|c| BRANCH_READ_ONLY_SHORT.contains(&c)) {
                return false;
            }
            listing |= short.contains('l');
        } else {
            names = true;
        }
    }
    // Without --list, a name means "create this branch"; with it, names are patterns.
    listing || !names
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::CommandLine;

    fn judged(script: &str) -> Vec<GitWrite> {
        CommandLine::parse(&["bash".into(), "-c".into(), script.into()])
            .segments
            .iter()
            .map(check)
            .filter(|g| *g != GitWrite::No)
            .collect()
    }

    #[test]
    fn expansions_where_the_subcommand_goes_could_be_anything() {
        assert_eq!(judged("git $SUB"), [GitWrite::Maybe]);
        assert_eq!(judged("git -C $DIR $SUB"), [GitWrite::Maybe]);
        assert_eq!(judged("git -c $X status"), [GitWrite::Maybe]);
        assert_eq!(judged("git stash $ACTION"), [GitWrite::Maybe]);
        assert_eq!(judged("git branch $NAME"), [GitWrite::Maybe]);
        assert_eq!(judged("git push $REMOTE"), [GitWrite::Yes]);
        assert!(judged("git log $RANGE").is_empty());
        assert!(judged("git diff --stat $BASE").is_empty());
        assert!(judged("echo $X").is_empty());
    }

    fn writes(line: &str) -> bool {
        let words: Vec<String> = line.split_whitespace().map(str::to_owned).collect();
        writes_shared_state(&words)
    }

    #[test]
    fn commit_and_push_are_always_refused() {
        for line in [
            "git commit",
            "git commit -m x",
            "git commit --dry-run",
            "git push",
            "git push origin main --force",
            "git -C sub commit -am x",
            "git -c user.name=x commit",
            "git --git-dir=.git push",
            "git --git-dir .git --work-tree . push",
            "git --no-pager commit",
            "git -c alias.ci=commit ci",
            "git -c Alias.x=!sh x",
            "git --config-env=alias.ci=VAR ci",
            "git -c include.path=cfg ci",
            "git -c includeIf.gitdir:/.path=cfg ci",
            "git-commit -m x",
            "git-push",
            "git-stash pop",
        ] {
            assert!(writes(line), "{line}");
        }
    }

    #[test]
    fn stash_is_refused_except_list_and_show() {
        for line in [
            "git stash",
            "git stash push",
            "git stash pop",
            "git stash apply",
            "git stash drop",
            "git stash clear",
            "git stash -u",
            "git stash save x",
        ] {
            assert!(writes(line), "{line}");
        }
        for line in [
            "git stash list",
            "git stash show",
            "git stash show -p stash@{0}",
        ] {
            assert!(!writes(line), "{line}");
        }
    }

    #[test]
    fn branch_is_refused_except_listing() {
        for line in [
            "git branch new",
            "git branch -d old",
            "git branch -D old",
            "git branch -m a b",
            "git branch -c a b",
            "git branch -f main HEAD~1",
            "git branch --set-upstream-to=origin/x",
            "git branch -u origin/x",
            "git branch --unset-upstream",
            "git branch --edit-description",
            "git branch --track x origin/x",
            "git branch -a new",
            "git branch -r new",
            "git branch -vd old",
            "git branch -- new",
            "git branch --delete old",
        ] {
            assert!(writes(line), "{line}");
        }
        for line in [
            "git branch",
            "git branch --list",
            "git branch -l",
            "git branch --list feat/*",
            "git branch -l 'feat/*'",
            "git branch -a",
            "git branch -r",
            "git branch -av",
            "git branch -vv",
            "git branch --show-current",
            "git branch --contains abc123",
            "git branch --contains=abc123",
            "git branch --merged",
            "git branch --merged main",
            "git branch --no-merged main -a",
            "git branch --format %(refname) --sort -committerdate",
            "git branch --points-at HEAD",
            "git -C sub branch --show-current",
        ] {
            assert!(!writes(line), "{line}");
        }
    }

    #[test]
    fn other_git_commands_and_programs_pass() {
        for line in [
            "git status",
            "git diff --stat",
            "git log --branches",
            "git show HEAD",
            "git blame src/a.rs",
            "git",
            "git --version",
            "gitk --all",
            "echo git push",
            "git -c core.pager=less log",
            "git-log",
            "git-stash list",
        ] {
            assert!(!writes(line), "{line}");
        }
    }
}
