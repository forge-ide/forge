//! Secrets the egress proxy injects, and where they come from.
//!
//! A secret never enters a sandbox. For each secret a profile names, the agent gets a
//! placeholder value and a base URL pointing at a route on the session's egress proxy; the
//! proxy forwards requests on that route over TLS to the secret's fixed upstream, replacing
//! the placeholder header with the real value. A placeholder sent anywhere else is worthless.
//!
//! Values come from the OS keyring: service `forge`, account the secret's name. Linux reads
//! them with `secret-tool` (libsecret), macOS with `security` (the login keychain). A login
//! secret (an agent's own subscription sign-in) is read instead from where the agent's CLI
//! keeps it, fresh for each request, through a [`SecretSource`].

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;

/// What agents see in place of a secret's value.
pub const PLACEHOLDER: &str = "forge-proxy-injected";

/// The keyring service Forge's secrets are stored under.
pub const SERVICE: &str = "forge";

/// How a secret travels in a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Header {
    /// `<name>: <value>`.
    Named(&'static str),
    /// `Authorization: Bearer <value>`.
    Bearer,
}

impl Header {
    /// The header's name, lowercase.
    pub fn name(self) -> &'static str {
        match self {
            Header::Named(name) => name,
            Header::Bearer => "authorization",
        }
    }

    /// The header line carrying `value`.
    pub(crate) fn line(self, value: &str) -> String {
        match self {
            Header::Named(name) => format!("{name}: {value}\r\n"),
            Header::Bearer => format!("Authorization: Bearer {value}\r\n"),
        }
    }
}

/// A secret Forge knows how to inject.
#[derive(Debug, PartialEq, Eq)]
pub struct KnownSecret {
    /// The name profiles use in `env_secrets`, also the placeholder's variable.
    pub name: &'static str,
    /// The proxy route: requests to `/_forge/<route>/...` go to `host`.
    pub route: &'static str,
    /// The only host the secret is sent to, over TLS on port 443.
    pub host: &'static str,
    pub header: Header,
    /// The variable that points the agent's client at the route.
    pub base_url_var: &'static str,
    /// Appended to the route in the base URL, matching the client's default base URL.
    pub base_url_suffix: &'static str,
    /// The only paths a request carrying the secret may use: these, or anything below them.
    /// Inference and model listing only, so a key with wider scope can't be used to manage
    /// the account (minting new keys, for one).
    pub paths: &'static [&'static str],
}

impl KnownSecret {
    /// Whether a request for `path` (with any query) may carry this secret. Paths that a
    /// server might normalize into another one (`..`, `%` escapes, backslashes, empty
    /// segments) are refused outright.
    pub fn allows_path(&self, path: &str) -> bool {
        let path = path.split(['?', '#']).next().unwrap_or_default();
        let suspicious =
            path.contains("..") || path.contains('%') || path.contains('\\') || path.contains("//");
        !suspicious
            && self.paths.iter().any(|allowed| {
                path == *allowed
                    || path
                        .strip_prefix(allowed)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
    }
}

/// Every secret Forge knows how to inject.
pub const KNOWN: &[KnownSecret] = &[
    KnownSecret {
        name: "ANTHROPIC_API_KEY",
        route: "anthropic",
        host: "api.anthropic.com",
        header: Header::Named("x-api-key"),
        base_url_var: "ANTHROPIC_BASE_URL",
        base_url_suffix: "",
        paths: &["/v1/messages", "/v1/complete", "/v1/models"],
    },
    KnownSecret {
        name: "OPENAI_API_KEY",
        route: "openai",
        host: "api.openai.com",
        header: Header::Bearer,
        base_url_var: "OPENAI_BASE_URL",
        base_url_suffix: "/v1",
        paths: &[
            "/v1/responses",
            "/v1/chat/completions",
            "/v1/completions",
            "/v1/embeddings",
            "/v1/models",
        ],
    },
    KnownSecret {
        name: "GEMINI_API_KEY",
        route: "gemini",
        host: "generativelanguage.googleapis.com",
        header: Header::Named("x-goog-api-key"),
        base_url_var: "GOOGLE_GEMINI_BASE_URL",
        base_url_suffix: "",
        paths: &["/v1beta/models", "/v1/models"],
    },
    // Logins: an agent's own subscription sign-in, which Forge reads where the agent's CLI
    // keeps it when the keyring has no API key. Claude Code takes the token as
    // CLAUDE_CODE_OAUTH_TOKEN (a `claude setup-token` token can be stored in the keyring under
    // that name too).
    KnownSecret {
        name: "CLAUDE_CODE_OAUTH_TOKEN",
        route: "anthropic-login",
        host: "api.anthropic.com",
        header: Header::Bearer,
        base_url_var: "ANTHROPIC_BASE_URL",
        base_url_suffix: "",
        paths: &["/v1/messages", "/v1/models"],
    },
    // Codex reads its ChatGPT sign-in from auth.json, which the shim writes with this
    // placeholder; the variables only carry the placeholder and the route to the shim.
    KnownSecret {
        name: "CODEX_CHATGPT_TOKEN",
        route: "chatgpt",
        host: "chatgpt.com",
        header: Header::Bearer,
        base_url_var: "CODEX_CHATGPT_BASE_URL",
        base_url_suffix: "/backend-api",
        paths: &["/backend-api/codex"],
    },
];

/// The known secret called `name`.
pub fn known(name: &str) -> Option<&'static KnownSecret> {
    KNOWN.iter().find(|s| s.name == name)
}

/// The names Forge can inject, for error messages.
pub fn known_names() -> String {
    KNOWN.iter().map(|s| s.name).collect::<Vec<_>>().join(", ")
}

/// A secret's value. Its `Debug` output never shows the value.
#[derive(Clone)]
pub struct Secret(Arc<str>);

impl Secret {
    pub fn new(value: impl Into<Arc<str>>) -> Self {
        Self(value.into())
    }

    /// The value, for sending it, and for the secret scan that keeps it out of the log.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    #[error("secret {name} is not in the keyring; store it with: {fix}")]
    Missing { name: String, fix: String },
    #[error("the keyring can't be read: {reason}; {fix}")]
    Unavailable { reason: String, fix: String },
    #[error(
        "secret {name} in the keyring contains control characters, so it can't be sent as a header; store it again"
    )]
    Invalid { name: String },
}

/// Where secret values come from. Blocking.
pub trait SecretStore: Send + Sync + 'static {
    fn get(&self, name: &str) -> Result<Secret, SecretError>;
}

/// Where a route's value comes from: read once per request sent on the route, so a value that
/// changes (a login the agent's own CLI refreshes) is current. Blocking; the proxy calls it
/// off the runtime. An error is the reason the request is refused, shown to the agent.
pub trait SecretSource: Send + Sync + 'static {
    fn current(&self) -> Result<Secret, String>;
}

impl SecretSource for Secret {
    fn current(&self) -> Result<Secret, String> {
        Ok(self.clone())
    }
}

/// The OS keyring, through its command-line tool.
#[derive(Clone, Debug)]
pub struct Keyring {
    program: PathBuf,
}

impl Keyring {
    /// The platform's tool, resolved to an absolute path now, so a program planted later on
    /// `PATH` never sees a lookup: `/usr/bin/security` on macOS, and `secret-tool` from
    /// `PATH` on Linux. `FORGE_KEYRING_TOOL` names another tool, for tests.
    pub fn system() -> Self {
        let program = std::env::var_os("FORGE_KEYRING_TOOL")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(platform::program);
        Self { program }
    }

    /// A specific tool, for tests.
    pub fn with_program(program: PathBuf) -> Self {
        Self { program }
    }
}

impl SecretStore for Keyring {
    fn get(&self, name: &str) -> Result<Secret, SecretError> {
        let output = Command::new(&self.program)
            .args(platform::lookup_args(name))
            .stdin(Stdio::null())
            .output()
            .map_err(|e| SecretError::Unavailable {
                reason: format!("running {}: {e}", self.program.display()),
                fix: platform::INSTALL.to_owned(),
            })?;
        let value = String::from_utf8_lossy(&output.stdout);
        let value = value.strip_suffix('\n').unwrap_or(&value);
        if output.status.success() && !value.is_empty() {
            // A line break in the value would inject headers into the request it is sent in.
            if value.chars().any(char::is_control) {
                return Err(SecretError::Invalid {
                    name: name.to_owned(),
                });
            }
            return Ok(Secret::new(value));
        }
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if platform::is_missing(output.status.code(), &stderr) {
            Err(SecretError::Missing {
                name: name.to_owned(),
                fix: platform::store_command(name),
            })
        } else {
            Err(SecretError::Unavailable {
                reason: format!(
                    "{} failed ({}): {stderr}",
                    self.program.display(),
                    output.status
                ),
                fix: platform::UNLOCK.to_owned(),
            })
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::SERVICE;

    pub const INSTALL: &str =
        "install libsecret's secret-tool (for example `sudo apt install libsecret-tools`)";
    pub const UNLOCK: &str = "make sure a Secret Service keyring (such as GNOME Keyring or KWallet) is running and unlocked";

    pub fn program() -> std::path::PathBuf {
        let found = std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join("secret-tool"))
                .find(|p| p.is_file())
        });
        // Not installed: running the bare name fails with the install hint.
        found.unwrap_or_else(|| "secret-tool".into())
    }

    pub fn lookup_args(name: &str) -> Vec<String> {
        ["lookup", "service", SERVICE, "account", name]
            .map(str::to_owned)
            .to_vec()
    }

    /// `secret-tool lookup` exits 1 with nothing on stderr when there is no such item.
    pub fn is_missing(code: Option<i32>, stderr: &str) -> bool {
        code == Some(1) && stderr.is_empty()
    }

    pub fn store_command(name: &str) -> String {
        format!("secret-tool store --label='Forge {name}' service {SERVICE} account {name}")
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::SERVICE;

    pub const INSTALL: &str = "/usr/bin/security should be part of macOS; check that it is on PATH";
    pub const UNLOCK: &str = "make sure the login keychain is unlocked";

    /// Always the system's own tool, never one found on `PATH`.
    pub fn program() -> std::path::PathBuf {
        "/usr/bin/security".into()
    }

    pub fn lookup_args(name: &str) -> Vec<String> {
        ["find-generic-password", "-s", SERVICE, "-a", name, "-w"]
            .map(str::to_owned)
            .to_vec()
    }

    /// `security` exits 44 (errSecItemNotFound) when there is no such item.
    pub fn is_missing(code: Option<i32>, _stderr: &str) -> bool {
        code == Some(44)
    }

    pub fn store_command(name: &str) -> String {
        format!("security add-generic-password -s {SERVICE} -a {name} -w")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_secrets_have_unique_names_and_routes() {
        for (i, a) in KNOWN.iter().enumerate() {
            for b in &KNOWN[i + 1..] {
                assert_ne!(a.name, b.name);
                assert_ne!(a.route, b.route);
            }
        }
        assert_eq!(known("OPENAI_API_KEY").unwrap().host, "api.openai.com");
        assert!(known("AWS_SECRET_ACCESS_KEY").is_none());
    }

    #[test]
    fn logins_go_only_to_inference_paths() {
        let chatgpt = known("CODEX_CHATGPT_TOKEN").unwrap();
        assert_eq!(chatgpt.host, "chatgpt.com");
        assert!(chatgpt.allows_path("/backend-api/codex/responses"));
        assert!(chatgpt.allows_path("/backend-api/codex/models?client_version=1"));
        for bad in [
            "/backend-api/accounts",
            "/backend-api/codexx",
            "/backend-api/me",
            "/backend-api/wham/tasks",
            "/backend-api/wham/accounts/check",
            "/backend-api/ps/mcp",
        ] {
            assert!(!chatgpt.allows_path(bad), "{bad}");
        }
        let claude = known("CLAUDE_CODE_OAUTH_TOKEN").unwrap();
        assert_eq!(claude.header, Header::Bearer);
        assert!(claude.allows_path("/v1/messages"));
        assert!(!claude.allows_path("/api/oauth/claude_cli/create_api_key"));
    }

    #[test]
    fn secrets_go_only_to_inference_paths() {
        let anthropic = known("ANTHROPIC_API_KEY").unwrap();
        for ok in [
            "/v1/messages",
            "/v1/messages/count_tokens",
            "/v1/models?limit=5",
        ] {
            assert!(anthropic.allows_path(ok), "{ok}");
        }
        for bad in [
            "/v1/organizations/api_keys",
            "/v1/messagesX",
            "/v1/messages/../organizations",
            "/v1/messages/%2e%2e/organizations",
            "/v1/messages\\..\\x",
            "/v1/messages//x",
            "/",
            "",
        ] {
            assert!(!anthropic.allows_path(bad), "{bad}");
        }
        let openai = known("OPENAI_API_KEY").unwrap();
        assert!(openai.allows_path("/v1/responses"));
        assert!(!openai.allows_path("/v1/organization/projects/p/service_accounts"));
        let gemini = known("GEMINI_API_KEY").unwrap();
        assert!(gemini.allows_path("/v1beta/models/gemini-pro:generateContent"));
        assert!(!gemini.allows_path("/v1beta/files"));
    }

    #[test]
    fn secrets_never_show_in_debug_output() {
        let secret = Secret::new("sk-very-secret");
        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
        assert_eq!(secret.expose(), "sk-very-secret");
    }

    /// A fake keyring tool: prints `value` for `ANTHROPIC_API_KEY`, and otherwise behaves as
    /// the platform's tool does for a missing item, or fails loudly for `BROKEN`.
    fn fake_keyring(dir: &std::path::Path, value: &str) -> Keyring {
        use std::os::unix::fs::PermissionsExt;
        let missing_exit = if cfg!(target_os = "macos") { 44 } else { 1 };
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n  *ANTHROPIC_API_KEY*) printf '%s\\n' '{value}' ;;\n  *GEMINI_API_KEY*) printf 'line1\\r\\nX-Evil: 1\\n' ;;\n  *BROKEN*) echo 'cannot reach the keyring' >&2; exit 2 ;;\n  *) exit {missing_exit} ;;\nesac\n"
        );
        let path = dir.join("keyring");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        Keyring::with_program(path)
    }

    #[test]
    fn reads_values_and_explains_missing_ones() {
        let dir = tempfile::tempdir().unwrap();
        let keyring = fake_keyring(dir.path(), "sk-test");
        assert_eq!(
            keyring.get("ANTHROPIC_API_KEY").unwrap().expose(),
            "sk-test"
        );

        let missing = keyring.get("OPENAI_API_KEY").unwrap_err();
        let expected = if cfg!(target_os = "macos") {
            "security add-generic-password -s forge -a OPENAI_API_KEY -w"
        } else {
            "secret-tool store --label='Forge OPENAI_API_KEY' service forge account OPENAI_API_KEY"
        };
        assert!(missing.to_string().contains(expected), "{missing}");

        let broken = keyring.get("BROKEN").unwrap_err();
        assert!(
            matches!(broken, SecretError::Unavailable { .. }),
            "{broken}"
        );
        assert!(broken.to_string().contains("cannot reach the keyring"));

        let injected = keyring.get("GEMINI_API_KEY").unwrap_err();
        assert!(
            matches!(injected, SecretError::Invalid { .. }),
            "{injected}"
        );

        let absent = Keyring::with_program(dir.path().join("no-such-tool"));
        let err = absent.get("ANTHROPIC_API_KEY").unwrap_err();
        assert!(matches!(err, SecretError::Unavailable { .. }), "{err}");
    }
}
