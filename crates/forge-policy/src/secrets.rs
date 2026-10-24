//! The secret-scan guard: finds credentials in text an agent wants to write, and redacts them
//! from text Forge records.
//!
//! The patterns are built in and not configurable; on top of them, a scanner can look for the
//! exact values of secrets the egress proxy injects.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::Range;

use regex::Regex;

/// Built-in patterns, as `(kind, regex)`. Earlier entries win where matches overlap.
const PATTERNS: &[(&str, &str)] = &[
    // The whole block, through its END line, or to the end of the text when it isn't closed.
    (
        "private-key",
        r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----(?:[\s\S]*?-----END (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----|[\s\S]*)",
    ),
    ("aws-access-key", r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b"),
    ("github-token", r"\bgh[pousr]_[A-Za-z0-9]{36,}\b"),
    ("github-token", r"\bgithub_pat_[A-Za-z0-9_]{22,}\b"),
    // Distinctive prefixes match even when glued to an identifier (`X_sk-ant-...`).
    ("anthropic-key", r"sk-ant-[A-Za-z0-9_-]{20,}"),
    (
        "openai-key",
        r"sk-(?:proj|svcacct|admin)-[A-Za-z0-9_-]{20,}",
    ),
    ("openai-key", r"\bsk-[A-Za-z0-9_-]{32,}"),
    ("google-api-key", r"AIza[0-9A-Za-z_-]{35}"),
    ("slack-token", r"xox[abeprs]-[A-Za-z0-9-]{10,}"),
    ("slack-token", r"xapp-[0-9]-[A-Za-z0-9-]{10,}"),
    ("stripe-key", r"[rs]k_live_[A-Za-z0-9]{24,}"),
    ("gitlab-token", r"glpat-[A-Za-z0-9_-]{20,}"),
    ("npm-token", r"\bnpm_[A-Za-z0-9]{36}\b"),
    ("pypi-token", r"pypi-AgEIcHlwaS5vcmc[A-Za-z0-9_-]{50,}"),
    ("huggingface-token", r"\bhf_[A-Za-z0-9]{34,}\b"),
    ("sendgrid-key", r"SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43}"),
];

/// The first line of a private key block.
const KEY_BEGIN: &str = r"-----BEGIN (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----";
/// The last line of a private key block.
const KEY_END: &str = r"-----END (?:[A-Z0-9]+ )*PRIVATE KEY(?: BLOCK)?-----";

/// Values shorter than this aren't scanned for, since they'd match ordinary text.
const MIN_VALUE_LEN: usize = 8;

/// One secret found in a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finding {
    /// What kind of secret, such as `aws-access-key`, or `injected-secret` for a known value.
    pub kind: &'static str,
    /// Byte range in the scanned text.
    pub range: Range<usize>,
}

/// Finds and redacts secrets.
#[derive(Clone, Debug)]
pub struct SecretScanner {
    patterns: Vec<(&'static str, Regex)>,
    values: Vec<String>,
    key_begin: Regex,
    key_end: Regex,
}

impl Default for SecretScanner {
    fn default() -> Self {
        Self::new(Vec::new())
    }
}

impl SecretScanner {
    /// A scanner for the built-in patterns plus the exact `values` given, such as the secrets
    /// a session's proxy injects.
    pub fn new(values: Vec<String>) -> Self {
        let patterns = PATTERNS
            .iter()
            .map(|(kind, re)| (*kind, Regex::new(re).expect("built-in patterns compile")))
            .collect();
        let values = values
            .into_iter()
            .filter(|v| v.len() >= MIN_VALUE_LEN)
            .collect();
        Self {
            patterns,
            values,
            key_begin: Regex::new(KEY_BEGIN).expect("built-in patterns compile"),
            key_end: Regex::new(KEY_END).expect("built-in patterns compile"),
        }
    }

    /// The length of the longest value scanned for, in bytes.
    pub fn longest_value(&self) -> usize {
        self.values.iter().map(String::len).max().unwrap_or(0)
    }

    /// The length of the longest value scanned for that spans lines, or 0.
    pub fn longest_multiline_value(&self) -> usize {
        self.values
            .iter()
            .filter(|v| v.contains('\n'))
            .map(String::len)
            .max()
            .unwrap_or(0)
    }

    /// Where a private key block starts in `text` without its END line after it.
    pub fn open_key_block(&self, text: &str) -> Option<usize> {
        let begin = self.key_begin.find_iter(text).last()?;
        (!self.key_end.is_match(&text[begin.end()..])).then_some(begin.start())
    }

    /// Where the first private key END line in `text` ends.
    pub fn key_block_end(&self, text: &str) -> Option<usize> {
        self.key_end.find(text).map(|m| m.end())
    }

    /// Every secret in `text`, in order, without overlaps.
    pub fn scan(&self, text: &str) -> Vec<Finding> {
        let mut found: Vec<Finding> = Vec::new();
        for value in &self.values {
            found.extend(text.match_indices(value.as_str()).map(|(i, v)| Finding {
                kind: "injected-secret",
                range: i..i + v.len(),
            }));
        }
        for (kind, re) in &self.patterns {
            found.extend(re.find_iter(text).map(|m| Finding {
                kind,
                range: m.range(),
            }));
        }

        // Keep the first-listed finding where two overlap: values, then patterns in order.
        // Kept findings never overlap, so a new one need only be checked against its
        // neighbours by start.
        let mut kept: BTreeMap<usize, Finding> = BTreeMap::new();
        for f in found {
            let before = kept.range(..=f.range.start).next_back();
            let after = kept.range(f.range.start..).next();
            let clashes = before.is_some_and(|(_, k)| k.range.end > f.range.start)
                || after.is_some_and(|(_, k)| k.range.start < f.range.end);
            if !clashes {
                kept.insert(f.range.start, f);
            }
        }
        kept.into_values().collect()
    }

    /// Like [`SecretScanner::scan`], for text that may have been cut from longer text (a
    /// chunk of output, the end of long output): an END line of a private key with no BEGIN
    /// before it means everything up to it is key material. Edits and command lines are never
    /// cut, so the policy engine scans them plainly.
    pub fn scan_cut(&self, text: &str) -> Vec<Finding> {
        let mut found = self.scan(text);
        if let Some(end) = self.key_end.find(text)
            && !self.key_begin.is_match(&text[..end.start()])
        {
            found.retain(|f| f.range.start >= end.end());
            found.insert(
                0,
                Finding {
                    kind: "private-key",
                    range: 0..end.end(),
                },
            );
        }
        found
    }

    /// `text` with each secret replaced by `[redacted:<kind>]`, treating it as possibly cut
    /// (see [`SecretScanner::scan_cut`]), as for what Forge records.
    pub fn redact<'t>(&self, text: &'t str) -> Cow<'t, str> {
        let found = self.scan_cut(text);
        if found.is_empty() {
            return Cow::Borrowed(text);
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0;
        for f in found {
            out.push_str(&text[at..f.range.start]);
            out.push_str("[redacted:");
            out.push_str(f.kind);
            out.push(']');
            at = f.range.end;
        }
        out.push_str(&text[at..]);
        Cow::Owned(out)
    }

    /// The distinct kinds of secret in `text`, in order of first appearance.
    pub fn kinds(&self, text: &str) -> Vec<&'static str> {
        let mut kinds = Vec::new();
        for f in self.scan(text) {
            if !kinds.contains(&f.kind) {
                kinds.push(f.kind);
            }
        }
        kinds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fake credentials assembled at run time, so this file never holds one a scanner (ours or
    /// a hosting provider's) would flag.
    fn fake(prefix: &str, body: char, n: usize) -> String {
        format!("{prefix}{}", body.to_string().repeat(n))
    }

    #[test]
    fn finds_each_built_in_kind() {
        let cases = [
            (fake("AKIA", 'Q', 16), "aws-access-key"),
            (fake("ghp_", 'a', 36), "github-token"),
            (fake("github_pat_", 'b', 30), "github-token"),
            (fake("sk-ant-api03-", 'c', 40), "anthropic-key"),
            (fake("sk-proj-", 'd', 40), "openai-key"),
            (fake("AIza", 'e', 35), "google-api-key"),
            (fake("xoxb-", '1', 20), "slack-token"),
            (fake("sk_live_", 'f', 24), "stripe-key"),
            (fake("glpat-", 'g', 20), "gitlab-token"),
            (fake("npm_", 'h', 36), "npm-token"),
            (fake("pypi-AgEIcHlwaS5vcmc", 'i', 60), "pypi-token"),
            (fake("hf_", 'j', 34), "huggingface-token"),
            (fake("xapp-1-", 'k', 20), "slack-token"),
            (
                format!("{}.{}", fake("SG.", 'l', 22), "m".repeat(43)),
                "sendgrid-key",
            ),
            (format!("X_{}", fake("sk-ant-", 'n', 30)), "anthropic-key"),
            (
                format!("-----BEGIN {}PRIVATE KEY-----", "OPENSSH "),
                "private-key",
            ),
        ];
        let scanner = SecretScanner::default();
        for (secret, kind) in cases {
            let text = format!("let key = \"{secret}\";\n");
            assert_eq!(scanner.kinds(&text), [kind], "{secret}");
        }
    }

    #[test]
    fn ignores_ordinary_text() {
        let scanner = SecretScanner::default();
        for text in [
            "fn main() { println!(\"hello\"); }",
            "AKIA is a prefix",
            "sk-short",
            "ghp_tooShort",
            "-----BEGIN PUBLIC KEY-----",
            "the task-ant-colony module",
        ] {
            assert!(scanner.scan(text).is_empty(), "{text}");
        }
    }

    #[test]
    fn finds_injected_values_and_skips_short_ones() {
        let scanner = SecretScanner::new(vec!["opaque-value-123".into(), "short".into()]);
        assert_eq!(
            scanner.kinds("token=opaque-value-123 and short"),
            ["injected-secret"]
        );
    }

    #[test]
    fn many_findings_scan_quickly() {
        let key = fake("AKIA", 'Q', 16);
        let text = format!("{key} ").repeat(50_000);
        let start = std::time::Instant::now();
        assert_eq!(SecretScanner::default().scan(&text).len(), 50_000);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            start.elapsed()
        );
    }

    #[test]
    fn whole_key_blocks_are_redacted_even_cut_short() {
        let s = SecretScanner::default();
        let key = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\nq1w2e3r4\n-----END RSA PRIVATE KEY-----";
        assert_eq!(
            s.redact(&format!("preamble\n{key}\ntrailer")),
            "preamble\n[redacted:private-key]\ntrailer"
        );
        // Not closed: to the end of the text.
        assert_eq!(
            s.redact("x\n-----BEGIN PRIVATE KEY-----\nMIIE\nabcd"),
            "x\n[redacted:private-key]"
        );
        // Its head cut off: from the start of the text through the END line, but only for
        // text that may have been cut, not for policy's plain scan.
        assert_eq!(
            s.redact("q1w2e3r4\n-----END RSA PRIVATE KEY-----\nafter"),
            "[redacted:private-key]\nafter"
        );
        assert!(
            s.kinds("docs: a key ends with -----END PRIVATE KEY-----")
                .is_empty()
        );
        assert_eq!(
            s.open_key_block("a\n-----BEGIN EC PRIVATE KEY-----\nMHc"),
            Some(2)
        );
        assert_eq!(s.open_key_block(key), None);
        assert_eq!(
            s.key_block_end("ab\n-----END PRIVATE KEY-----\nc"),
            Some(28)
        );
        assert_eq!(
            SecretScanner::new(vec!["0123456789abcdef".into()]).longest_value(),
            16
        );
    }

    #[test]
    fn redacts_every_secret_once() {
        let aws = fake("AKIA", 'Z', 16);
        let ant = fake("sk-ant-", 'x', 30);
        let scanner = SecretScanner::default();
        let text = format!("a={aws} b={ant} c={aws}");
        assert_eq!(
            scanner.redact(&text),
            "a=[redacted:aws-access-key] b=[redacted:anthropic-key] c=[redacted:aws-access-key]"
        );
        // An Anthropic key also looks like an OpenAI one; only the first kind is reported.
        assert_eq!(scanner.kinds(&ant), ["anthropic-key"]);
        assert!(matches!(scanner.redact("clean"), Cow::Borrowed("clean")));
    }
}
