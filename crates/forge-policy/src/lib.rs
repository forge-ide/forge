//! Rules, modes, and guards.
//!
//! Every action an agent wants to take becomes a [`Request`] and gets exactly one [`Decision`]:
//! allow, deny, or ask, with a reason and the [`Source`] that decided. [`Engine`] evaluates in
//! the order from component-architecture.md, first match winning and no later step loosening
//! an earlier one:
//!
//! 1. Hard denies: the built-in git rule, then `[[deny]]` rules from the managed, user, and
//!    project layers, then denials remembered for the session.
//! 2. Guards: edits outside the worktree or inside `.git`, protected paths, commands whose
//!    program Forge can't tell, then secrets in the content or command line. A deny rule or
//!    the git rule that an expansion could complete asks here too.
//! 3. Pre-tool hooks, which may also rewrite a command; a rewritten command goes through steps
//!    1 and 2 again. A hook's allow counts only from a hook the agent can't influence.
//! 4. `[[ask]]` rules from any layer, then allows remembered for the session and `[[allow]]`
//!    rules. Ask beats allow whichever layers they come from, so a lower layer never loosens a
//!    higher one, and a hook's allow doesn't override an ask rule either.
//! 5. The classifier, in Auto mode only.
//! 6. The mode's default ([`mode_default`]).

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use futures_util::future::BoxFuture;

pub use forge_api::events::Verdict;
pub use forge_api::model::{Action, EditKind, Mode};

pub mod exec;
pub mod git;
pub mod paths;
pub mod rules;
pub mod secrets;
pub mod shell;

pub use rules::Rules;
pub use secrets::SecretScanner;

/// An action to decide, with the text it would write, when there is some.
///
/// Edit paths are checked lexically: the engine normalizes `.` and `..` but never touches the
/// filesystem. Callers must resolve symlinks first, so that the path decided is the path
/// written: canonicalize the parent directory, check it is still inside the worktree, refuse a
/// final component that is a symlink, and pass the result relative to the worktree. Otherwise
/// a link the agent planted (`notes.txt -> .env`) would carry an allowed edit to a protected
/// file. The ACP workspace's write resolution does this.
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub action: Action,
    /// The proposed content of an edit, or other text the agent supplied with the action;
    /// scanned for secrets.
    pub content: Option<String>,
}

impl Request {
    pub fn new(action: Action) -> Self {
        Self {
            action,
            content: None,
        }
    }

    pub fn with_content(mut self, content: impl Into<String>) -> Self {
        self.content = Some(content.into());
        self
    }
}

/// A policy layer, from the most to the least authoritative.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Built into Forge; no file can remove it.
    Builtin,
    /// Org-managed policy.
    Managed,
    /// `~/.config/forge/policy.toml`.
    User,
    /// `<root>/.forge/policy.toml`.
    Project,
    /// Decisions remembered for one session.
    Session,
}

impl fmt::Display for Layer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Layer::Builtin => "builtin",
            Layer::Managed => "managed",
            Layer::User => "user",
            Layer::Project => "project",
            Layer::Session => "session",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleList {
    Deny,
    Ask,
    Allow,
}

impl fmt::Display for RuleList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            RuleList::Deny => "deny",
            RuleList::Ask => "ask",
            RuleList::Allow => "allow",
        })
    }
}

/// What produced a decision. Its `Display` form is the `source` in `policy.decision` events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A rule in a policy file: `project:deny[2]`.
    Rule {
        layer: Layer,
        list: RuleList,
        index: usize,
    },
    /// A decision remembered for the session: `session:allow`.
    Remembered(RuleList),
    /// The built-in rule against git commands that write shared state.
    GitSharedState,
    /// An edit outside the worktree or inside `.git`.
    OutsideWorktree,
    /// An edit to a protected path.
    ProtectedPath,
    /// A command whose program Forge can't tell: an expansion in command position, `eval`
    /// of a variable, an alias, broken quoting, shells nested too deep.
    OpaqueCommand,
    /// Content or a command line that looks like it holds a secret.
    SecretScan,
    /// A pre-tool hook, by name.
    Hook(String),
    Classifier,
    /// The mode's default.
    Mode(Mode),
    /// Network access, which the agent profile's allowlist governs.
    NetworkAllowlist,
}

impl Source {
    /// Whether remembering a human's answer could change later decisions: only when the ask
    /// came from the mode or the classifier, since remembered allows never beat ask rules,
    /// guards, or hooks.
    pub fn rememberable(&self) -> bool {
        matches!(self, Source::Mode(_) | Source::Classifier)
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Rule { layer, list, index } => write!(f, "{layer}:{list}[{index}]"),
            Source::Remembered(list) => write!(f, "session:{list}"),
            Source::GitSharedState => f.write_str("builtin:git-shared-state"),
            Source::OutsideWorktree => f.write_str("guard:outside-worktree"),
            Source::ProtectedPath => f.write_str("guard:protected-path"),
            Source::OpaqueCommand => f.write_str("guard:opaque-command"),
            Source::SecretScan => f.write_str("guard:secret-scan"),
            Source::Hook(name) => write!(f, "hook:{name}"),
            Source::Classifier => f.write_str("classifier"),
            Source::Mode(mode) => write!(f, "mode:{}", mode_name(*mode)),
            Source::NetworkAllowlist => f.write_str("network:allowlist"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub verdict: Verdict,
    pub reason: String,
    pub source: Source,
}

impl Decision {
    fn new(verdict: Verdict, reason: impl Into<String>, source: Source) -> Self {
        Self {
            verdict,
            reason: reason.into(),
            source,
        }
    }
}

/// A decision, the action it applies to (a hook may have rewritten the request's), and a note
/// for the agent from a hook.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub decision: Decision,
    pub action: Action,
    pub note: Option<String>,
    /// The hook that rewrote the action, if one did.
    pub rewritten_by: Option<String>,
}

impl Outcome {
    fn of(decision: Decision, action: Action) -> Self {
        Self {
            decision,
            action,
            note: None,
            rewritten_by: None,
        }
    }
}

/// A policy file that can't be used.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{layer} policy: {message}")]
pub struct PolicyError {
    pub layer: Layer,
    pub message: String,
}

/// What the pre-tool hooks say about a request, combined.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookAdvice {
    /// The hook that decided or rewrote, for the decision's source.
    pub hook: String,
    pub verdict: Option<Verdict>,
    pub reason: Option<String>,
    /// A replacement argv; only for `exec` actions.
    pub rewrite: Option<Vec<String>>,
    /// The hook that rewrote the action, when another one decided.
    pub rewritten_by: Option<String>,
    pub note: Option<String>,
    /// Whether the hooks ran where the agent can't influence them. Only then does an allow
    /// count: a hook in the agent's sandbox carries at most the agent's trust, so it may deny,
    /// ask, or rewrite, but its allow is no opinion.
    pub trusted: bool,
}

/// Runs the session's pre-tool hooks (`tool.pre`, `edit.pre`) and combines what they say.
pub trait PreToolHooks: Send + Sync + 'static {
    fn advise<'a>(&'a self, request: &'a Request, mode: Mode) -> BoxFuture<'a, Option<HookAdvice>>;
}

/// No hooks.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoHooks;

impl PreToolHooks for NoHooks {
    fn advise<'a>(&'a self, _: &'a Request, _: Mode) -> BoxFuture<'a, Option<HookAdvice>> {
        Box::pin(async { None })
    }
}

/// What a classifier sees. v0.1 has no classifier backend, so the session passes the action
/// and mode and leaves the context empty.
#[derive(Clone, Copy, Debug)]
pub struct ClassifyRequest<'a> {
    pub action: &'a Action,
    pub mode: Mode,
    /// Recent transcript turns.
    pub transcript: &'a [String],
    /// The task spec.
    pub task: Option<&'a str>,
    /// The project's rules files (`AGENTS.md` and the like).
    pub rules_files: &'a [String],
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ClassifierError(pub String);

/// Decides what rules left open, in Auto mode. Implementations enforce their own timeout and
/// return an error when it passes; an error means Ask, never Allow.
pub trait Classifier: Send + Sync + 'static {
    /// A verdict and reason, or `None` for no opinion.
    fn classify<'a>(
        &'a self,
        request: ClassifyRequest<'a>,
    ) -> BoxFuture<'a, Result<Option<(Verdict, String)>, ClassifierError>>;
}

/// No classifier: Auto falls through to its mode default.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoClassifier;

impl Classifier for NoClassifier {
    fn classify<'a>(
        &'a self,
        _: ClassifyRequest<'a>,
    ) -> BoxFuture<'a, Result<Option<(Verdict, String)>, ClassifierError>> {
        Box::pin(async { Ok(None) })
    }
}

/// Decides agents' actions. One per session, since remembered decisions are per session.
pub trait Policy: Send + Sync + 'static {
    fn decide<'a>(&'a self, request: &'a Request, mode: Mode) -> BoxFuture<'a, Outcome>;

    /// Remember a human's allow or deny of exactly `action` for the rest of the session.
    fn remember(&self, action: &Action, verdict: Verdict);
}

/// The policy engine.
pub struct Engine {
    rules: Arc<Rules>,
    scanner: Arc<SecretScanner>,
    hooks: Arc<dyn PreToolHooks>,
    classifier: Arc<dyn Classifier>,
    remembered: Mutex<Vec<(Action, Verdict)>>,
}

impl Engine {
    pub fn new(
        rules: Arc<Rules>,
        scanner: Arc<SecretScanner>,
        hooks: Arc<dyn PreToolHooks>,
        classifier: Arc<dyn Classifier>,
    ) -> Self {
        Self {
            rules,
            scanner,
            hooks,
            classifier,
            remembered: Mutex::default(),
        }
    }

    /// Steps 1 and 2: hard denies and guards. `None` if neither decides.
    pub fn check(&self, request: &Request, mode: Mode) -> Option<Decision> {
        let action = &request.action;
        let cmd = match action {
            Action::Exec { argv, .. } => Some(exec::CommandLine::parse(argv)),
            _ => None,
        };
        let git: Vec<git::GitWrite> = cmd
            .iter()
            .flat_map(|c| c.segments.iter().map(git::check))
            .collect();
        if git.contains(&git::GitWrite::Yes) {
            return Some(Decision::new(
                Verdict::Deny,
                git::REASON,
                Source::GitSharedState,
            ));
        }
        let deny = self.rules.first_match(RuleList::Deny, action, mode);
        if let Some(m) = deny.as_ref().filter(|m| m.uncertain.is_none()) {
            return Some(rule_decision(Verdict::Deny, m));
        }
        if let Some((_, Verdict::Deny)) = self.recalled(action) {
            return Some(Decision::new(
                Verdict::Deny,
                "denied earlier in this session",
                Source::Remembered(RuleList::Deny),
            ));
        }
        if git.contains(&git::GitWrite::Maybe) {
            return Some(Decision::new(
                Verdict::Ask,
                "an expansion decides which git subcommand runs, and git commit, push, stash, \
                 and branch are refused to agents; a human decides",
                Source::GitSharedState,
            ));
        }
        if let Some(m) = &deny {
            // Uncertain: an expansion could complete the deny rule.
            return Some(rule_decision(Verdict::Deny, m));
        }
        if let Some(why) = cmd.as_ref().and_then(|c| c.opaque.as_ref()) {
            return Some(Decision::new(
                Verdict::Ask,
                format!("{why}; a human decides"),
                Source::OpaqueCommand,
            ));
        }
        if let Action::Edit { path, .. } = action {
            let Some(path) = paths::normalize(path) else {
                return Some(Decision::new(
                    Verdict::Deny,
                    format!("{path:?} is outside the worktree"),
                    Source::OutsideWorktree,
                ));
            };
            if path.split('/').any(is_git_dir_name) {
                return Some(Decision::new(
                    Verdict::Deny,
                    format!("{path:?} is inside .git"),
                    Source::OutsideWorktree,
                ));
            }
            if let Some(p) = self.rules.protected(&path) {
                return Some(Decision::new(
                    Verdict::Deny,
                    format!(
                        "{path:?} is a protected path ({} pattern {:?}); a human edits it outside \
                         the agent",
                        p.layer, p.pattern
                    ),
                    Source::ProtectedPath,
                ));
            }
        }
        let mut kinds = Vec::new();
        if let Some(content) = &request.content {
            kinds.extend(self.scanner.kinds(content));
        }
        if let Action::Exec { argv, .. } = action {
            for kind in self.scanner.kinds(&argv.join(" ")) {
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                }
            }
        }
        if !kinds.is_empty() {
            return Some(Decision::new(
                Verdict::Ask,
                format!(
                    "looks like it contains a secret ({}); confirm it isn't a real credential",
                    kinds.join(", ")
                ),
                Source::SecretScan,
            ));
        }
        None
    }

    /// Steps 3 to 6 for a request that passed [`Engine::check`]: the caller may run that
    /// check, which scans the content, on the blocking pool for large content.
    pub async fn decide_checked(&self, request: &Request, mode: Mode) -> Outcome {
        let mut outcome = Outcome::of(
            Decision::new(Verdict::Ask, "", Source::Mode(mode)),
            request.action.clone(),
        );
        let mut hook_verdict = None;
        if let Some(advice) = self.hooks.advise(request, mode).await {
            outcome.note = advice.note.clone();
            let source = Source::Hook(advice.hook.clone());
            if let Some(argv) = advice.rewrite {
                let Action::Exec { cwd, .. } = &outcome.action else {
                    outcome.decision = Decision::new(
                        Verdict::Deny,
                        "a hook tried to rewrite an action that isn't a command",
                        source,
                    );
                    return outcome;
                };
                outcome.action = Action::Exec {
                    argv,
                    cwd: cwd.clone(),
                };
                outcome.rewritten_by = Some(advice.rewritten_by.unwrap_or(advice.hook.clone()));
                let rewritten = Request {
                    action: outcome.action.clone(),
                    content: request.content.clone(),
                };
                if let Some(decision) = self.check(&rewritten, mode) {
                    outcome.decision = decision;
                    return outcome;
                }
            }
            match advice.verdict {
                Some(Verdict::Allow) if !advice.trusted => {}
                Some(verdict) => {
                    let reason = advice.reason.unwrap_or_else(|| {
                        format!("hook {} said {}", advice.hook, verdict_name(verdict))
                    });
                    hook_verdict = Some(Decision::new(verdict, reason, source));
                }
                None => {}
            }
        }
        outcome.decision = self.decide_from(&outcome.action, mode, hook_verdict).await;
        outcome
    }

    /// Steps 3's verdict (if any) and 4 to 6, for `action`.
    async fn decide_from(&self, action: &Action, mode: Mode, hook: Option<Decision>) -> Decision {
        if let Some(hook) = &hook
            && hook.verdict != Verdict::Allow
        {
            return hook.clone();
        }
        if let Some(m) = self.rules.first_match(RuleList::Ask, action, mode) {
            return rule_decision(Verdict::Ask, &m);
        }
        if let Some(hook) = hook {
            return hook;
        }
        if let Some((_, Verdict::Allow)) = self.recalled(action) {
            return Decision::new(
                Verdict::Allow,
                "allowed earlier in this session",
                Source::Remembered(RuleList::Allow),
            );
        }
        if let Some(m) = self.rules.first_match(RuleList::Allow, action, mode) {
            return rule_decision(Verdict::Allow, &m);
        }
        if mode == Mode::Auto {
            let request = ClassifyRequest {
                action,
                mode,
                transcript: &[],
                task: None,
                rules_files: &[],
            };
            match self.classifier.classify(request).await {
                Ok(Some((verdict, reason))) => {
                    return Decision::new(verdict, reason, Source::Classifier);
                }
                Ok(None) => {}
                Err(e) => {
                    return Decision::new(
                        Verdict::Ask,
                        format!("the classifier failed ({e}), so a human decides"),
                        Source::Classifier,
                    );
                }
            }
        }
        let (verdict, reason) = mode_default(action, mode);
        Decision::new(verdict, reason, Source::Mode(mode))
    }

    /// A remembered decision for exactly `action`.
    fn recalled(&self, action: &Action) -> Option<(Action, Verdict)> {
        lock(&self.remembered)
            .iter()
            .find(|(a, _)| same_action(a, action))
            .cloned()
    }
}

impl Policy for Engine {
    fn decide<'a>(&'a self, request: &'a Request, mode: Mode) -> BoxFuture<'a, Outcome> {
        Box::pin(async move {
            if let Action::Network { .. } = request.action {
                let decision = Decision::new(
                    Verdict::Deny,
                    "network access is governed by the agent profile's [network] allow list",
                    Source::NetworkAllowlist,
                );
                return Outcome::of(decision, request.action.clone());
            }
            if let Some(decision) = self.check(request, mode) {
                return Outcome::of(decision, request.action.clone());
            }
            self.decide_checked(request, mode).await
        })
    }

    fn remember(&self, action: &Action, verdict: Verdict) {
        if verdict == Verdict::Ask {
            return;
        }
        let mut remembered = lock(&self.remembered);
        remembered.retain(|(a, _)| !same_action(a, action));
        remembered.push((action.clone(), verdict));
    }
}

/// The `exec` pattern that matches exactly `action` in a policy file, or why there is none.
/// Only a command that is one plain simple command whose words hold no spaces or tabs has one:
/// allow rules match nothing else, and the pattern matches the space-joined argv, so a word
/// with a space in it would also match the same text split into more words. Edits have none
/// yet, since an edit pattern also matches everything under the path.
pub fn exact_pattern(action: &Action) -> Result<String, String> {
    let Action::Exec { argv, .. } = action else {
        return Err("only commands can be remembered in a policy file".into());
    };
    let cmd = exec::CommandLine::parse(argv);
    if !cmd.simple {
        return Err(
            "a rule can only name a single plain command, with no shell syntax, \
                    wrapper, or expansion"
                .into(),
        );
    }
    if cmd.words.iter().any(|w| w.contains([' ', '\t'])) {
        return Err("a rule can't tell a word with a space in it from two words".into());
    }
    Ok(exec::escape(&cmd.line))
}

/// The verdict when nothing earlier decided, and why.
///
/// | Action | Ask | Accept edits | Auto | Full access |
/// | --- | --- | --- | --- | --- |
/// | edit | ask | allow | allow | allow |
/// | exec | ask | ask | ask | allow |
/// | mcp | ask | ask | ask | allow |
/// | git | ask | ask | allow | allow |
///
/// Rules-only Auto asks for commands no rule allows: without a classifier it fails closed.
/// Network actions are denied in every mode, since the egress proxy's allowlist governs them.
pub fn mode_default(action: &Action, mode: Mode) -> (Verdict, String) {
    use Mode::*;
    let (what, allowed) = match action {
        Action::Edit { .. } => ("edits", mode != Ask),
        Action::Exec { .. } => ("commands", mode == FullAccess),
        Action::Mcp { .. } => ("MCP tool calls", mode == FullAccess),
        Action::Git { .. } => ("git operations", matches!(mode, Auto | FullAccess)),
        Action::Network { .. } => {
            return (
                Verdict::Deny,
                "network access is governed by the agent profile's [network] allow list".into(),
            );
        }
    };
    let label = mode_label(mode);
    if allowed {
        (
            Verdict::Allow,
            format!("{what} are allowed in {label} mode"),
        )
    } else {
        (Verdict::Ask, format!("not in allow rules ({label} mode)"))
    }
}

/// The decision a matching rule makes. A rule that matches only if an expansion turns out a
/// certain way asks instead: in no mode does uncertainty allow, and it never denies outright.
fn rule_decision(verdict: Verdict, m: &rules::Matched) -> Decision {
    if let Some(word) = &m.uncertain {
        return Decision::new(
            Verdict::Ask,
            format!(
                "{word} could expand to something {} {} rule {:?} matches; a human decides",
                m.layer, m.list, m.pattern
            ),
            Source::Rule {
                layer: m.layer,
                list: m.list,
                index: m.index,
            },
        );
    }
    let reason = match &m.reason {
        Some(reason) => format!("{reason} (matches {:?})", m.pattern),
        None => format!("matches {} {} rule {:?}", m.layer, m.list, m.pattern),
    };
    Decision::new(
        verdict,
        reason,
        Source::Rule {
            layer: m.layer,
            list: m.list,
            index: m.index,
        },
    )
}

/// Whether two actions are the same for remembering: commands by argv (any directory), edits
/// by normalized path and kind.
fn same_action(a: &Action, b: &Action) -> bool {
    match (a, b) {
        (Action::Exec { argv: x, .. }, Action::Exec { argv: y, .. }) => x == y,
        (Action::Edit { path: p, kind: k }, Action::Edit { path: q, kind: l }) => {
            k == l && paths::normalize(p).is_some() && paths::normalize(p) == paths::normalize(q)
        }
        _ => a == b,
    }
}

/// `.git` in any spelling a case-insensitive filesystem would resolve to it, including the
/// characters HFS+ ignores (the set git guards against, CVE-2014-9390), as the ACP workspace
/// checks.
fn is_git_dir_name(segment: &str) -> bool {
    let visible: String = segment
        .chars()
        .filter(|c| {
            !matches!(c, '\u{200c}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{206a}'..='\u{206f}' | '\u{feff}')
        })
        .collect();
    visible.eq_ignore_ascii_case(".git")
}

pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Ask => "ask",
        Mode::AcceptEdits => "accept-edits",
        Mode::Auto => "auto",
        Mode::FullAccess => "full-access",
    }
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Ask => "Ask",
        Mode::AcceptEdits => "Accept edits",
        Mode::Auto => "Auto",
        Mode::FullAccess => "Full access",
    }
}

pub fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Allow => "allow",
        Verdict::Deny => "deny",
        Verdict::Ask => "ask",
    }
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}
