//! Per-provider credential storage.
//!
//! F-587 establishes a single trait, [`Credentials`], for fetching, storing,
//! and removing API keys keyed by `provider_id` (e.g. `"anthropic"`,
//! `"openai"`). The orchestrator pulls the credential for the active provider
//! at the start of each turn so a key rotation lands without a process
//! restart.
//!
//! # Implementations
//!
//! - [`MemoryStore`] — in-process map. Test seam.
//! - [`EnvFallbackStore`] — read-only view over `ANTHROPIC_API_KEY` /
//!   `OPENAI_API_KEY`, used at startup when no keyring entry exists.
//! - [`LayeredStore`] — composes a primary store with a fallback. Production
//!   wiring is `LayeredStore::new(KeyringStore::new()?, EnvFallbackStore)`.
//! - [`KeyringStore`] — platform-native backend, gated by `cfg(target_os)`.
//!   Linux uses the Secret Service (`secret-service`), macOS uses the
//!   Keychain (`security-framework`), Windows uses DPAPI / Credential
//!   Manager (`keyring`).
//!
//! # Security guarantees
//!
//! Every API boundary uses [`secrecy::SecretString`]; the backing bytes are
//! zeroed on drop and the `Debug` impl prints `[REDACTED alloc::string::String]`
//! rather than the value. **Do not log credential values, even at
//! `tracing::debug` level.** `tracing::trace` is fine for non-secret context
//! (provider id, store type, hit/miss).
//!
//! Network egress paths (e.g. Anthropic / OpenAI provider auth headers)
//! should call [`secrecy::ExposeSecret::expose_secret`] at the last possible
//! moment and avoid copying the result into longer-lived `String` values.

use async_trait::async_trait;
use secrecy::SecretString;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ForgeError;

pub mod env;
pub mod layered;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub mod keyring;

pub use env::EnvFallbackStore;
pub use layered::LayeredStore;

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub use keyring::KeyringStore;

/// Per-provider credential store.
///
/// Implementations must:
///
/// 1. Never log, expose, or `Debug`-print the underlying secret bytes.
/// 2. Treat `provider_id` as a stable, lowercase ASCII slug (`"anthropic"`,
///    `"openai"`). Implementations may namespace freely (e.g. by prefixing
///    `"forge."` in the keyring service field), but the identifier the
///    caller hands in is the source of truth.
/// 3. Return `Ok(None)` for "no entry", reserving `Err(_)` for backend
///    failure. Callers (notably [`LayeredStore`]) rely on this to fall
///    through cleanly.
#[async_trait]
pub trait Credentials: Send + Sync {
    /// Fetch the credential for `provider_id`, or `Ok(None)` if no entry
    /// is stored.
    async fn get(&self, provider_id: &str) -> Result<Option<SecretString>, ForgeError>;

    /// Persist `value` under `provider_id`, replacing any existing entry.
    async fn set(&self, provider_id: &str, value: SecretString) -> Result<(), ForgeError>;

    /// Remove the entry for `provider_id`. Idempotent — removing a missing
    /// entry must not error.
    async fn remove(&self, provider_id: &str) -> Result<(), ForgeError>;

    /// Convenience predicate. Default impl maps `get` to `is_some`; backends
    /// that can answer presence without materializing the secret may
    /// override.
    async fn has(&self, provider_id: &str) -> Result<bool, ForgeError> {
        Ok(self.get(provider_id).await?.is_some())
    }
}

/// Blanket impl for `Arc<dyn Credentials>` so call sites can hold a
/// trait-object behind a refcounted handle without re-wrapping.
#[async_trait]
impl<T: Credentials + ?Sized> Credentials for Arc<T> {
    async fn get(&self, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        (**self).get(provider_id).await
    }

    async fn set(&self, provider_id: &str, value: SecretString) -> Result<(), ForgeError> {
        (**self).set(provider_id, value).await
    }

    async fn remove(&self, provider_id: &str) -> Result<(), ForgeError> {
        (**self).remove(provider_id).await
    }

    async fn has(&self, provider_id: &str) -> Result<bool, ForgeError> {
        (**self).has(provider_id).await
    }
}

/// In-process credential store.
///
/// Useful for tests and as a no-OS-keyring fallback in headless contexts
/// (e.g. `forge-cli` over SSH where the Secret Service daemon is not
/// running). Values vanish with the process — by design.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<HashMap<String, SecretString>>,
}

// Manual `Debug`: never derive on a struct holding `SecretString`.
// `secrecy` already redacts the value in its own `Debug` impl, but the
// project rule is that the credential-holding container itself
// `finish_non_exhaustive`s — same shape used for [`CredentialContext`]
// in `forge-session::orchestrator` and [`KeyringStore`].
impl std::fmt::Debug for MemoryStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryStore").finish_non_exhaustive()
    }
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl Credentials for MemoryStore {
    async fn get(&self, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        let guard = self.inner.lock().await;
        Ok(guard.get(provider_id).cloned())
    }

    async fn set(&self, provider_id: &str, value: SecretString) -> Result<(), ForgeError> {
        let mut guard = self.inner.lock().await;
        guard.insert(provider_id.to_string(), value);
        Ok(())
    }

    async fn remove(&self, provider_id: &str) -> Result<(), ForgeError> {
        let mut guard = self.inner.lock().await;
        guard.remove(provider_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[tokio::test]
    async fn memory_store_round_trip() {
        let store = MemoryStore::new();
        assert!(store.get("anthropic").await.unwrap().is_none());
        assert!(!store.has("anthropic").await.unwrap());

        store
            .set("anthropic", SecretString::from("sk-test-1"))
            .await
            .unwrap();

        let got = store.get("anthropic").await.unwrap().expect("entry");
        assert_eq!(got.expose_secret(), "sk-test-1");
        assert!(store.has("anthropic").await.unwrap());
    }

    #[tokio::test]
    async fn memory_store_overwrites() {
        let store = MemoryStore::new();
        store.set("openai", SecretString::from("v1")).await.unwrap();
        store.set("openai", SecretString::from("v2")).await.unwrap();
        let got = store.get("openai").await.unwrap().unwrap();
        assert_eq!(got.expose_secret(), "v2");
    }

    #[tokio::test]
    async fn memory_store_remove_idempotent() {
        let store = MemoryStore::new();
        store.remove("missing").await.unwrap();
        store
            .set("anthropic", SecretString::from("k"))
            .await
            .unwrap();
        store.remove("anthropic").await.unwrap();
        store.remove("anthropic").await.unwrap();
        assert!(store.get("anthropic").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn arc_dyn_credentials_dispatches() {
        let store: Arc<dyn Credentials> = Arc::new(MemoryStore::new());
        store
            .set("anthropic", SecretString::from("k"))
            .await
            .unwrap();
        assert_eq!(
            store
                .get("anthropic")
                .await
                .unwrap()
                .unwrap()
                .expose_secret(),
            "k"
        );
    }

    /// `SecretString::Debug` must redact, not print the value. This pins
    /// the `secrecy::SecretBox` contract we depend on.
    #[test]
    fn secret_string_debug_does_not_leak() {
        let s = SecretString::from("super-secret-value");
        let dbg = format!("{s:?}");
        assert!(!dbg.contains("super-secret-value"), "Debug leaked: {dbg}");
    }
}
