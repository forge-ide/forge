//! A small tokenizer for the POSIX-shaped shell scripts agents run (`bash -c '...'`).
//!
//! It knows the shell's closed grammar and nothing else: operators and groups, reserved words,
//! arithmetic commands, redirections (dropped with their targets), heredocs (their bodies are
//! data), quoting including `$'...'`, comments, and expansions. Nothing is expanded or run.
//!
//! Two rules keep it honest:
//!
//! - A word is literal only where the shell passes exactly its text. Everything a shell
//!   expands ends the literal part: parameters (`$x`, `${...}`), command substitutions,
//!   arithmetic, `$'\x..'` escapes by code, and unquoted brace expansion, glob characters, and
//!   a leading `~`.
//! - No text is skipped unseen. Every command substitution, backtick, and process
//!   substitution, wherever it appears (inside `${...}`, arithmetic, double quotes, or an
//!   unquoted heredoc body), is returned as a nested script that also runs.

/// One word of a command, with quoting removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Word {
    /// The word's text. Expansions appear as written (`$X`, `$(cmd)`, `*.rs`).
    pub text: String,
    /// How many bytes of `text` come before the first expansion: all of it for a literal word.
    pub literal_len: usize,
}

impl Word {
    /// A word with no expansion in it.
    pub fn plain(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            literal_len: text.len(),
            text,
        }
    }

    /// Whether the shell passes exactly `text`.
    pub fn is_literal(&self) -> bool {
        self.literal_len >= self.text.len()
    }

    /// The part before the first expansion.
    pub fn literal_part(&self) -> &str {
        &self.text[..self.literal_len.min(self.text.len())]
    }
}

/// One simple command.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Command {
    pub words: Vec<Word>,
    /// Whether it redirects input or output.
    pub redirected: bool,
}

/// A parsed script.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Script {
    pub commands: Vec<Command>,
    /// Scripts found inside command and process substitutions: they also run.
    pub nested: Vec<String>,
    /// Whether the script is more than one plain command: operators, groups, reserved words,
    /// arithmetic commands, or substitutions.
    pub compound: bool,
    /// Why the script can't be fully understood (an unclosed quote, a heredoc without its
    /// delimiter), if it can't. The shell would reject such a script anyway.
    pub problem: Option<&'static str>,
}

/// Words that are keywords in command position. `time` is left to the caller, as a wrapper.
const RESERVED: &[&str] = &[
    "if", "then", "elif", "else", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "{", "}", "!", "coproc", "select", "function", "[[", "]]",
];

/// Parse `text` as a shell script.
pub fn parse(text: &str) -> Script {
    let mut p = Parser {
        chars: text.chars().collect(),
        i: 0,
        script: Script::default(),
        command: Command::default(),
        word: WordBuf::default(),
        next: Next::Normal,
        skip_rest: false,
        skip_words: 0,
        cases: 0,
        heredocs: Vec::new(),
    };
    p.run();
    let mut script = p.script;
    script.compound |= !script.nested.is_empty();
    script
}

/// The scripts that command substitutions, backticks, and process substitutions in `text`
/// run, wherever they are. With `quotes`, single-quoted text is skipped as the shell would;
/// without (a heredoc body), quote characters are ordinary.
pub fn substitutions(text: &str, quotes: bool) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    let mut i = 0;
    let mut double = false;
    while let Some(&c) = chars.get(i) {
        match c {
            '\\' => i += 2,
            '\'' if quotes && !double => {
                i += 1;
                while chars.get(i).is_some_and(|&c| c != '\'') {
                    i += 1;
                }
                i += 1;
            }
            '"' if quotes => {
                double = !double;
                i += 1;
            }
            '$' if chars.get(i + 1) == Some(&'(') => {
                let arithmetic = chars.get(i + 2) == Some(&'(');
                let (body, end, _) = balanced(&chars, i + 1, '(', ')');
                if arithmetic {
                    found.extend(substitutions(&body, quotes));
                } else {
                    found.push(body);
                }
                i = end;
            }
            '$' if chars.get(i + 1) == Some(&'{') => {
                let (body, end, _) = balanced(&chars, i + 1, '{', '}');
                found.extend(substitutions(&body, quotes));
                i = end;
            }
            '<' | '>' if !double && chars.get(i + 1) == Some(&'(') => {
                let (body, end, _) = balanced(&chars, i + 1, '(', ')');
                found.push(body);
                i = end;
            }
            '`' => {
                let (body, end, _) = backtick(&chars, i + 1);
                found.push(body);
                i = end;
            }
            _ => i += 1,
        }
    }
    found
}

/// The text from `start` (on `open`) to the matching `close`, skipping quoted text; the index
/// after it; and whether it was closed.
fn balanced(chars: &[char], start: usize, open: char, close: char) -> (String, usize, bool) {
    debug_assert_eq!(chars.get(start), Some(&open));
    let mut i = start + 1;
    let mut depth = 1;
    let mut body = String::new();
    let mut quote: Option<char> = None;
    while let Some(&c) = chars.get(i) {
        i += 1;
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (_, '\\') => {
                body.push(c);
                if let Some(&n) = chars.get(i) {
                    body.push(n);
                    i += 1;
                }
                continue;
            }
            (Some('"'), '"') => quote = None,
            (Some('"'), _) => {}
            (None, '\'' | '"') => quote = Some(c),
            (None, c) if c == open => depth += 1,
            (None, c) if c == close => {
                depth -= 1;
                if depth == 0 {
                    return (body, i, true);
                }
            }
            _ => {}
        }
        body.push(c);
    }
    (body, i, false)
}

/// The body of a backtick substitution starting after the opening backtick; the index after
/// the closing one; and whether it was closed.
fn backtick(chars: &[char], start: usize) -> (String, usize, bool) {
    let mut i = start;
    let mut body = String::new();
    while let Some(&c) = chars.get(i) {
        i += 1;
        match c {
            '`' => return (body, i, true),
            '\\' if matches!(chars.get(i), Some('`' | '\\' | '$')) => {
                body.push(chars[i]);
                i += 1;
            }
            c => body.push(c),
        }
    }
    (body, i, false)
}

/// What the word being read is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Normal,
    /// A redirection's target: dropped.
    Target,
    /// A heredoc's delimiter; `true` for `<<-`, which strips leading tabs.
    Delimiter(bool),
}

struct Heredoc {
    delimiter: String,
    quoted: bool,
    strip_tabs: bool,
}

/// The word being read.
#[derive(Default)]
struct WordBuf {
    text: String,
    started: bool,
    /// Whether any of it was quoted or escaped.
    quoted: bool,
    /// Where the first parameter, command, or arithmetic expansion starts.
    expansion: Option<usize>,
    /// Unquoted characters that brace expansion, globbing, or tilde expansion act on, with
    /// their byte offsets.
    marks: Vec<(usize, char)>,
}

impl WordBuf {
    fn push(&mut self, c: char) {
        self.text.push(c);
        self.started = true;
    }

    /// An unquoted character, which may take part in brace, glob, or tilde expansion.
    fn push_unquoted(&mut self, c: char) {
        if "*?[]{},.~=:".contains(c) {
            self.marks.push((self.text.len(), c));
        }
        self.push(c);
    }

    fn expansion_here(&mut self) {
        self.expansion.get_or_insert(self.text.len());
        self.started = true;
    }

    /// The offset of the first expansion of any kind, if any.
    fn first_expansion(&self) -> Option<usize> {
        let marks = &self.marks;
        let mut first = self.expansion;
        let mut note = |at: usize| first = Some(first.map_or(at, |f| f.min(at)));
        for (k, &(at, c)) in marks.iter().enumerate() {
            let later = &marks[k + 1..];
            match c {
                // A leading `~`, or one after `=` or `:` (bash expands those in words shaped
                // like assignments, `X=~/y`, `--opt=~/y`, `PATH=a:~/b`).
                '~' if at == 0
                    || marks[..k]
                        .last()
                        .is_some_and(|&(p, c)| p + 1 == at && (c == '=' || c == ':')) =>
                {
                    note(at)
                }
                '*' | '?' => note(at),
                // A lone `[` is the test command, not a glob.
                '[' if later.iter().any(|&(_, c)| c == ']') => note(at),
                '{' => {
                    // Brace expansion needs a `,` or `..` before the matching `}`; `{}` alone
                    // stays literal.
                    let close = later.iter().position(|&(_, c)| c == '}');
                    if let Some(close) = close {
                        let inside = &later[..close];
                        let comma = inside.iter().any(|&(_, c)| c == ',');
                        let range = inside
                            .windows(2)
                            .any(|w| w[0].1 == '.' && w[1].1 == '.' && w[1].0 == w[0].0 + 1);
                        if comma || range {
                            note(at);
                        }
                    }
                }
                _ => {}
            }
        }
        first
    }

    fn take(&mut self) -> Word {
        let literal_len = self.first_expansion().unwrap_or(self.text.len());
        let word = Word {
            text: std::mem::take(&mut self.text),
            literal_len,
        };
        *self = WordBuf::default();
        word
    }
}

struct Parser {
    chars: Vec<char>,
    i: usize,
    script: Script,
    command: Command,
    word: WordBuf,
    next: Next,
    /// Inside `for x in ...`, `select`, or a `case` pattern: the rest isn't a command.
    skip_rest: bool,
    /// Words to skip: the name after `function`.
    skip_words: usize,
    /// How many `case` statements are open; after `;;` in one, a pattern comes next.
    cases: usize,
    heredocs: Vec<Heredoc>,
}

impl Parser {
    fn peek(&self, ahead: usize) -> Option<char> {
        self.chars.get(self.i + ahead).copied()
    }

    fn run(&mut self) {
        while let Some(c) = self.peek(0) {
            match c {
                ' ' | '\t' => {
                    self.end_word();
                    self.i += 1;
                }
                '\n' => {
                    self.end_word();
                    self.i += 1;
                    self.end_command();
                    self.read_heredoc_bodies();
                }
                '#' if !self.word.started => {
                    while self.peek(0).is_some_and(|c| c != '\n') {
                        self.i += 1;
                    }
                }
                '(' if !self.word.started
                    && self.command.words.is_empty()
                    && self.peek(1) == Some('(') =>
                {
                    // An arithmetic command, `(( ... ))`, or a C-style `for (( ...; ...; ... ))`.
                    let (body, end, closed) = balanced(&self.chars, self.i, '(', ')');
                    self.i = end;
                    self.script.nested.extend(substitutions(&body, true));
                    self.script.compound = true;
                    if !closed {
                        self.problem("an unclosed ((");
                    }
                }
                ';' | '&' | '|' | '(' | ')' => {
                    if c == '&' && self.peek(1) == Some('>') {
                        // `&>` and `&>>`: redirect both streams.
                        self.end_word();
                        self.i += if self.peek(2) == Some('>') { 3 } else { 2 };
                        self.redirect(Next::Target);
                        continue;
                    }
                    self.end_word();
                    let two = matches!(
                        (c, self.peek(1)),
                        ('&', Some('&')) | ('|', Some('|' | '&')) | (';', Some(';' | '&'))
                    );
                    self.i += if two { 2 } else { 1 };
                    if c == ';' && two && self.peek(0) == Some('&') {
                        self.i += 1;
                    }
                    self.end_command();
                    self.script.compound = true;
                    if c == ';' && two && self.cases > 0 {
                        // The next case's pattern, up to its `)`.
                        self.skip_rest = true;
                    }
                }
                '<' | '>' => {
                    if self.peek(1) == Some('(') {
                        // Process substitution: `<(cmd)`, `>(cmd)`.
                        self.i += 1;
                        self.substitution();
                        continue;
                    }
                    // A word of digits right before is the file descriptor: `2>`.
                    let w = &self.word;
                    if w.started
                        && !w.quoted
                        && w.expansion.is_none()
                        && w.text.chars().all(|d| d.is_ascii_digit())
                    {
                        self.word = WordBuf::default();
                    } else {
                        self.end_word();
                    }
                    let rest: String = self.chars[self.i..].iter().take(3).collect();
                    let (len, next) = if rest.starts_with("<<<") {
                        (3, Next::Target)
                    } else if rest.starts_with("<<-") {
                        (3, Next::Delimiter(true))
                    } else if rest.starts_with("<<") {
                        (2, Next::Delimiter(false))
                    } else if [">>", ">|", ">&", "<&", "<>"]
                        .iter()
                        .any(|op| rest.starts_with(op))
                    {
                        (2, Next::Target)
                    } else {
                        (1, Next::Target)
                    };
                    self.i += len;
                    self.redirect(next);
                }
                '\\' => {
                    self.i += 1;
                    match self.peek(0) {
                        // A line continuation.
                        Some('\n') => self.i += 1,
                        Some(next) => {
                            self.word.push(next);
                            self.word.quoted = true;
                            self.i += 1;
                        }
                        None => self.word.push('\\'),
                    }
                }
                '\'' => {
                    self.word.quoted = true;
                    self.word.started = true;
                    self.i += 1;
                    match self.chars[self.i..].iter().position(|&c| c == '\'') {
                        Some(end) => {
                            let text: String = self.chars[self.i..self.i + end].iter().collect();
                            self.word.text.push_str(&text);
                            self.i += end + 1;
                        }
                        None => {
                            let text: String = self.chars[self.i..].iter().collect();
                            self.word.text.push_str(&text);
                            self.i = self.chars.len();
                            self.problem("an unclosed quote");
                        }
                    }
                }
                '"' => {
                    self.word.quoted = true;
                    self.word.started = true;
                    self.i += 1;
                    self.double_quoted();
                }
                '$' => match self.peek(1) {
                    Some('\'') => {
                        self.word.quoted = true;
                        self.i += 2;
                        self.ansi_c();
                    }
                    Some('"') => {
                        self.word.quoted = true;
                        self.word.started = true;
                        self.i += 2;
                        self.double_quoted();
                    }
                    _ => self.dollar(),
                },
                '`' => self.backtick(),
                c => {
                    self.word.push_unquoted(c);
                    self.i += 1;
                }
            }
        }
        self.end_word();
        self.end_command();
        if !self.heredocs.is_empty() {
            self.problem("a heredoc without its closing delimiter");
        }
    }

    /// Whether the next word is followed by a `{` or `(` group: `coproc NAME { ... }`.
    fn name_before_group(&self) -> bool {
        let rest = &self.chars[self.i..];
        let start = rest.iter().position(|c| !matches!(c, ' ' | '\t'));
        let Some(start) = start else { return false };
        let end = rest[start..]
            .iter()
            .position(|c| matches!(c, ' ' | '\t' | '\n' | ';' | '&' | '|' | '(' | ')' | '{'))
            .map_or(rest.len(), |p| start + p);
        let next = rest[end..].iter().find(|c| !matches!(c, ' ' | '\t'));
        end > start && matches!(next, Some('{' | '('))
    }

    fn problem(&mut self, what: &'static str) {
        self.script.problem.get_or_insert(what);
    }

    fn redirect(&mut self, next: Next) {
        self.command.redirected = true;
        self.next = next;
    }

    /// Inside `"..."`, after the opening quote.
    fn double_quoted(&mut self) {
        while let Some(c) = self.peek(0) {
            match c {
                '"' => {
                    self.i += 1;
                    return;
                }
                '\\' => {
                    self.i += 1;
                    match self.peek(0) {
                        Some(n @ ('$' | '`' | '"' | '\\')) => {
                            self.word.text.push(n);
                            self.i += 1;
                        }
                        Some('\n') => self.i += 1,
                        _ => self.word.text.push('\\'),
                    }
                }
                '$' => self.dollar(),
                '`' => self.backtick(),
                c => {
                    self.word.text.push(c);
                    self.i += 1;
                }
            }
        }
        self.problem("an unclosed quote");
    }

    /// Inside `$'...'`, after the opening quote. Escapes that name characters by code end the
    /// literal part, since they could spell anything.
    fn ansi_c(&mut self) {
        self.word.started = true;
        while let Some(c) = self.peek(0) {
            self.i += 1;
            match c {
                '\'' => return,
                '\\' => {
                    let Some(e) = self.peek(0) else { break };
                    self.i += 1;
                    let decoded = match e {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        'a' => '\u{7}',
                        'b' => '\u{8}',
                        'e' | 'E' => '\u{1b}',
                        'f' => '\u{c}',
                        'v' => '\u{b}',
                        '\\' | '\'' | '"' | '?' => e,
                        _ => {
                            self.word.expansion_here();
                            self.word.text.push('\\');
                            self.word.text.push(e);
                            continue;
                        }
                    };
                    self.word.text.push(decoded);
                }
                c => self.word.text.push(c),
            }
        }
        self.problem("an unclosed quote");
    }

    /// At a `$` outside single quotes.
    fn dollar(&mut self) {
        match self.peek(1) {
            Some('(') if self.peek(2) == Some('(') => {
                // Arithmetic: its own value is opaque, but substitutions inside it run.
                let (body, end, closed) = balanced(&self.chars, self.i + 1, '(', ')');
                self.i = end;
                self.script.nested.extend(substitutions(&body, true));
                self.opaque_text(&format!("$({body})"));
                if !closed {
                    self.problem("an unclosed $((");
                }
            }
            Some('(') => {
                self.i += 1;
                self.substitution();
            }
            Some('{') => {
                let (body, end, closed) = balanced(&self.chars, self.i + 1, '{', '}');
                self.i = end;
                self.script.nested.extend(substitutions(&body, true));
                self.opaque_text(&format!("${{{body}}}"));
                if !closed {
                    self.problem("an unclosed ${");
                }
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => {
                let name: String = self.chars[self.i + 1..]
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == '_')
                    .collect();
                self.i += 1 + name.chars().count();
                self.opaque_text(&format!("${name}"));
            }
            Some(c) if c.is_ascii_digit() || "@*#?$!-".contains(c) => {
                self.i += 2;
                self.opaque_text(&format!("${c}"));
            }
            _ => {
                self.word.push('$');
                self.i += 1;
            }
        }
    }

    /// On the `(` of `$(`, `<(`, or `>(`: a script that runs.
    fn substitution(&mut self) {
        let (body, end, closed) = balanced(&self.chars, self.i, '(', ')');
        self.i = end;
        self.opaque_text(&format!("$({body})"));
        self.script.nested.push(body);
        if !closed {
            self.problem("an unclosed command substitution");
        }
    }

    fn backtick(&mut self) {
        let (body, end, closed) = backtick(&self.chars, self.i + 1);
        self.i = end;
        self.opaque_text(&format!("`{body}`"));
        self.script.nested.push(body);
        if !closed {
            self.problem("an unclosed backtick");
        }
    }

    fn opaque_text(&mut self, text: &str) {
        self.word.expansion_here();
        self.word.text.push_str(text);
    }

    fn end_word(&mut self) {
        if !self.word.started {
            return;
        }
        let quoted = self.word.quoted;
        let word = self.word.take();
        match std::mem::replace(&mut self.next, Next::Normal) {
            Next::Target => return,
            Next::Delimiter(strip_tabs) => {
                self.heredocs.push(Heredoc {
                    delimiter: word.text,
                    quoted,
                    strip_tabs,
                });
                return;
            }
            Next::Normal => {}
        }
        let keyword = word.is_literal() && !quoted;
        if self.skip_rest {
            // `;; esac` closes the case where a pattern would otherwise be.
            if self.cases > 0 && keyword && word.text == "esac" {
                self.cases -= 1;
                self.skip_rest = false;
            }
            return;
        }
        if self.skip_words > 0 {
            self.skip_words -= 1;
            return;
        }
        if self.command.words.is_empty() && keyword && RESERVED.contains(&word.text.as_str()) {
            self.script.compound = true;
            match word.text.as_str() {
                "case" => {
                    self.cases += 1;
                    self.skip_rest = true;
                }
                "esac" => self.cases = self.cases.saturating_sub(1),
                "for" | "select" => self.skip_rest = true,
                "function" => self.skip_words = 1,
                // `coproc NAME { ...; }` names the coprocess before its compound command.
                "coproc" if self.name_before_group() => self.skip_words = 1,
                _ => {}
            }
            return;
        }
        self.command.words.push(word);
    }

    fn end_command(&mut self) {
        let command = std::mem::take(&mut self.command);
        if !command.words.is_empty() {
            self.script.commands.push(command);
        } else if command.redirected {
            self.script.compound = true;
        }
        self.skip_rest = false;
        self.skip_words = 0;
        self.next = Next::Normal;
    }

    /// After a newline: the bodies of heredocs started on that line.
    fn read_heredoc_bodies(&mut self) {
        for doc in std::mem::take(&mut self.heredocs) {
            let mut closed = false;
            while self.i < self.chars.len() {
                let end = self.chars[self.i..]
                    .iter()
                    .position(|&c| c == '\n')
                    .map_or(self.chars.len(), |p| self.i + p);
                let line: String = self.chars[self.i..end].iter().collect();
                self.i = (end + 1).min(self.chars.len());
                let compared = if doc.strip_tabs {
                    line.trim_start_matches('\t')
                } else {
                    line.as_str()
                };
                if compared == doc.delimiter {
                    closed = true;
                    break;
                }
                if !doc.quoted {
                    // An unquoted body expands substitutions, which run; quotes in it are
                    // ordinary characters.
                    self.script.nested.extend(substitutions(&line, false));
                }
            }
            self.script.compound = true;
            if !closed {
                self.problem("a heredoc without its closing delimiter");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each command's words, with the expanded part of a word marked by `~`.
    fn commands(text: &str) -> Vec<String> {
        parse(text)
            .commands
            .iter()
            .map(|c| {
                c.words
                    .iter()
                    .map(|w| {
                        if w.is_literal() {
                            w.text.clone()
                        } else {
                            format!("{}~{}", w.literal_part(), &w.text[w.literal_len..])
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect()
    }

    #[test]
    fn splits_on_operators() {
        assert_eq!(
            commands("a && b || c; d | e |& f & g"),
            ["a", "b", "c", "d", "e", "f", "g"]
        );
        assert_eq!(commands("a\nb"), ["a", "b"]);
        assert_eq!(commands("(a; b)"), ["a", "b"]);
        let s = parse("cargo test");
        assert!(!s.compound);
        assert_eq!(s.commands.len(), 1);
        assert!(parse("a; ").compound);
    }

    #[test]
    fn carriage_returns_are_ordinary_characters() {
        // As in bash: `\r` is a word, not a line break.
        assert_eq!(commands("a \r; b"), ["a \r", "b"]);
        let s = parse("cat <<EOF \r; git push\nEOF\n");
        assert!(s.commands.iter().any(|c| c.words[0].text == "git"), "{s:?}");
    }

    #[test]
    fn removes_quoting() {
        assert_eq!(
            commands(r#"git commit -m 'a b' "c \" d" e\ f '' g''it"#),
            [r#"git commit -m a b c " d e f  git"#]
        );
        let s = parse(r#"git commit -m 'a b' "#);
        assert_eq!(s.commands[0].words[3], Word::plain("a b"));
        assert_eq!(s.commands[0].words.len(), 4);
        assert_eq!(commands("echo a\\\nb"), ["echo ab"]);
    }

    #[test]
    fn decodes_ansi_c_quotes() {
        assert_eq!(commands(r"X=$'\'' git push"), ["X=' git push"]);
        assert_eq!(commands(r"echo $'a\tb'"), ["echo a\tb"]);
        // Escapes by code could spell anything.
        assert_eq!(commands(r"$'\x67it' push"), [r"~\x67it push"]);
    }

    #[test]
    fn skips_reserved_words_and_groups() {
        assert_eq!(commands("{ git push; }"), ["git push"]);
        assert_eq!(commands("if true; then git push; fi"), ["true", "git push"]);
        assert_eq!(commands("! git push"), ["git push"]);
        assert_eq!(
            commands("for f in a b; do rustfmt $f; done"),
            ["rustfmt ~$f"]
        );
        assert_eq!(
            commands("case $x in a) git push;; b) ls;; esac"),
            ["git push", "ls"]
        );
        assert_eq!(
            commands("case $x in a) ls;; esac; git push;; echo x"),
            ["ls", "git push", "echo x"]
        );
        assert_eq!(commands("function f { git push; }; f"), ["git push", "f"]);
        assert_eq!(commands("coproc NAME { git push; }"), ["git push"]);
        assert_eq!(commands("coproc { git push; }"), ["git push"]);
        assert_eq!(commands("coproc git push"), ["git push"]);
        assert_eq!(commands("f() { git push; }; f"), ["f", "git push", "f"]);
        // Only in command position, and only unquoted.
        assert_eq!(commands("echo if then {"), ["echo if then {"]);
        assert_eq!(commands("'if' x"), ["if x"]);
    }

    #[test]
    fn arithmetic_commands_are_not_heredocs() {
        assert_eq!(commands("((x<<2))\ngit push\n2"), ["git push", "2"]);
        assert_eq!(
            commands("for ((i=0; i<3; i++)); do git push; done"),
            ["git push"]
        );
        let s = parse("(( $(git push) ))");
        assert_eq!(s.nested, ["git push"]);
        assert!(s.compound);
        // A subshell inside a subshell is not arithmetic.
        assert_eq!(commands("( (git push) )"), ["git push"]);
    }

    #[test]
    fn drops_redirections_with_their_targets() {
        assert_eq!(commands("git >/dev/null push"), ["git push"]);
        assert_eq!(commands("git push 2>&1 --force"), ["git push --force"]);
        assert_eq!(commands("cargo test &> log"), ["cargo test"]);
        assert_eq!(commands("sort < in >> out"), ["sort"]);
        assert_eq!(commands("cat <<< 'text'"), ["cat"]);
        assert!(commands("> out").is_empty());
        assert!(parse("ls > out").commands[0].redirected);
        assert!(!parse("ls").commands[0].redirected);
        assert_eq!(commands("echo a>out"), ["echo a"]);
        // A quoted `2` is a word, not a descriptor.
        assert_eq!(commands("echo '2'>out"), ["echo 2"]);
    }

    #[test]
    fn heredoc_bodies_are_data_but_their_substitutions_run() {
        let s = parse("cat <<'EOF' > notes.md\ngit push\nEOF\necho done");
        assert_eq!(s.commands.len(), 2);
        assert!(s.nested.is_empty());
        assert_eq!(s.problem, None);
        let s = parse("cat <<EOF\n$(git push) `id` don't $(ls)\nEOF");
        assert_eq!(s.nested, ["git push", "id", "ls"]);
        let s = parse("cat <<-END\n\tbody\n\tEND\nls");
        assert_eq!(s.commands.len(), 2);
        assert_eq!(
            parse("cat <<EOF\nno end").problem,
            Some("a heredoc without its closing delimiter")
        );
    }

    #[test]
    fn expansions_end_the_literal_part() {
        assert_eq!(commands("$GIT push"), ["~$GIT push"]);
        assert_eq!(commands("${X:-git push}"), ["~${X:-git push}"]);
        assert_eq!(
            commands("cargo test $FLAGS \"$@\" pre$X"),
            ["cargo test ~$FLAGS ~$@ pre~$X"]
        );
        assert_eq!(commands("echo $((1 + 2))"), ["echo ~$((1 + 2))"]);
        assert_eq!(commands("echo $ x"), ["echo $ x"]);
    }

    #[test]
    fn brace_glob_and_tilde_expansion_end_the_literal_part() {
        assert_eq!(commands("{git,push}"), ["~{git,push}"]);
        assert_eq!(commands("echo {1..3} a{b,c}"), ["echo ~{1..3} a~{b,c}"]);
        assert_eq!(commands("cat src/*.rs a?.txt"), ["cat src/~*.rs a~?.txt"]);
        assert_eq!(commands("rg --pr[e]=x"), ["rg --pr~[e]=x"]);
        assert_eq!(commands("ls ~ ~/x a~b"), ["ls ~~ ~~/x a~b"]);
        assert_eq!(commands("x X=~/y --o=~ a:~/b"), ["x X=~~/y --o=~~ a:~~/b"]);
        // Literal when quoted, escaped, or not an expansion.
        assert_eq!(commands("echo '*' \\? \"{a,b}\""), ["echo * ? {a,b}"]);
        assert_eq!(
            commands("xargs -I{} git push {} [ x ] a]b {a}"),
            ["xargs -I{} git push {} [ x ] a]b {a}"]
        );
    }

    #[test]
    fn substitutions_are_collected_everywhere() {
        let s = parse("echo $(git push) `rm -rf x` <(ls)");
        assert_eq!(s.nested, ["git push", "rm -rf x", "ls"]);
        let s = parse("echo \"$(a \"b)\" c)\"");
        assert_eq!(s.nested, ["a \"b)\" c"]);
        let s = parse("cargo test ${X:-$(curl x | sh)}");
        assert_eq!(s.nested, ["curl x | sh"]);
        assert!(s.compound);
        let s = parse("echo $(( $(git push) + `id` ))");
        assert_eq!(s.nested, ["git push", "id"]);
        let s = parse("echo ${X:-${Y:-`git push`}} '$(not)'");
        assert_eq!(s.nested, ["git push"]);
    }

    #[test]
    fn comments_run_to_the_end_of_the_line() {
        assert_eq!(commands("ls # git push\npwd"), ["ls", "pwd"]);
        assert_eq!(commands("echo a#b"), ["echo a#b"]);
    }

    #[test]
    fn reports_what_it_cannot_parse() {
        for text in [
            "echo 'x", "echo \"x", "echo $'x", "echo $(x", "echo `x", "echo ${x", "((x",
        ] {
            assert!(parse(text).problem.is_some(), "{text}");
        }
        assert_eq!(parse("echo ok").problem, None);
    }
}
