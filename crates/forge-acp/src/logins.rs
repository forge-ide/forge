//! Agents' own subscription sign-ins (claude.ai, ChatGPT), used when the keyring has no API key
//! for the agent.
//!
//! Forge reads a login where the agent's CLI keeps it, in the user's real home, and never
//! writes it. The sandbox gets a placeholder; the egress proxy sends the real access token,
//! read fresh for each request, to the login's one host and paths (see
//! [`forge_sandbox::secrets::KNOWN`]). Refresh tokens never leave the file, so refreshing an
//! expired login stays with the agent's own CLI; a request made with an expired one is refused
//! with that advice.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use forge_sandbox::proxy::SecretRoute;
use forge_sandbox::secrets::{self, KnownSecret, Secret, SecretSource};
use serde::Deserialize;
use serde_json::Value;

/// The largest login file Forge reads.
const MAX_FILE: u64 = 1024 * 1024;

/// How long a value read from a login file is reused before the file is read again.
const REREAD: Duration = Duration::from_secs(30);

/// Which sign-in an agent's CLI keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LoginKind {
    /// Claude Code's claude.ai sign-in: `~/.claude/.credentials.json` (on macOS, the login
    /// keychain item `Claude Code-credentials`, falling back to the file).
    ClaudeAi,
    /// Codex's ChatGPT sign-in: `~/.codex/auth.json`.
    Chatgpt,
}

impl LoginKind {
    /// What the user calls it.
    pub fn label(self) -> &'static str {
        match self {
            LoginKind::ClaudeAi => "claude.ai",
            LoginKind::Chatgpt => "ChatGPT",
        }
    }

    /// The command that signs in, or refreshes an expired sign-in.
    pub fn sign_in_command(self) -> &'static str {
        match self {
            LoginKind::ClaudeAi => "claude",
            LoginKind::Chatgpt => "codex login",
        }
    }

    /// The secret the proxy sends the login's token as.
    pub fn secret(self) -> &'static KnownSecret {
        let name = match self {
            LoginKind::ClaudeAi => "CLAUDE_CODE_OAUTH_TOKEN",
            LoginKind::Chatgpt => "CODEX_CHATGPT_TOKEN",
        };
        secrets::known(name).expect("logins are known secrets")
    }
}

/// What Codex needs besides the token to use a ChatGPT sign-in: none of it is a credential.
#[derive(Clone, Debug, PartialEq)]
pub struct ChatgptAccount {
    pub account_id: String,
    /// The claims of the sign-in's ID token, which Codex reads the plan and email from.
    pub id_claims: Value,
    /// The claims of its access token.
    pub access_claims: Value,
}

/// A login found in the user's home.
#[derive(Clone)]
pub struct Login {
    pub kind: LoginKind,
    source: Arc<LoginSource>,
    /// For a ChatGPT login.
    pub account: Option<ChatgptAccount>,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login")
            .field("kind", &self.kind)
            .field("path", &self.source.path)
            .finish_non_exhaustive()
    }
}

impl Login {
    /// The proxy route that sends the login's token.
    pub fn route(&self) -> SecretRoute {
        SecretRoute {
            secret: self.kind.secret(),
            value: self.source.clone(),
        }
    }

    /// The token as it is now, for the log redactor; an error says why it can't be used.
    pub fn current(&self) -> Result<Secret, String> {
        self.source.current()
    }
}

/// Where `kind`'s CLI keeps its login under `home` (the user's real home dir).
pub fn path(kind: LoginKind, home: &Path) -> PathBuf {
    match kind {
        LoginKind::ClaudeAi => home.join(".claude/.credentials.json"),
        LoginKind::Chatgpt => home.join(".codex/auth.json"),
    }
}

/// The user's `kind` login under `home`, if they have one; an error for one that is there but
/// can't be used now (expired, malformed). Blocking.
pub fn find(kind: LoginKind, home: &Path) -> Result<Option<Login>, String> {
    let source = Arc::new(LoginSource {
        kind,
        path: path(kind, home),
        cache: Mutex::new(None),
    });
    let Some(raw) = source.read()? else {
        return Ok(None);
    };
    let parsed = parse(kind, &raw)?;
    let Some(parsed) = parsed else {
        return Ok(None);
    };
    let account = parsed.account.clone();
    source.check(&parsed)?;
    *lock(&source.cache) = Some((Instant::now(), parsed));
    Ok(Some(Login {
        kind,
        source,
        account,
    }))
}

/// Reads a login's file for each request, reusing what it read for a short while.
struct LoginSource {
    kind: LoginKind,
    path: PathBuf,
    cache: Mutex<Option<(Instant, Parsed)>>,
}

impl SecretSource for LoginSource {
    fn current(&self) -> Result<Secret, String> {
        let cached = lock(&self.cache)
            .as_ref()
            .filter(|(at, _)| at.elapsed() < REREAD)
            .map(|(_, parsed)| parsed.clone());
        let parsed = match cached {
            Some(parsed) => parsed,
            None => {
                let raw = self.read()?.ok_or_else(|| {
                    format!(
                        "the {} sign-in is gone; sign in again with `{}`",
                        self.kind.label(),
                        self.kind.sign_in_command()
                    )
                })?;
                let parsed = parse(self.kind, &raw)?.ok_or_else(|| {
                    format!(
                        "the {} sign-in was replaced by another kind of sign-in",
                        self.kind.label()
                    )
                })?;
                *lock(&self.cache) = Some((Instant::now(), parsed.clone()));
                parsed
            }
        };
        self.check(&parsed)?;
        Ok(Secret::new(parsed.token))
    }
}

impl LoginSource {
    /// The login's raw contents; `None` when there is none.
    fn read(&self) -> Result<Option<Vec<u8>>, String> {
        #[cfg(target_os = "macos")]
        if self.kind == LoginKind::ClaudeAi
            && let Some(found) = macos::claude_keychain()?
        {
            return Ok(Some(found));
        }
        read_bounded(&self.path)
    }

    /// Refuse a token that has expired: the agent's own CLI refreshes it.
    fn check(&self, parsed: &Parsed) -> Result<(), String> {
        match parsed.expires_at {
            Some(at) if at <= SystemTime::now() => Err(format!(
                "the {} sign-in has expired; run `{}` once outside Forge to refresh it, then retry",
                self.kind.label(),
                self.kind.sign_in_command()
            )),
            _ => Ok(()),
        }
    }
}

#[derive(Clone)]
struct Parsed {
    token: String,
    expires_at: Option<SystemTime>,
    account: Option<ChatgptAccount>,
}

fn parse(kind: LoginKind, raw: &[u8]) -> Result<Option<Parsed>, String> {
    let malformed = |why: &str| format!("the {} sign-in can't be read: {why}", kind.label());
    let json: Value = serde_json::from_slice(raw).map_err(|_| malformed("not JSON"))?;
    match kind {
        LoginKind::ClaudeAi => {
            let Some(oauth) = json.get("claudeAiOauth") else {
                return Ok(None);
            };
            let token = oauth["accessToken"]
                .as_str()
                .filter(|t| !t.is_empty())
                .ok_or_else(|| malformed("no access token"))?;
            let expires_at = oauth["expiresAt"]
                .as_u64()
                .map(|ms| UNIX_EPOCH + Duration::from_millis(ms));
            Ok(Some(Parsed {
                token: token.to_owned(),
                expires_at,
                account: None,
            }))
        }
        LoginKind::Chatgpt => {
            // An API key kept by Codex is not a login Forge carries.
            if json["auth_mode"].as_str() != Some("chatgpt") {
                return Ok(None);
            }
            let tokens = &json["tokens"];
            let field = |name: &str| {
                tokens[name]
                    .as_str()
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| malformed(&format!("no {name}")))
            };
            let token = field("access_token")?;
            let account_id = field("account_id")?;
            let access_claims = claims(token).ok_or_else(|| malformed("odd access token"))?;
            let id_claims = claims(field("id_token")?).ok_or_else(|| malformed("odd ID token"))?;
            let expires_at = access_claims["exp"]
                .as_u64()
                .map(|s| UNIX_EPOCH + Duration::from_secs(s));
            Ok(Some(Parsed {
                token: token.to_owned(),
                expires_at,
                account: Some(ChatgptAccount {
                    account_id: account_id.to_owned(),
                    id_claims,
                    access_claims,
                }),
            }))
        }
    }
}

/// A JWT's claims, unverified: Codex reads them, and Forge only passes them on.
fn claims(jwt: &str) -> Option<Value> {
    let payload = jwt.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .filter(Value::is_object)
}

/// An unsigned JWT carrying `claims`, which works only where nothing checks the signature: in
/// Codex, which reads its tokens' claims but leaves checking them to the server.
pub fn unsigned_jwt(claims: &Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = URL_SAFE_NO_PAD.encode(claims.to_string());
    format!("{header}.{payload}.{}", secrets::PLACEHOLDER)
}

/// A regular file's contents, if it exists and isn't too large. A symlink is read through: the
/// file is in the user's own home, outside any sandbox.
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, String> {
    use std::io::Read;
    // Checked before opening too, so a FIFO there can't block the open.
    match std::fs::metadata(path) {
        Ok(meta) if !meta.is_file() => {
            return Err(format!("{} is not a regular file", path.display()));
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => {}
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let meta = file
        .metadata()
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > MAX_FILE {
        return Err(format!(
            "{} is not a regular file under 1 MiB",
            path.display()
        ));
    }
    let mut out = Vec::new();
    file.take(MAX_FILE)
        .read_to_end(&mut out)
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(Some(out))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(target_os = "macos")]
mod macos {
    use std::process::{Command, Stdio};

    /// Claude Code's keychain item, if there is one. The system's own tool, or for tests the
    /// one `FORGE_KEYRING_TOOL` names, as for Forge's own secrets.
    pub fn claude_keychain() -> Result<Option<Vec<u8>>, String> {
        let program = std::env::var_os("FORGE_KEYRING_TOOL")
            .filter(|p| !p.is_empty())
            .unwrap_or_else(|| "/usr/bin/security".into());
        let output = Command::new(&program)
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("running {}: {e}", program.to_string_lossy()))?;
        match output.status.code() {
            Some(0) => Ok(Some(output.stdout)),
            // errSecItemNotFound: the file, if any, holds the login.
            Some(44) => Ok(None),
            _ => Err(
                "the claude.ai sign-in in the login keychain can't be read; make sure the keychain is unlocked"
                    .into(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn jwt(claims: Value) -> String {
        let body = URL_SAFE_NO_PAD.encode(claims.to_string());
        format!("eyJhbGciOiJSUzI1NiJ9.{body}.c2ln")
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn write(home: &Path, rel: &str, contents: &str) {
        let path = home.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }

    // On macOS, Claude's login is in the keychain, which these tests don't fake.
    #[cfg(not(target_os = "macos"))]
    fn claude(token: &str, expires_ms: u64) -> String {
        json!({
            "claudeAiOauth": {"accessToken": token, "refreshToken": "refresh-secret", "expiresAt": expires_ms},
            "mcpOAuth": {"plugin:x": {"accessToken": "other-service"}},
        })
        .to_string()
    }

    fn codex(access: &str, id: &str) -> String {
        json!({
            "auth_mode": "chatgpt",
            "OPENAI_API_KEY": null,
            "tokens": {"id_token": id, "access_token": access, "refresh_token": "refresh-secret", "account_id": "acct-1"},
            "last_refresh": "2026-10-24T00:00:00Z",
        })
        .to_string()
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_claude_login_sends_only_its_access_token() {
        let home = tempfile::tempdir().unwrap();
        assert!(find(LoginKind::ClaudeAi, home.path()).unwrap().is_none());
        let later = (now_secs() + 3600) * 1000;
        write(
            home.path(),
            ".claude/.credentials.json",
            &claude("at-1", later),
        );
        let login = find(LoginKind::ClaudeAi, home.path()).unwrap().unwrap();
        assert_eq!(login.current().unwrap().expose(), "at-1");
        assert_eq!(login.route().secret.name, "CLAUDE_CODE_OAUTH_TOKEN");
        assert!(login.account.is_none());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn an_expired_claude_login_says_how_to_refresh_it() {
        let home = tempfile::tempdir().unwrap();
        let earlier = (now_secs() - 60) * 1000;
        write(
            home.path(),
            ".claude/.credentials.json",
            &claude("at-1", earlier),
        );
        let err = find(LoginKind::ClaudeAi, home.path()).unwrap_err();
        assert!(err.contains("expired") && err.contains("`claude`"), "{err}");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn a_login_is_read_again_once_the_cache_is_stale() {
        let home = tempfile::tempdir().unwrap();
        let later = (now_secs() + 3600) * 1000;
        write(
            home.path(),
            ".claude/.credentials.json",
            &claude("at-1", later),
        );
        let login = find(LoginKind::ClaudeAi, home.path()).unwrap().unwrap();
        // The CLI refreshed it.
        write(
            home.path(),
            ".claude/.credentials.json",
            &claude("at-2", later),
        );
        assert_eq!(login.current().unwrap().expose(), "at-1");
        *lock(&login.source.cache) = None;
        assert_eq!(login.current().unwrap().expose(), "at-2");
        std::fs::remove_file(home.path().join(".claude/.credentials.json")).unwrap();
        *lock(&login.source.cache) = None;
        let err = login.current().unwrap_err();
        assert!(err.contains("sign in again"), "{err}");
    }

    #[test]
    fn a_chatgpt_login_carries_its_account_but_no_refresh_token() {
        let home = tempfile::tempdir().unwrap();
        let exp = now_secs() + 3600;
        let access =
            jwt(json!({"exp": exp, "https://api.openai.com/auth": {"chatgpt_plan_type": "pro"}}));
        let id = jwt(json!({"email": "u@example.com", "exp": exp}));
        write(home.path(), ".codex/auth.json", &codex(&access, &id));
        let login = find(LoginKind::Chatgpt, home.path()).unwrap().unwrap();
        assert_eq!(login.current().unwrap().expose(), access);
        let account = login.account.unwrap();
        assert_eq!(account.account_id, "acct-1");
        assert_eq!(account.id_claims["email"], "u@example.com");
        assert_eq!(account.access_claims["exp"], exp);
        assert!(!format!("{account:?}").contains("refresh-secret"));
    }

    #[test]
    fn codex_api_keys_and_odd_files_are_not_logins() {
        let home = tempfile::tempdir().unwrap();
        write(
            home.path(),
            ".codex/auth.json",
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"sk-x"}"#,
        );
        assert!(find(LoginKind::Chatgpt, home.path()).unwrap().is_none());
        write(home.path(), ".codex/auth.json", "not json");
        assert!(find(LoginKind::Chatgpt, home.path()).is_err());
        let expired = jwt(json!({"exp": now_secs() - 10}));
        write(home.path(), ".codex/auth.json", &codex(&expired, &expired));
        let err = find(LoginKind::Chatgpt, home.path()).unwrap_err();
        assert!(err.contains("`codex login`"), "{err}");
    }

    #[test]
    fn unsigned_jwts_carry_their_claims_and_no_token() {
        let made = unsigned_jwt(&json!({"exp": 5, "email": "u@example.com"}));
        assert_eq!(claims(&made).unwrap()["email"], "u@example.com");
        assert!(made.ends_with(secrets::PLACEHOLDER));
    }
}
