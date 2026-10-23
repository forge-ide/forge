//! Branch names for new sessions: conventional type prefixes and a slug of the title.

use forge_config::BranchRules;

/// Longest slug, in characters; longer titles are cut at a word boundary.
const MAX_SLUG: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("title {0:?} has no letters or digits to name a branch after; pass a branch name instead")]
pub struct NoSlug(pub String);

/// The branch name for a session titled `title`.
///
/// A title that starts with a configured type in conventional-commit style (`fix: ...`,
/// `fix(api): ...`, `fix!: ...`) uses that type and the rest as the slug; any other title uses
/// the first configured type and the whole title.
pub fn derive_branch(title: &str, rules: &BranchRules) -> Result<String, NoSlug> {
    let (kind, rest) =
        split_type(title, &rules.types).unwrap_or_else(|| (rules.types[0].as_str(), title));
    let slug = slugify(rest);
    if slug.is_empty() {
        return Err(NoSlug(title.to_owned()));
    }
    Ok(rules
        .format
        .replace("{type}", kind)
        .replace("{slug}", &slug))
}

fn split_type<'a>(title: &'a str, types: &'a [String]) -> Option<(&'a str, &'a str)> {
    let (head, rest) = title.split_once(':')?;
    let head = head.trim();
    let head = head.strip_suffix('!').unwrap_or(head);
    let name = match head.split_once('(') {
        Some((name, scope)) if scope.ends_with(')') => name,
        Some(_) => return None,
        None => head,
    };
    let kind = types.iter().find(|t| t.eq_ignore_ascii_case(name))?;
    Some((kind.as_str(), rest))
}

/// Lowercase ASCII letters and digits, with runs of anything else as one hyphen.
pub fn slugify(text: &str) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            current.push(c.to_ascii_lowercase());
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    let mut slug = String::new();
    for word in words {
        let extra = if slug.is_empty() { 0 } else { 1 };
        if slug.len() + extra + word.len() > MAX_SLUG {
            if slug.is_empty() {
                slug.push_str(&word[..MAX_SLUG]);
            }
            break;
        }
        if extra == 1 {
            slug.push('-');
        }
        slug.push_str(&word);
    }
    slug
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> BranchRules {
        BranchRules {
            format: "{type}/{slug}".into(),
            types: vec!["feat".into(), "fix".into(), "bug".into()],
        }
    }

    #[test]
    fn conventional_prefixes_pick_the_type() {
        let r = rules();
        for (title, branch) in [
            ("fix: Retry loop never ends!", "fix/retry-loop-never-ends"),
            ("Fix(api): retry", "fix/retry"),
            ("bug!: crash on start", "bug/crash-on-start"),
            ("Add a greeting", "feat/add-a-greeting"),
            ("chore: tidy", "feat/chore-tidy"),
            ("fix (no colon here)", "feat/fix-no-colon-here"),
            ("note: 10:30 meeting", "feat/note-10-30-meeting"),
        ] {
            assert_eq!(derive_branch(title, &r).unwrap(), branch, "{title}");
        }
    }

    #[test]
    fn formats_apply() {
        let r = BranchRules {
            format: "jeff/{type}/{slug}".into(),
            ..rules()
        };
        assert_eq!(derive_branch("fix: x", &r).unwrap(), "jeff/fix/x");
    }

    #[test]
    fn slugs_are_short_and_plain() {
        assert_eq!(slugify("  Héllo, World!! "), "h-llo-world");
        let long = "word ".repeat(20);
        let slug = slugify(&long);
        assert!(slug.len() <= MAX_SLUG && slug.ends_with("word"), "{slug}");
        assert_eq!(slugify(&"x".repeat(80)).len(), MAX_SLUG);
        assert!(derive_branch("fix: !!!", &rules()).is_err());
        assert!(derive_branch("日本語", &rules()).is_err());
    }
}
