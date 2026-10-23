//! Commands as policy sees them: the line an allow rule matches, the simple commands deny and
//! ask rules look inside, and whether Forge can tell what runs at all.
//!
//! An argv runs one program, but that program may be a shell running a script, or a wrapper
//! (`env`, `timeout`, `xargs`, `find -exec`, ...) running another program. Scripts are parsed
//! with [`crate::shell`], and each simple command in them has its variable assignments and
//! wrappers stripped, so `env FOO=1 timeout 5 sh -c 'g""it >/dev/null push'` is seen as
//! `git push`. Over-matching here only over-denies.
//!
//! When the program that runs can't be known (an expansion in command position, `eval` of a
//! variable, an alias, broken quoting, shells nested too deep), the line is *opaque* and the
//! policy engine asks. An expansion in an argument (`cargo test $FLAGS`) is not: it can only
//! change the arguments, so it is matched by its literal prefix, and a deny or ask rule that
//! the expansion could complete makes the engine ask.
//!
//! An allow rule matches only a line that is exactly one simple command: no shell syntax, no
//! redirection, no wrapper or assignment, no interpreter running inline code, no option that
//! runs another program (`rg --pre`, `find -exec`), no shell that reads startup files, no
//! shell inside a shell, nothing but ASCII spaces and tabs between words, and a program word
//! that is a plain literal name found through `PATH`, or a path in a system directory, never
//! one the agent could have written (`./cargo`).
//!
//! All of this is lexical and best effort; the sandbox is the backstop for what it misses.

use crate::shell::{self, Word};

/// Programs that run their `-c` argument as a script.
const SHELLS: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "dash",
    "ksh",
    "mksh",
    "yash",
    "fish",
    "ash",
    "csh",
    "tcsh",
    "pwsh",
    "powershell",
];

/// Shells that read startup files from the (sandbox-writable) home even for `-c`.
const RC_SHELLS: &[&str] = &["zsh", "fish", "csh", "tcsh"];

/// Interpreters and the arguments that make them run code given inline. An empty list means
/// every invocation does (awk's program is an argument).
const INTERPRETERS: &[(&str, &[&str])] = &[
    ("python", &["-c"]),
    ("python2", &["-c"]),
    ("python3", &["-c"]),
    ("pypy", &["-c"]),
    ("pypy3", &["-c"]),
    ("node", &["-e", "--eval", "-p", "--print"]),
    ("nodejs", &["-e", "--eval", "-p", "--print"]),
    ("bun", &["-e", "--eval", "-p", "--print"]),
    ("deno", &["eval"]),
    ("perl", &["-e", "-E"]),
    ("ruby", &["-e"]),
    ("php", &["-r"]),
    ("lua", &["-e"]),
    ("osascript", &["-e"]),
    ("rscript", &["-e"]),
    ("awk", &[]),
    ("gawk", &[]),
    ("mawk", &[]),
    ("nawk", &[]),
];

/// Options that make a tool people think of as read-only run another program. An empty list
/// means the tool can always do so (pagers and editors have shell escapes). `sed` is handled
/// on its own: its `e` command runs anything unless `--sandbox` is given.
const UNSAFE_OPTIONS: &[(&str, &[&str])] = &[
    ("rg", &["--pre"]),
    ("grep", &["--pre"]),
    (
        "find",
        &[
            "-exec", "-execdir", "-ok", "-okdir", "-delete", "-fprint", "-fprintf", "-fls",
        ],
    ),
    (
        "git",
        &[
            "-c",
            "--config-env",
            "-O",
            "--open-files-in-pager",
            "--exec-path",
        ],
    ),
    (
        "tar",
        &[
            "-I",
            "--use-compress-program",
            "--to-command",
            "--rmt-command",
            "--checkpoint-action",
            "--info-script",
            "--new-volume-script",
            "-F",
        ],
    ),
    ("sort", &["--compress-program"]),
    ("less", &[]),
    ("more", &[]),
    ("man", &[]),
    ("vi", &[]),
    ("vim", &[]),
    ("nvim", &[]),
    ("emacs", &[]),
];

/// Programs that run another program given as an argument: the wrapper's options that take a
/// value, and how many positional arguments come before the wrapped command.
const WRAPPERS: &[(&str, &[&str], usize)] = &[
    ("env", &["-u", "--unset", "-C", "--chdir"], 0),
    ("command", &[], 0),
    ("exec", &["-a"], 0),
    ("builtin", &[], 0),
    ("nice", &["-n", "--adjustment"], 0),
    ("nohup", &[], 0),
    ("time", &["-f", "--format", "-o", "--output"], 0),
    ("timeout", &["-s", "--signal", "-k", "--kill-after"], 1),
    (
        "stdbuf",
        &["-i", "-o", "-e", "--input", "--output", "--error"],
        0,
    ),
    (
        "ionice",
        &["-c", "--class", "-n", "--classdata", "-p", "--pid"],
        0,
    ),
    ("setsid", &[], 0),
    ("chrt", &[], 1),
    ("taskset", &[], 1),
    (
        "flock",
        &["-w", "--timeout", "-E", "--conflict-exit-code"],
        1,
    ),
    (
        "sudo",
        &[
            "-u", "-g", "-h", "-p", "-C", "-D", "-r", "-t", "-U", "-T", "--user", "--group",
        ],
        0,
    ),
    ("doas", &["-u", "-C"], 0),
    (
        "xargs",
        &[
            "-I",
            "-L",
            "-n",
            "-P",
            "-s",
            "-d",
            "-E",
            "-a",
            "--max-args",
            "--max-procs",
            "--delimiter",
            "--arg-file",
            "--max-lines",
            "--max-chars",
        ],
        0,
    ),
    ("busybox", &[], 0),
    (
        "strace",
        &[
            "-e", "-o", "-p", "-s", "-u", "-E", "-P", "-a", "-b", "-I", "-O", "-S", "-X",
        ],
        0,
    ),
    (
        "ltrace",
        &["-e", "-o", "-p", "-s", "-u", "-n", "-a", "-l"],
        0,
    ),
    ("chroot", &["--userspec", "--groups"], 1),
    (
        "unshare",
        &[
            "-S",
            "-G",
            "-R",
            "-w",
            "--setuid",
            "--setgid",
            "--root",
            "--wd",
            "--map-user",
            "--map-group",
        ],
        0,
    ),
    (
        "nsenter",
        &[
            "-t", "--target", "-S", "--setuid", "-G", "--setgid", "--root", "--wd",
        ],
        0,
    ),
    (
        "setpriv",
        &[
            "--reuid",
            "--regid",
            "--groups",
            "--inh-caps",
            "--ambient-caps",
            "--bounding-set",
            "--securebits",
            "--selinux-label",
            "--apparmor-profile",
            "--pdeathsig",
        ],
        0,
    ),
    (
        "parallel",
        &[
            "-j",
            "--jobs",
            "-S",
            "--sshlogin",
            "--joblog",
            "-a",
            "--arg-file",
            "--colsep",
            "-d",
            "--delimiter",
            "-I",
            "--replace",
        ],
        0,
    ),
    (
        "systemd-run",
        &[
            "-u",
            "--unit",
            "-p",
            "--property",
            "-E",
            "--setenv",
            "--slice",
            "--uid",
            "--gid",
            "--working-directory",
            "-M",
            "--machine",
            "--description",
        ],
        0,
    ),
    ("caffeinate", &["-t", "-w"], 0),
];

/// `find` actions that run a command, up to `;` or `+`.
const FIND_EXEC: &[&str] = &["-exec", "-execdir", "-ok", "-okdir"];

/// Directories whose programs the agent can't have written.
const TRUSTED_DIRS: &[&str] = &[
    "/usr/bin",
    "/bin",
    "/usr/sbin",
    "/sbin",
    "/usr/local/bin",
    "/usr/local/sbin",
    "/opt/homebrew/bin",
    "/opt/homebrew/sbin",
];

/// Text in an argv element that suggests the argv is really a shell line.
const SHELL_SYNTAX: &[&str] = &[
    "&&", "||", ";", "|", "&", "`", "$(", "${", ">", "<", "\n", "\r", "(", ")",
];

/// How deep shells inside shells are parsed.
const MAX_DEPTH: usize = 4;

/// One simple command, as deny and ask rules see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Its words, assignments and wrappers stripped, quoting removed, the program reduced to
    /// its file name (and to lower case on macOS, whose filesystems usually ignore case).
    /// Expansions appear as written.
    pub words: Vec<String>,
    /// How many leading words are literal; the rest start with an expansion.
    pub literal: usize,
    /// The literal start of the first word with an expansion (`target/` of `target/*`).
    pub partial: String,
}

impl Segment {
    pub fn line(&self) -> String {
        self.words.join(" ")
    }

    pub fn is_literal(&self) -> bool {
        self.literal == self.words.len()
    }

    /// Everything before the first expansion: the literal words, joined with spaces, and the
    /// literal start of the word the expansion is in.
    pub fn literal_prefix(&self) -> String {
        let mut prefix = self.words[..self.literal].join(" ");
        if self.literal > 0 {
            prefix.push(' ');
        }
        prefix.push_str(&self.partial);
        prefix
    }

    /// The first word with an expansion, if any.
    pub fn first_expansion(&self) -> Option<&str> {
        self.words.get(self.literal).map(String::as_str)
    }
}

/// A command line as policy sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandLine {
    /// The line an allow rule matches: the words of the one simple command, with the program
    /// reduced to its file name. For a line that isn't simple, the argv joined as given.
    pub line: String,
    /// The words of the one simple command an allow rule may match, when the line is simple:
    /// what runs, argument by argument.
    pub words: Vec<String>,
    /// Each simple command in the line.
    pub segments: Vec<Segment>,
    /// Whether an allow rule may match `line`.
    pub simple: bool,
    /// Why Forge can't tell what the line runs, if it can't.
    pub opaque: Option<String>,
}

impl CommandLine {
    pub fn parse(argv: &[String]) -> Self {
        let mut ctx = Ctx::default();
        let words: Vec<Word> = argv.iter().map(Word::plain).collect();
        let mut allow_line = command(&words, 0, false, &mut ctx);
        let runs_script = argv
            .first()
            .is_some_and(|p| SHELLS.contains(&program_name(p).as_str()));
        if !runs_script && argv.iter().any(|a| has_shell_syntax(a)) {
            // An argv element with shell syntax may mean the argv is really a shell line (some
            // agents describe commands that way in permission requests). Its simple commands
            // count for deny and ask rules too, but what it can't parse doesn't make the
            // literal argv opaque.
            let mut extra = Ctx::default();
            script(&argv.join(" "), 1, false, &mut extra);
            ctx.segments.extend(extra.segments);
            allow_line = None;
        }
        let simple = allow_line.is_some() && ctx.opaque.is_none();
        let words = allow_line.filter(|_| simple).unwrap_or_default();
        Self {
            line: if simple {
                words.join(" ")
            } else {
                argv.join(" ")
            },
            words,
            segments: ctx.segments,
            simple,
            opaque: ctx.opaque,
        }
    }

    /// Each segment's words joined with spaces.
    pub fn segment_lines(&self) -> impl Iterator<Item = String> + '_ {
        self.segments.iter().map(Segment::line)
    }
}

#[derive(Default)]
struct Ctx {
    segments: Vec<Segment>,
    opaque: Option<String>,
}

impl Ctx {
    fn opaque(&mut self, why: String) {
        self.opaque.get_or_insert(why);
    }

    fn push(&mut self, words: &[Word]) {
        let literal = words.iter().take_while(|w| w.is_literal()).count();
        let partial = words
            .get(literal)
            .map(|w| w.literal_part().to_owned())
            .unwrap_or_default();
        let mut texts: Vec<String> = words.iter().map(|w| w.text.clone()).collect();
        if let Some(first) = texts.first_mut() {
            *first = program_name(first);
        }
        self.segments.push(Segment {
            words: texts,
            literal,
            partial,
        });
    }
}

/// Record the simple commands in `words` (one command) and return the line an allow rule may
/// match, if the command is simple. `in_shell` is set inside a shell's script.
fn command(words: &[Word], depth: usize, in_shell: bool, ctx: &mut Ctx) -> Option<Vec<String>> {
    if words.is_empty() {
        return None;
    }
    if depth > MAX_DEPTH {
        ctx.opaque(format!("shells are nested more than {MAX_DEPTH} deep"));
        ctx.push(words);
        return None;
    }
    let (inner, wrapped) = unwrap(words);
    let Some(program) = inner.first() else {
        ctx.push(words);
        return None;
    };
    if !program.is_literal() {
        ctx.opaque(format!(
            "{:?} is an expansion in command position, so Forge can't tell what runs",
            program.text
        ));
        ctx.push(&inner);
        return None;
    }
    let name = program_name(&program.text);
    let args = &inner[1..];
    ctx.push(&inner);
    match name.as_str() {
        "eval" => {
            if args.iter().all(|w| w.is_literal()) {
                let text: Vec<&str> = args.iter().map(|w| w.text.as_str()).collect();
                script(&text.join(" "), depth + 1, true, ctx);
            } else {
                ctx.opaque("eval runs a script built from expansions".into());
            }
            return None;
        }
        "trap" => {
            // Its first argument after the options is code, run on a signal or exit.
            let code = args
                .iter()
                .find(|w| !(w.text.starts_with('-') && w.text.len() > 1 && w.is_literal()));
            if let Some(code) = code {
                run_script_word(code, depth, ctx);
            }
            return None;
        }
        "bind" | "complete" | "compgen" | "mapfile" | "readarray" => {
            // `bind -x` and `-C` options name code to run.
            let flag = if name == "bind" { "-x" } else { "-C" };
            let mut i = 0;
            while let Some(arg) = args.get(i) {
                i += 1;
                if arg.text == flag {
                    match args.get(i) {
                        Some(code) => run_script_word(code, depth, ctx),
                        None => ctx.opaque(format!("{name} {flag} without its command")),
                    }
                } else if arg.text.starts_with(flag) && arg.text.len() > flag.len() {
                    ctx.opaque(format!("{name} runs code given with {flag}"));
                }
            }
            return None;
        }
        "source" | "." if args.first().is_none_or(|w| !w.is_literal()) => {
            ctx.opaque(format!("{name} runs a script Forge can't see"));
            return None;
        }
        "hash" if args.iter().any(|w| w.text.starts_with("-p")) => {
            ctx.opaque("hash -p makes a name run another program".into());
            return None;
        }
        "enable" if args.iter().any(|w| w.text.starts_with("-f")) => {
            ctx.opaque("enable -f loads a shared object into the shell".into());
            return None;
        }
        "alias" | "unalias" => {
            ctx.opaque("aliases can make any word run anything".into());
            return None;
        }
        "shopt" if args.iter().any(|w| w.text == "expand_aliases") => {
            ctx.opaque("aliases can make any word run anything".into());
            return None;
        }
        "watch" => {
            watch(args, depth, ctx);
            return None;
        }
        "find" => find_exec(args, depth, ctx),
        _ => {}
    }
    if SHELLS.contains(&name.as_str()) || name == "su" || name == "script" {
        match runner_args(&name, args) {
            Runner::Script {
                script: text,
                rc,
                init,
            } => {
                if let Some(init) = init {
                    run_script_word(init, depth, ctx);
                }
                if !text.is_literal() {
                    ctx.opaque("the shell's script is an expansion".into());
                    return None;
                }
                let line = script(&text.text, depth + 1, true, ctx);
                let shell = SHELLS.contains(&name.as_str());
                let plain = words.iter().all(|w| w.is_literal() && plain_text(&w.text))
                    && trusted(&words[0]);
                return line.filter(|_| shell && plain && !wrapped && !rc && !in_shell);
            }
            Runner::Opaque(why) => {
                ctx.opaque(why.into());
                return None;
            }
            Runner::Broken => return None,
            Runner::Stdin => {
                ctx.opaque(format!(
                    "{name} reads its commands from standard input, which may be data turned \
                     into code"
                ));
                return None;
            }
            // A shell running a script file is an ordinary command.
            Runner::File => {}
        }
    }
    let simple = words.iter().all(|w| w.is_literal() && plain_text(&w.text))
        && trusted(&words[0])
        && !wrapped
        && !runs_code(&name, args);
    simple.then(|| {
        let mut line = vec![basename(&program.text).to_owned()];
        line.extend(args.iter().map(|w| w.text.clone()));
        line
    })
}

/// Record the simple commands in a shell script and return the line an allow rule may match,
/// if the script is one simple command.
fn script(text: &str, depth: usize, in_shell: bool, ctx: &mut Ctx) -> Option<Vec<String>> {
    if depth > MAX_DEPTH {
        ctx.opaque(format!("shells are nested more than {MAX_DEPTH} deep"));
        return None;
    }
    let parsed = shell::parse(text);
    if let Some(problem) = parsed.problem {
        ctx.opaque(format!("the script has {problem}"));
    }
    for nested in &parsed.nested {
        script(nested, depth + 1, false, ctx);
    }
    let mut lines: Vec<Option<Vec<String>>> = parsed
        .commands
        .iter()
        .map(|c| command(&c.words, depth, in_shell, ctx))
        .collect();
    let one = parsed.commands.len() == 1
        && !parsed.compound
        && parsed.nested.is_empty()
        && !parsed.commands[0].redirected
        && plain_text(text);
    if one { lines.pop().flatten() } else { None }
}

fn run_script_word(word: &Word, depth: usize, ctx: &mut Ctx) {
    if word.is_literal() {
        script(&word.text, depth + 1, true, ctx);
    } else {
        ctx.opaque("the shell's script is an expansion".into());
    }
}

/// `watch` runs its arguments as a shell line, or as an argv with `-x`.
fn watch(args: &[Word], depth: usize, ctx: &mut Ctx) {
    const WITH_VALUE: &[&str] = &["-n", "--interval", "-q", "--equexit", "--differences"];
    let mut exec = false;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        if arg.text == "--" {
            i += 1;
            break;
        }
        if !arg.text.starts_with('-') || arg.text.len() < 2 {
            break;
        }
        exec |= arg.text == "-x" || arg.text == "--exec";
        i += if WITH_VALUE.contains(&arg.text.as_str()) {
            2
        } else {
            1
        };
    }
    let rest = args.get(i..).unwrap_or(&[]);
    if exec {
        command(rest, depth + 1, false, ctx);
    } else if rest.iter().all(|w| w.is_literal()) {
        let text: Vec<&str> = rest.iter().map(|w| w.text.as_str()).collect();
        script(&text.join(" "), depth + 1, true, ctx);
    } else {
        ctx.opaque("watch runs a shell line built from expansions".into());
    }
}

/// Record the commands `find -exec ... ;` and its relatives run.
fn find_exec(args: &[Word], depth: usize, ctx: &mut Ctx) {
    let mut i = 0;
    while i < args.len() {
        if FIND_EXEC.contains(&args[i].text.as_str()) {
            let start = i + 1;
            let end = args[start..]
                .iter()
                .position(|w| w.text == ";" || w.text == "+")
                .map_or(args.len(), |p| start + p);
            let mut run = args[start..end].to_vec();
            mark_expansion(&mut run, "{}");
            command(&run, depth + 1, false, ctx);
            i = end;
        }
        i += 1;
    }
}

enum Runner<'a> {
    /// A script given inline; `rc` if the program also reads startup files first, and `init`
    /// for a second script it runs before (fish's `-C`).
    Script {
        script: &'a Word,
        rc: bool,
        init: Option<&'a Word>,
    },
    /// Code Forge can't read, such as an encoded PowerShell command.
    Opaque(&'static str),
    /// `-c` without a script.
    Broken,
    /// No script at all: reads commands from standard input, which may be data
    /// (`echo cmd | sh`, `bash <<< cmd`).
    Stdin,
    /// No inline script: runs a script file, or reads commands from stdin.
    File,
}

/// Find the inline script among a shell's (or `su`'s, or `script`'s) leading options.
fn runner_args<'a>(name: &str, args: &'a [Word]) -> Runner<'a> {
    match name {
        "pwsh" | "powershell" => pwsh_args(args),
        "su" | "script" => {
            let mut i = 0;
            while let Some(arg) = args.get(i) {
                i += 1;
                if arg.text.starts_with("--command=") {
                    return Runner::Opaque(
                        "an inline command Forge can't separate from its option",
                    );
                }
                if arg.text == "-c" || arg.text == "--command" {
                    return match args.get(i) {
                        Some(script) => Runner::Script {
                            script,
                            rc: true,
                            init: None,
                        },
                        None => Runner::Broken,
                    };
                }
            }
            Runner::Stdin
        }
        _ => shell_args(name, args),
    }
}

fn shell_args<'a>(name: &str, args: &'a [Word]) -> Runner<'a> {
    let (mut inline, mut rc, mut stdin) = (false, RC_SHELLS.contains(&name), false);
    let mut init = None;
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        i += 1;
        match arg.text.as_str() {
            "--" => break,
            "-o" | "+o" | "-O" | "+O" => i += 1,
            "--rcfile" | "--init-file" => {
                rc = true;
                i += 1;
            }
            "--login" | "--interactive" => rc = true,
            // fish's long form of -c.
            "--command" => {
                inline = true;
                break;
            }
            // fish's init command, which runs first.
            "-C" | "--init-command" if name == "fish" => {
                init = args.get(i);
                rc = true;
                i += 1;
            }
            t if t.starts_with("--command=") || t.starts_with("--init-command=") => {
                return Runner::Opaque("an inline command Forge can't separate from its option");
            }
            t if t.starts_with("--") => {}
            t if (t.starts_with('-') || t.starts_with('+')) && t.len() > 1 => {
                inline |= t.contains('c');
                stdin |= t.starts_with('-') && t.contains('s');
                rc |= t.contains('i') || t.contains('l');
            }
            _ => {
                i -= 1;
                break;
            }
        }
    }
    match (inline, args.get(i)) {
        (true, Some(script)) => Runner::Script { script, rc, init },
        (true, None) => Runner::Broken,
        (false, file) => match (init, file) {
            (Some(init), _) => Runner::Script {
                script: init,
                rc: true,
                init: None,
            },
            _ if stdin => Runner::Stdin,
            (None, None) => Runner::Stdin,
            (None, Some(file)) if !file.is_literal() => {
                Runner::Opaque("a shell running a script file named by an expansion")
            }
            (None, Some(_)) => Runner::File,
        },
    }
}

/// PowerShell's options are case-insensitive.
fn pwsh_args(args: &[Word]) -> Runner<'_> {
    let rc = !args.iter().any(|a| {
        let lower = a.text.to_ascii_lowercase();
        lower == "-noprofile" || lower == "-nop"
    });
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        i += 1;
        let lower = arg.text.to_ascii_lowercase();
        match lower.as_str() {
            "-c" | "-command" => {
                return match args.get(i) {
                    Some(script) => Runner::Script {
                        script,
                        rc,
                        init: None,
                    },
                    None => Runner::Broken,
                };
            }
            "-e" | "-ec" | "-enc" | "-encodedcommand" => {
                return Runner::Opaque("an encoded PowerShell command");
            }
            "-f" | "-file" => {
                return match args.get(i) {
                    Some(file) if file.is_literal() => Runner::File,
                    Some(_) => Runner::Opaque("a script file named by an expansion"),
                    None => Runner::Broken,
                };
            }
            l if l.starts_with('-') => {}
            _ if arg.is_literal() => return Runner::File,
            _ => return Runner::Opaque("a script file named by an expansion"),
        }
    }
    Runner::Stdin
}

/// The command a line of wrappers and variable assignments runs: `FOO=1 env -u X timeout 5
/// git push` → `git push`; and whether there were any.
fn unwrap(words: &[Word]) -> (Vec<Word>, bool) {
    let mut rest: Vec<Word> = words.to_vec();
    let mut wrapped = false;
    loop {
        let skip = rest.iter().take_while(|w| is_assignment(&w.text)).count();
        if skip > 0 {
            rest.drain(..skip);
            wrapped = true;
        }
        let Some(first) = rest.first() else {
            return (rest, wrapped);
        };
        if !first.is_literal() {
            return (rest, wrapped);
        }
        let name = program_name(&first.text);
        if name == "env"
            && let Some(split) = env_split(&rest[1..])
        {
            rest = split;
            wrapped = true;
            continue;
        }
        let Some((_, with_value, positional)) = WRAPPERS.iter().find(|(w, ..)| *w == name) else {
            return (rest, wrapped);
        };
        let feeds_stdin = name == "xargs" || name == "parallel";
        let mut placeholder = (name == "parallel").then(|| "{}".to_owned());
        let mut i = 1;
        while let Some(arg) = rest.get(i) {
            if !arg.is_literal() {
                break;
            }
            if arg.text == "--" {
                i += 1;
                break;
            }
            if feeds_stdin {
                match arg.text.as_str() {
                    "-I" | "--replace" => placeholder = rest.get(i + 1).map(|w| w.text.clone()),
                    "-i" => placeholder = Some("{}".into()),
                    t if t.starts_with("-I") => placeholder = Some(t[2..].to_owned()),
                    t if t.starts_with("--replace=") => placeholder = Some(t[10..].to_owned()),
                    t if t.starts_with("-i") => placeholder = Some(t[2..].to_owned()),
                    _ => {}
                }
            }
            if is_assignment(&arg.text) && name == "env" {
                i += 1;
            } else if arg.text.starts_with('-') && arg.text.len() > 1 {
                let takes_value = with_value.contains(&arg.text.as_str());
                i += if takes_value { 2 } else { 1 };
            } else {
                break;
            }
        }
        i += positional;
        rest.drain(..i.min(rest.len()));
        wrapped = true;
        if feeds_stdin {
            // The arguments come from standard input: in place of a placeholder, or appended.
            if let Some(placeholder) = &placeholder {
                mark_expansion(&mut rest, placeholder);
            }
            rest.push(Word {
                text: STDIN_ARGS.into(),
                literal_len: 0,
            });
        }
    }
}

/// Stands for the arguments `xargs` and `parallel` read from standard input.
const STDIN_ARGS: &str = "<arguments from stdin>";

/// End the literal part of each word at the first `placeholder` in it: `{}` in `find -exec`,
/// or the `-I` string of `xargs`, which the program's input replaces.
fn mark_expansion(words: &mut [Word], placeholder: &str) {
    if placeholder.is_empty() {
        return;
    }
    for word in words {
        if let Some(at) = word.text.find(placeholder) {
            word.literal_len = word.literal_len.min(at);
        }
    }
}

/// `env -S 'cmd args'` (or `--split-string`): the words it runs, with the string split as a
/// shell would and any arguments after it appended.
fn env_split(args: &[Word]) -> Option<Vec<Word>> {
    let mut i = 0;
    while let Some(arg) = args.get(i) {
        i += 1;
        let value = if arg.text == "-S" || arg.text == "--split-string" {
            let value = args.get(i)?;
            i += 1;
            value.clone()
        } else if let Some(v) = arg.text.strip_prefix("--split-string=") {
            Word {
                literal_len: arg.literal_len.saturating_sub(arg.text.len() - v.len()),
                text: v.to_owned(),
            }
        } else if let Some(v) = arg.text.strip_prefix("-S").filter(|v| !v.is_empty()) {
            Word {
                literal_len: arg.literal_len.saturating_sub(arg.text.len() - v.len()),
                text: v.to_owned(),
            }
        } else if arg.text.starts_with('-') || is_assignment(&arg.text) {
            continue;
        } else {
            return None;
        };
        let mut words: Vec<Word> = if value.is_literal() {
            shell::parse(&value.text)
                .commands
                .into_iter()
                .flat_map(|c| c.words)
                .collect()
        } else {
            vec![value]
        };
        words.extend(args[i..].iter().cloned());
        return Some(words);
    }
    None
}

/// `NAME=value`, a shell variable assignment.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Whether a simple command runs code given inline or through an option.
fn runs_code(name: &str, args: &[Word]) -> bool {
    if name == "sed" {
        return !args.iter().any(|a| a.text == "--sandbox");
    }
    let base = interpreter_name(name);
    let Some((_, flags)) = INTERPRETERS
        .iter()
        .chain(UNSAFE_OPTIONS)
        .find(|(n, _)| *n == base)
    else {
        return false;
    };
    flags.is_empty()
        || args
            .iter()
            .any(|arg| flags.iter().any(|flag| has_flag(&arg.text, flag)))
}

/// Whether `arg` gives `flag`: exactly, as `--flag=value`, or (for a short flag) in a cluster
/// such as `-Bc`. Long flags ignore case, for PowerShell-style options; short ones don't.
fn has_flag(arg: &str, flag: &str) -> bool {
    let short = flag.len() == 2 && flag.starts_with('-') && !flag.starts_with("--");
    if short {
        return arg == flag || is_flag_with(arg, flag.chars().nth(1).expect("two chars"));
    }
    let (arg, flag) = (arg.to_ascii_lowercase(), flag.to_ascii_lowercase());
    arg == flag || arg.starts_with(&format!("{flag}="))
}

fn has_shell_syntax(text: &str) -> bool {
    SHELL_SYNTAX.iter().any(|op| text.contains(op))
}

/// Text with no control characters and no whitespace but ASCII spaces and tabs, so words
/// split here the way a shell splits them.
fn plain_text(text: &str) -> bool {
    text.chars()
        .all(|c| c == ' ' || c == '\t' || !(c.is_control() || c.is_whitespace()))
}

/// Whether a program word is one the agent can't have written: a plain name found through
/// `PATH`, or a path in a system directory.
fn trusted(program: &Word) -> bool {
    if !program.is_literal() {
        return false;
    }
    match program.text.rsplit_once('/') {
        None => is_plain_name(&program.text),
        Some((dir, name)) => TRUSTED_DIRS.contains(&dir) && is_plain_name(name),
    }
}

/// A program name with nothing a shell would expand or split: `cargo`, `pg_dump`, `g++`.
fn is_plain_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
}

/// `-c`, or a cluster of short flags containing `c`, such as `-lc`.
fn is_flag_with(arg: &str, flag: char) -> bool {
    arg.strip_prefix('-')
        .is_some_and(|f| !f.starts_with('-') && !f.is_empty() && f.contains(flag))
}

/// `python3.12` → `python3`.
fn interpreter_name(name: &str) -> &str {
    match name.split_once('.') {
        Some((head, tail)) if tail.chars().all(|c| c.is_ascii_digit()) => head,
        _ => name,
    }
}

/// The file name of a program path: `/usr/bin/git` → `git`.
pub fn basename(program: &str) -> &str {
    program.rsplit('/').next().unwrap_or(program)
}

/// A program's name for matching: its file name, in lower case on macOS.
fn program_name(program: &str) -> String {
    let name = basename(program);
    if cfg!(target_os = "macos") {
        name.to_lowercase()
    } else {
        name.to_owned()
    }
}

/// An `exec` pattern: `*` matches any run of characters, including none; `\*` and `\\` are a
/// literal `*` and `\`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecGlob {
    pattern: String,
    /// Literal pieces between the stars.
    parts: Vec<String>,
}

impl ExecGlob {
    pub fn new(pattern: &str) -> Self {
        let mut parts = vec![String::new()];
        let mut chars = pattern.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some(next) => parts.last_mut().expect("non-empty").push(next),
                    None => parts.last_mut().expect("non-empty").push('\\'),
                },
                '*' => parts.push(String::new()),
                c => parts.last_mut().expect("non-empty").push(c),
            }
        }
        Self {
            pattern: pattern.to_owned(),
            parts,
        }
    }

    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    pub fn matches(&self, text: &str) -> bool {
        let (first, rest) = self.parts.split_first().expect("non-empty");
        let Some(mut text) = text.strip_prefix(first.as_str()) else {
            return false;
        };
        let Some((last, middle)) = rest.split_last() else {
            return text.is_empty();
        };
        for part in middle {
            match text.find(part.as_str()) {
                Some(i) => text = &text[i + part.len()..],
                None => return false,
            }
        }
        text.ends_with(last.as_str())
    }

    /// Whether every text starting with `prefix` matches: the pattern ends in `*` and its
    /// literal pieces are all in `prefix` already, in order. Then no expansion after the prefix
    /// can stop the match.
    pub fn matches_prefix_certainly(&self, prefix: &str) -> bool {
        let Some((last, rest)) = self.parts.split_last() else {
            return false;
        };
        if self.parts.len() < 2 || !last.is_empty() {
            return false;
        }
        let (first, middle) = rest.split_first().expect("two parts");
        let Some(mut text) = prefix.strip_prefix(first.as_str()) else {
            return false;
        };
        for part in middle {
            match text.find(part.as_str()) {
                Some(i) => text = &text[i + part.len()..],
                None => return false,
            }
        }
        true
    }

    /// Whether some text starting with `prefix` matches: what an expansion after a literal
    /// prefix could turn into.
    pub fn could_match_prefix(&self, prefix: &str) -> bool {
        let first = &self.parts[0];
        if self.parts.len() == 1 {
            return first.starts_with(prefix);
        }
        // After the first star anything can follow, and the expansion supplies it.
        prefix.starts_with(first.as_str()) || first.starts_with(prefix)
    }
}

/// Escape `text` so an [`ExecGlob`] matches exactly it.
pub fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace('*', "\\*")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&str]) -> CommandLine {
        CommandLine::parse(&argv.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    }

    fn segments(argv: &[&str]) -> Vec<String> {
        parse(argv).segment_lines().collect()
    }

    fn has(argv: &[&str], line: &str) -> bool {
        segments(argv).iter().any(|l| l == line)
    }

    #[test]
    fn plain_commands_are_simple() {
        let c = parse(&["/usr/bin/cargo", "test", "--all"]);
        assert_eq!(c.line, "cargo test --all");
        assert!(c.simple);
        assert_eq!(
            segments(&["/usr/bin/cargo", "test", "--all"]),
            ["cargo test --all"]
        );
        assert!(parse(&["pg_dump", "db"]).simple);
        assert!(parse(&["g++", "-c", "a.cc"]).simple);
    }

    #[test]
    fn program_words_must_be_plain_trusted_names() {
        for argv in [
            &["./cargo", "test"][..],
            &["bin/cargo", "test"],
            &["/home/u/repo/.worktrees/x/cargo", "test"],
            &["./sh", "-c", "ls"],
            &["./bin/cargo test"],
            &["cargo test", "x"],
            &["bash", "-c", "./ls x"],
            &["bash", "-c", "'cargo test' x"],
            &["bash", "-c", "cargo\\ test x"],
            &["bash", "-c", "ls* x"],
            &["bash", "-c", "~/bin/ls"],
        ] {
            assert!(!parse(argv).simple, "{argv:?}");
        }
        // They're still seen by name for deny rules.
        assert!(has(&["./git", "push"], "git push"));
    }

    #[test]
    fn arguments_keep_their_boundaries() {
        let c = parse(&["git", "-c", "a=b c", "commit"]);
        assert_eq!(c.segments[0].words, ["git", "-c", "a=b c", "commit"]);
        let c = parse(&["git", "-C", "a b", "commit", "-m", "fix;"]);
        assert_eq!(
            c.segments[0].words,
            ["git", "-C", "a b", "commit", "-m", "fix;"]
        );
        assert!(
            !c.simple,
            "shell syntax in an argument is treated with suspicion"
        );
        for argv in [
            &["cargo\u{2003}test", "x"][..],
            &["bash", "-c", "ls\u{a0}x"],
            &["ls", "a\u{0}b"],
        ] {
            assert!(!parse(argv).simple, "{argv:?}");
        }
    }

    #[test]
    fn shell_syntax_in_argv_also_counts_as_a_line() {
        for argv in [
            &["cargo", "test", "&&", "curl", "x"][..],
            &["cargo", "test", ">", "out"],
            &["echo", "$(cat ~/.ssh/id_ed25519)"],
            &["cargo test && curl x"],
        ] {
            assert!(!parse(argv).simple, "{argv:?}");
        }
        assert!(has(&["cargo", "test", "&&", "git", "push"], "git push"));
        // But what it can't parse doesn't make the literal argv opaque.
        assert_eq!(parse(&["echo", "it's; fine"]).opaque, None);
        // A shell's script is parsed as a script, not re-joined with the argv.
        assert!(!has(&["bash", "-c", "echo 'a; git push'"], "git push"));
    }

    #[test]
    fn shells_are_unwrapped_to_their_script() {
        let c = parse(&["bash", "-c", "cargo test"]);
        assert!(c.simple);
        assert_eq!(c.line, "cargo test");
        let c = parse(&["/bin/sh", "-c", "cd sub && /usr/bin/git commit -m 'x y'"]);
        assert!(!c.simple);
        assert!(c.segment_lines().any(|l| l == "git commit -m x y"));
        for argv in [
            &["bash", "-e", "-c", "git push"][..],
            &["sh", "-c", "--", "git push"],
            &["bash", "-o", "pipefail", "-c", "git push"],
            &["tcsh", "-c", "git push"],
            &["busybox", "sh", "-c", "git push"],
            &["bash", "-c", "bash -c 'git push'"],
            &["fish", "--command", "git push"],
            &["fish", "-C", "git push", "x.fish"],
            &["pwsh", "-Command", "git push"],
            &["su", "-c", "git push"],
            &["script", "-q", "-c", "git push", "/dev/null"],
        ] {
            assert!(has(argv, "git push"), "{argv:?}: {:?}", segments(argv));
        }
        // A shell without -c runs a script file: an ordinary command.
        let c = parse(&["bash", "script.sh"]);
        assert!(c.simple);
        assert_eq!(c.line, "bash script.sh");
    }

    #[test]
    fn the_closed_grammar_is_parsed() {
        for script in [
            "{ git push; }",
            "if true; then git push; fi",
            "! git push",
            "git >/dev/null push",
            "X=$'\\'' git push",
            "echo $(git push)",
            "cat <<EOF\n$(git push)\nEOF",
            "for f in a; do git push; done",
        ] {
            assert!(has(&["bash", "-c", script], "git push"), "{script:?}");
        }
        // Heredoc bodies are data.
        let c = parse(&["bash", "-c", "cat <<'EOF' > x.md\ngit push\nEOF"]);
        assert!(!c.segment_lines().any(|l| l.starts_with("git")), "{c:?}");
        assert_eq!(c.opaque, None);
        // Redirections and pipes are understood, but not simple.
        let c = parse(&["bash", "-c", "cargo test 2>&1 | tail -20"]);
        assert_eq!(
            c.segment_lines().collect::<Vec<_>>(),
            [
                "bash -c cargo test 2>&1 | tail -20",
                "cargo test",
                "tail -20"
            ]
        );
        assert!(!c.simple);
        assert!(!parse(&["bash", "-c", "! cargo test"]).simple);
    }

    #[test]
    fn nested_and_rc_shells_are_never_simple() {
        assert!(!parse(&["sh", "-c", "bash -c 'cargo test'"]).simple);
        for shell in ["zsh", "fish", "csh", "tcsh"] {
            assert!(!parse(&[shell, "-c", "cargo test"]).simple, "{shell}");
        }
        assert!(!parse(&["bash", "-ic", "ls"]).simple);
        assert!(!parse(&["bash", "-lc", "ls"]).simple);
        assert!(!parse(&["bash", "--login", "-c", "ls"]).simple);
        assert!(parse(&["bash", "-ec", "ls"]).simple);
        assert!(parse(&["pwsh", "-NoProfile", "-c", "ls"]).simple);
        assert!(!parse(&["pwsh", "-c", "ls"]).simple);
    }

    #[test]
    fn expansions_in_command_position_are_opaque() {
        for argv in [
            &["bash", "-c", "$GIT push"][..],
            &["bash", "-c", "${X:-git push}"],
            &["bash", "-c", "eval \"$CMD\""],
            &["bash", "-c", "$(cat cmd)"],
            &["bash", "-c", "`cat cmd` x"],
            &["bash", "-c", "alias g=git; g push"],
            &["bash", "-c", "echo 'unclosed"],
            &["bash", "-c", "$'\\x67it' push"],
            &["bash", "-c", "\"$SHELL\""],
            &["bash", "-c", "env $X git push"],
            &["bash", "-c", "bash -c \"$CMD\""],
            &["pwsh", "-EncodedCommand", "ZQBjAGgAbwA="],
            &["sh", "-c", "sh -c \"sh -c 'sh -c \\\"sh -c x\\\"'\""],
        ] {
            let c = parse(argv);
            assert!(c.opaque.is_some(), "{argv:?}: {c:?}");
            assert!(!c.simple, "{argv:?}");
        }
        // A literal eval is parsed like a shell's script.
        let c = parse(&["bash", "-c", "eval git push"]);
        assert_eq!(c.opaque, None);
        assert!(c.segment_lines().any(|l| l == "git push"));
    }

    #[test]
    fn expansions_in_arguments_are_never_simple_but_keep_their_literal_prefix() {
        for script in [
            "cargo test $FLAGS",
            "rg ${X:---pre=evil} x",
            "rg {--pre=evil,x}",
            "rg --pr[e]=evil x",
            "ls *.rs",
            "cat ~/x",
        ] {
            let c = parse(&["bash", "-c", script]);
            assert!(!c.simple, "{script}: an expansion can pick an option");
            assert_eq!(c.opaque, None, "{script}");
        }
        let c = parse(&["bash", "-c", "cargo test $FLAGS"]);
        let seg = c.segments.iter().find(|s| s.words[0] == "cargo").unwrap();
        assert_eq!(seg.literal, 2);
        assert_eq!(seg.literal_prefix(), "cargo test ");
        assert_eq!(seg.first_expansion(), Some("$FLAGS"));
        // The prefix runs into the expanded word.
        let c = parse(&["bash", "-c", "rm -rf target/*"]);
        let seg = c.segments.iter().find(|s| s.words[0] == "rm").unwrap();
        assert_eq!(seg.literal_prefix(), "rm -rf target/");
        let c = parse(&["bash", "-c", "source .venv/bin/activate && pytest"]);
        assert!(!c.simple);
        assert_eq!(c.opaque, None);
    }

    #[test]
    fn shells_reading_data_are_opaque() {
        for argv in [
            &["bash", "-c", "echo git push | sh"][..],
            &["bash", "-c", "echo 'git push' | bash"],
            &["bash", "-c", "bash <<< 'git push'"],
            &["bash", "-c", "bash -s < x"],
            &["bash", "-c", "source <(echo git push)"],
            &["bash", "-c", ". <(echo git push)"],
            &["bash", "-c", "bash <(echo git push)"],
            &["bash", "-c", "bash \"$SCRIPT\""],
            &["bash", "-c", "hash -p /usr/bin/git x; x push"],
            &["sh"],
            &["su"],
        ] {
            assert!(parse(argv).opaque.is_some(), "{argv:?}");
        }
        for argv in [
            &["bash", "script.sh"][..],
            &["bash", "-c", "source .venv/bin/activate && pytest"],
            &["bash", "-c", ". ./env.sh"],
        ] {
            assert_eq!(parse(argv).opaque, None, "{argv:?}");
        }
    }

    #[test]
    fn xargs_supplies_arguments_policy_cannot_see() {
        let c = parse(&["bash", "-c", "echo push | xargs git"]);
        let seg = c.segments.iter().find(|s| s.words[0] == "git").unwrap();
        assert_eq!(seg.literal, 1, "{seg:?}");
        let c = parse(&["bash", "-c", "printf push | xargs -I{} git {}"]);
        let seg = c.segments.iter().find(|s| s.words[0] == "git").unwrap();
        assert_eq!(seg.literal, 1, "{seg:?}");
        let c = parse(&["xargs", "-I", "%", "git", "%"]);
        let seg = c.segments.iter().find(|s| s.words[0] == "git").unwrap();
        assert_eq!(seg.literal, 1, "{seg:?}");
        let c = parse(&["find", ".", "-exec", "git", "{}", ";"]);
        let seg = c.segments.iter().find(|s| s.words[0] == "git").unwrap();
        assert_eq!(seg.literal, 1, "{seg:?}");
    }

    #[test]
    fn builtins_that_run_code_are_parsed() {
        for argv in [
            &["bash", "-c", "trap 'git push' EXIT"][..],
            &["bash", "-c", "trap -- 'git push' EXIT"],
            &["bash", "-c", "complete -C 'git push' x"],
            &["bash", "-c", "mapfile -C 'git push' arr"],
            &["bash", "-c", "bind -x '\"\\C-x\": git push'"],
        ] {
            let c = parse(argv);
            assert!(
                c.segment_lines().any(|l| l.contains("git push")) || c.opaque.is_some(),
                "{argv:?}: {c:?}"
            );
        }
        assert!(has(&["bash", "-c", "trap 'git push' EXIT"], "git push"));
        assert!(
            parse(&["bash", "-c", "trap \"$CMD\" EXIT"])
                .opaque
                .is_some()
        );
        assert!(
            parse(&["bash", "-c", "enable -f ./x.so x"])
                .opaque
                .is_some()
        );
    }

    #[test]
    fn wrappers_and_assignments_are_stripped() {
        for argv in [
            &["env", "git", "push"][..],
            &["env", "-i", "FOO=1", "git", "push"],
            &["env", "-u", "HOME", "git", "push"],
            &["env", "-S", "git push"],
            &["env", "--split-string=git push"],
            &["timeout", "5", "git", "push"],
            &["timeout", "-s", "KILL", "5", "git", "push"],
            &["xargs", "-n", "1", "git", "push"],
            &["nice", "-n", "10", "nohup", "git", "push"],
            &["sudo", "-u", "root", "git", "push"],
            &["strace", "-f", "-o", "log", "git", "push"],
            &["chroot", "/", "git", "push"],
            &["parallel", "-j", "2", "git", "push"],
            &["find", ".", "-exec", "git", "push", ";"],
            &["watch", "git push"],
            &["watch", "-x", "git", "push"],
            &["bash", "-c", "GIT_DIR=x git push"],
            &["bash", "-c", "exec git push"],
            &["sh", "-c", "env sh -c 'git push'"],
        ] {
            // xargs and parallel add the arguments they read from stdin.
            let found = segments(argv).iter().any(|l| l.starts_with("git push"));
            assert!(found, "{argv:?}: {:?}", segments(argv));
            assert!(!parse(argv).simple, "{argv:?}");
        }
    }

    #[test]
    fn interpreter_code_and_unsafe_options_are_never_simple() {
        for argv in [
            &["python3", "-c", "print(1)"][..],
            &["python3.12", "-c", "import os"],
            &["python3", "-Bc", "import os"],
            &["node", "-e", "1"],
            &["node", "--eval", "1"],
            &["node", "--eval=1"],
            &["node", "-p", "1"],
            &["php", "-r", "1;"],
            &["deno", "eval", "1"],
            &["awk", "BEGIN{system(\"id\")}"],
            &["rg", "--pre", "./x", "pat"],
            &["rg", "--pre=./x", "pat"],
            &["find", ".", "-name", "x", "-delete"],
            &["git", "-c", "core.pager=sh", "log"],
            &["git", "grep", "-O", "x"],
            &["tar", "-I", "./x", "-cf", "a.tar", "."],
            &["sed", "-n", "1p", "f"],
            &["less", "README.md"],
            &["bash", "-c", "python3 -c 'import os'"],
        ] {
            assert!(!parse(argv).simple, "{argv:?}");
        }
        for argv in [
            &["python3", "manage.py", "test"][..],
            &["node", "script.js"],
            &["rg", "-n", "fn main"],
            &["sed", "--sandbox", "-n", "1p", "f"],
            &["git", "log", "-C"],
            &["tar", "-tf", "a.tar"],
        ] {
            assert!(parse(argv).simple, "{argv:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn program_names_ignore_case_on_macos() {
        assert_eq!(segments(&["GIT", "push"]), ["git push"]);
    }

    #[test]
    fn globs_match_whole_lines() {
        let g = ExecGlob::new("cargo test*");
        assert!(g.matches("cargo test"));
        assert!(g.matches("cargo test --all"));
        assert!(!g.matches("xcargo test"));
        assert!(!g.matches("cargo tes"));
        let g = ExecGlob::new("git * --force*");
        assert!(g.matches("git push --force"));
        assert!(g.matches("git push origin --force-with-lease"));
        assert!(!g.matches("git push"));
        assert!(ExecGlob::new("*").matches(""));
        assert!(ExecGlob::new("ls").matches("ls"));
        assert!(!ExecGlob::new("ls").matches("ls -la"));
    }

    #[test]
    fn prefixes_could_match_what_an_expansion_completes() {
        let g = ExecGlob::new("rm -rf /*");
        assert!(g.could_match_prefix("rm "));
        assert!(g.could_match_prefix("rm -rf "));
        assert!(!g.could_match_prefix("ls "));
        assert!(!g.could_match_prefix("rm -i "));
        let exact = ExecGlob::new("ls -la");
        assert!(exact.could_match_prefix("ls "));
        assert!(!exact.could_match_prefix("ls -la x "));
        assert!(ExecGlob::new("*").could_match_prefix("anything "));
    }

    #[test]
    fn prefixes_can_match_whatever_follows() {
        let g = ExecGlob::new("git push --force*");
        assert!(g.matches_prefix_certainly("git push --force "));
        assert!(g.matches_prefix_certainly("git push --force-with-lease="));
        assert!(!g.matches_prefix_certainly("git push "));
        assert!(ExecGlob::new("git * --force*").matches_prefix_certainly("git push --force "));
        assert!(!ExecGlob::new("git push").matches_prefix_certainly("git push "));
        assert!(!ExecGlob::new("git *x").matches_prefix_certainly("git push "));
    }

    #[test]
    fn escaped_stars_are_literal() {
        let exact = ExecGlob::new(&escape(r"rm a*b \x"));
        assert!(exact.matches(r"rm a*b \x"));
        assert!(!exact.matches(r"rm aXb \x"));
    }
}
