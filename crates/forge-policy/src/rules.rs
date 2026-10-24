//! Policy files compiled for matching: deny, ask, and allow rules, and protected paths, from
//! every layer.

use forge_api::model::{Action, Mode};
use forge_config::{PolicyConfig, PolicyRule, ProfileMode};

use crate::exec::{CommandLine, ExecGlob, Segment};
use crate::paths::{PathPatterns, normalize};
use crate::{Layer, PolicyError, RuleList};

/// Paths agents may never edit through Forge, whatever the files say.
pub const BUILTIN_PROTECTED: &[&str] = &[
    ".env",
    ".env.*",
    "*.pem",
    "*.key",
    ".forge/",
    // Files a developer tool runs code from on the user's machine, outside the sandbox, when
    // they open, enter, or commit to the project:
    // direnv, on entering the directory;
    ".envrc",
    ".envrc.local",
    ".direnv/",
    // git hooks on the user's own commits (husky, a `core.hooksPath` of `.githooks/`,
    // lefthook, pre-commit);
    ".husky/",
    ".githooks/",
    "lefthook.yml",
    "lefthook.yaml",
    ".lefthook.yml",
    ".lefthook.yaml",
    "lefthook-local.yml",
    "lefthook-local.yaml",
    ".pre-commit-config.yaml",
    // mise tasks and env hooks;
    "mise.toml",
    ".mise.toml",
    "mise.local.toml",
    ".mise.local.toml",
    ".mise/",
    ".config/mise.toml",
    // VS Code tasks, launch configurations, and settings that name programs to run.
    ".vscode/tasks.json",
    ".vscode/launch.json",
    ".vscode/settings.json",
    "*.code-workspace",
];

/// Every layer's rules, compiled once per session launch.
#[derive(Clone, Debug)]
pub struct Rules {
    deny: Vec<Rule>,
    ask: Vec<Rule>,
    allow: Vec<Rule>,
    protected: Vec<Protected>,
}

#[derive(Clone, Debug)]
struct Rule {
    layer: Layer,
    index: usize,
    exec: Vec<ExecGlob>,
    edit: Option<PathPatterns>,
    edit_patterns: Vec<String>,
    reason: Option<String>,
    modes: Option<Vec<Mode>>,
}

#[derive(Clone, Debug)]
struct Protected {
    layer: Layer,
    patterns: PathPatterns,
    sources: Vec<String>,
}

/// A rule that matched an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Matched {
    pub layer: Layer,
    pub list: RuleList,
    pub index: usize,
    /// The pattern that matched.
    pub pattern: String,
    pub reason: Option<String>,
    /// Set when the rule matches only if an expansion turns out a certain way: the expansion.
    pub uncertain: Option<String>,
}

/// A protected-path pattern that matched an edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProtectedMatch {
    pub layer: Layer,
    pub pattern: String,
}

impl Rules {
    /// Compile `layers`, which must be in order from the most to the least authoritative
    /// (managed, user, project). The built-in protected paths are always included.
    pub fn compile(layers: &[(Layer, &PolicyConfig)]) -> Result<Self, PolicyError> {
        let mut rules = Self {
            deny: Vec::new(),
            ask: Vec::new(),
            allow: Vec::new(),
            protected: vec![Protected {
                layer: Layer::Builtin,
                patterns: PathPatterns::new(BUILTIN_PROTECTED).expect("built-in patterns compile"),
                sources: BUILTIN_PROTECTED.iter().map(|p| (*p).to_owned()).collect(),
            }],
        };
        for (layer, config) in layers {
            let problems = config.validate();
            if !problems.is_empty() {
                return Err(PolicyError {
                    layer: *layer,
                    message: problems.join("; "),
                });
            }
            let fail = |message: String| PolicyError {
                layer: *layer,
                message,
            };
            for (list, source, out) in [
                (RuleList::Deny, &config.deny, &mut rules.deny),
                (RuleList::Ask, &config.ask, &mut rules.ask),
                (RuleList::Allow, &config.allow, &mut rules.allow),
            ] {
                for (index, rule) in source.iter().enumerate() {
                    out.push(
                        Rule::compile(*layer, index, rule)
                            .map_err(|e| fail(format!("{list}[{index}]: {e}")))?,
                    );
                }
            }
            if !config.protected.paths.is_empty() {
                rules.protected.push(Protected {
                    layer: *layer,
                    patterns: PathPatterns::new(&config.protected.paths)
                        .map_err(|e| fail(format!("protected: {e}")))?,
                    sources: config.protected.paths.clone(),
                });
            }
        }
        Ok(rules)
    }

    /// The first rule in `list` that matches `action` in `mode`, most authoritative layer
    /// first. A rule that certainly matches wins over one that matches only if an expansion
    /// turns out a certain way.
    pub fn first_match(&self, list: RuleList, action: &Action, mode: Mode) -> Option<Matched> {
        let rules = match list {
            RuleList::Deny => &self.deny,
            RuleList::Ask => &self.ask,
            RuleList::Allow => &self.allow,
        };
        let target = Target::of(action)?;
        let applies = |rule: &&Rule| !rule.modes.as_ref().is_some_and(|m| !m.contains(&mode));
        let found = |rule: &Rule, (pattern, uncertain): (String, Option<String>)| Matched {
            layer: rule.layer,
            list,
            index: rule.index,
            pattern,
            reason: rule.reason.clone(),
            uncertain,
        };
        let certain = rules
            .iter()
            .filter(applies)
            .find_map(|rule| rule.matches(&target, list).map(|m| found(rule, (m, None))));
        certain.or_else(|| {
            rules.iter().filter(applies).find_map(|rule| {
                rule.could_match(&target, list)
                    .map(|(p, word)| found(rule, (p, Some(word))))
            })
        })
    }

    /// The first protected-path pattern matching `path` (already normalized).
    pub fn protected(&self, path: &str) -> Option<ProtectedMatch> {
        self.protected.iter().find_map(|p| {
            p.patterns.first_match(path).map(|i| ProtectedMatch {
                layer: p.layer,
                pattern: p.sources[i].clone(),
            })
        })
    }
}

/// What a rule matches against.
enum Target {
    Exec(CommandLine),
    Edit(String),
}

impl Target {
    fn of(action: &Action) -> Option<Self> {
        match action {
            Action::Exec { argv, .. } => Some(Self::Exec(CommandLine::parse(argv))),
            Action::Edit { path, .. } => normalize(path).map(Self::Edit),
            _ => None,
        }
    }
}

impl Rule {
    fn compile(layer: Layer, index: usize, rule: &PolicyRule) -> Result<Self, String> {
        let edit = if rule.edit.is_empty() {
            None
        } else {
            Some(PathPatterns::new(&rule.edit).map_err(|e| e.to_string())?)
        };
        Ok(Self {
            layer,
            index,
            exec: rule.exec.iter().map(|p| ExecGlob::new(p)).collect(),
            edit,
            edit_patterns: rule.edit.clone(),
            reason: rule.reason.clone(),
            modes: rule
                .modes
                .as_ref()
                .map(|m| m.iter().copied().map(mode_of).collect()),
        })
    }

    /// The pattern of this rule that certainly matches, if any. Allow rules match only a
    /// simple command as a whole; deny and ask rules match the whole line or any simple
    /// command in it with no expansion.
    fn matches(&self, target: &Target, list: RuleList) -> Option<String> {
        match target {
            Target::Exec(cmd) => {
                if list == RuleList::Allow {
                    return cmd
                        .simple
                        .then(|| self.exec.iter().find(|g| g.matches(&cmd.line)))
                        .flatten()
                        .map(|g| g.pattern().to_owned());
                }
                let lines: Vec<String> = std::iter::once(cmd.line.clone())
                    .chain(
                        cmd.segments
                            .iter()
                            .filter(|s| s.is_literal())
                            .map(Segment::line),
                    )
                    .collect();
                // A command with an expansion still matches for certain when its literal
                // prefix already holds every piece of a pattern ending in `*`.
                let prefixes: Vec<String> = cmd
                    .segments
                    .iter()
                    .filter(|s| !s.is_literal() && s.literal > 0)
                    .map(Segment::literal_prefix)
                    .collect();
                self.exec
                    .iter()
                    .find(|g| {
                        lines.iter().any(|l| g.matches(l))
                            || prefixes.iter().any(|p| g.matches_prefix_certainly(p))
                    })
                    .map(|g| g.pattern().to_owned())
            }
            Target::Edit(path) => {
                let i = self.edit.as_ref()?.first_match(path)?;
                Some(self.edit_patterns[i].clone())
            }
        }
    }

    /// For a deny or ask rule: a pattern that a simple command with an expansion could match
    /// once expanded, and that expansion. Allow rules never match this way.
    fn could_match(&self, target: &Target, list: RuleList) -> Option<(String, String)> {
        let Target::Exec(cmd) = target else {
            return None;
        };
        if list == RuleList::Allow {
            return None;
        }
        cmd.segments
            .iter()
            .filter(|s| !s.is_literal() && s.literal > 0)
            .find_map(|seg| {
                let prefix = seg.literal_prefix();
                let glob = self.exec.iter().find(|g| g.could_match_prefix(&prefix))?;
                Some((
                    glob.pattern().to_owned(),
                    seg.first_expansion().unwrap_or_default().to_owned(),
                ))
            })
    }
}

pub fn mode_of(mode: ProfileMode) -> Mode {
    match mode {
        ProfileMode::Ask => Mode::Ask,
        ProfileMode::AcceptEdits => Mode::AcceptEdits,
        ProfileMode::Auto => Mode::Auto,
        ProfileMode::FullAccess => Mode::FullAccess,
    }
}
