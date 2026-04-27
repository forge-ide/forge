//! Environment-variable fallback for credential lookup.
//!
//! Read-only by design. `set` and `remove` are no-ops on this store; the
//! shell environment is not a writable destination — credential persistence
//! goes through the OS keyring. Compose with [`super::LayeredStore`] so the
//! keyring is consulted first and env vars catch the bootstrap case
//! (a freshly cloned dev box that exports `ANTHROPIC_API_KEY` in `.envrc`).

use async_trait::async_trait;
use secrecy::SecretString;
use std::collections::HashMap;

use super::Credentials;
use crate::ForgeError;

/// Map of `provider_id` → environment variable name.
///
/// `EnvFallbackStore::default()` wires the two providers Phase 3 ships with:
/// `anthropic` → `ANTHROPIC_API_KEY`, `openai` → `OPENAI_API_KEY`.
pub const ANTHROPIC_ENV_VAR: &str = "ANTHROPIC_API_KEY";
pub const OPENAI_ENV_VAR: &str = "OPENAI_API_KEY";

/// Function shape used to read environment variables. Boxed so callers
/// can inject a hermetic in-memory map under test without mutating
/// `std::env` (which is process-wide and racy across parallel tests).
type EnvReader = Box<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Reads credentials from process environment variables.
///
/// The `provider_id → env-var-name` mapping is held as configuration so
/// downstream crates (e.g. third-party providers) can extend it without
/// patching `forge-core`. Construction with [`EnvFallbackStore::default`]
/// gives the canonical Phase 3 mapping.
pub struct EnvFallbackStore {
    /// Captured at construction so tests can inject a hermetic env without
    /// mutating `std::env` (which is process-wide and racy).
    reader: EnvReader,
    mapping: HashMap<String, String>,
}

impl std::fmt::Debug for EnvFallbackStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvFallbackStore")
            .field("mapping", &self.mapping)
            .finish_non_exhaustive()
    }
}

impl EnvFallbackStore {
    /// Build a store that reads from `std::env::var` with the given
    /// `provider_id → env-var-name` map.
    pub fn with_mapping(mapping: HashMap<String, String>) -> Self {
        Self {
            reader: Box::new(|name| std::env::var(name).ok()),
            mapping,
        }
    }

    /// Test-only constructor. Lets the caller inject an explicit reader so
    /// the test does not have to mutate the process environment.
    #[doc(hidden)]
    pub fn with_reader<F>(mapping: HashMap<String, String>, reader: F) -> Self
    where
        F: Fn(&str) -> Option<String> + Send + Sync + 'static,
    {
        Self {
            reader: Box::new(reader),
            mapping,
        }
    }

    fn env_name(&self, provider_id: &str) -> Option<&str> {
        self.mapping.get(provider_id).map(String::as_str)
    }
}

impl Default for EnvFallbackStore {
    fn default() -> Self {
        let mut m = HashMap::new();
        m.insert("anthropic".to_string(), ANTHROPIC_ENV_VAR.to_string());
        m.insert("openai".to_string(), OPENAI_ENV_VAR.to_string());
        Self::with_mapping(m)
    }
}

#[async_trait]
impl Credentials for EnvFallbackStore {
    async fn get(&self, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        let Some(env_name) = self.env_name(provider_id) else {
            return Ok(None);
        };
        Ok((self.reader)(env_name)
            .filter(|v| !v.is_empty())
            .map(SecretString::from))
    }

    async fn set(&self, _provider_id: &str, _value: SecretString) -> Result<(), ForgeError> {
        // Read-only fallback. Compose with a writable store via `LayeredStore`
        // for any path that needs to persist credentials.
        Err(anyhow::anyhow!("EnvFallbackStore is read-only").into())
    }

    async fn remove(&self, _provider_id: &str) -> Result<(), ForgeError> {
        Err(anyhow::anyhow!("EnvFallbackStore is read-only").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;
    use std::sync::{Arc, Mutex};

    fn fixed_reader(
        values: HashMap<String, String>,
    ) -> impl Fn(&str) -> Option<String> + Send + Sync {
        let m = Arc::new(Mutex::new(values));
        move |name| m.lock().unwrap().get(name).cloned()
    }

    fn default_mapping() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("anthropic".to_string(), ANTHROPIC_ENV_VAR.to_string());
        m.insert("openai".to_string(), OPENAI_ENV_VAR.to_string());
        m
    }

    #[tokio::test]
    async fn reads_anthropic_key_from_env() {
        let mut env = HashMap::new();
        env.insert(ANTHROPIC_ENV_VAR.to_string(), "sk-ant-1".to_string());
        let store = EnvFallbackStore::with_reader(default_mapping(), fixed_reader(env));

        let got = store.get("anthropic").await.unwrap().expect("entry");
        assert_eq!(got.expose_secret(), "sk-ant-1");
    }

    #[tokio::test]
    async fn reads_openai_key_from_env() {
        let mut env = HashMap::new();
        env.insert(OPENAI_ENV_VAR.to_string(), "sk-oai-1".to_string());
        let store = EnvFallbackStore::with_reader(default_mapping(), fixed_reader(env));

        let got = store.get("openai").await.unwrap().expect("entry");
        assert_eq!(got.expose_secret(), "sk-oai-1");
    }

    #[tokio::test]
    async fn unset_env_returns_none() {
        let store = EnvFallbackStore::with_reader(default_mapping(), fixed_reader(HashMap::new()));
        assert!(store.get("anthropic").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn empty_env_returns_none() {
        let mut env = HashMap::new();
        env.insert(ANTHROPIC_ENV_VAR.to_string(), String::new());
        let store = EnvFallbackStore::with_reader(default_mapping(), fixed_reader(env));
        assert!(store.get("anthropic").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unmapped_provider_returns_none_not_error() {
        let store = EnvFallbackStore::with_reader(default_mapping(), fixed_reader(HashMap::new()));
        assert!(store.get("unknown-provider").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_is_rejected() {
        let store = EnvFallbackStore::default();
        let err = store
            .set("anthropic", SecretString::from("nope"))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("read-only"));
    }

    #[tokio::test]
    async fn remove_is_rejected() {
        let store = EnvFallbackStore::default();
        let err = store.remove("anthropic").await.unwrap_err();
        assert!(err.to_string().contains("read-only"));
    }
}
