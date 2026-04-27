//! Two-tier credential store: a writable primary, with a read-only fallback.
//!
//! Production wiring is `LayeredStore::new(KeyringStore, EnvFallbackStore)`.
//! `get` consults the keyring first; if absent, reads from process env.
//! `set` and `remove` always target the primary — env-vars are read-only.

use async_trait::async_trait;
use secrecy::SecretString;

use super::Credentials;
use crate::ForgeError;

/// Compose a writable primary store with a read-only fallback.
///
/// Generic so the type system pins each layer at construction. Call sites
/// that hold a `LayeredStore<KeyringStore, EnvFallbackStore>` can downcast
/// neither layer — it's all behind [`Credentials`] from outside.
pub struct LayeredStore<P, F> {
    primary: P,
    fallback: F,
}

impl<P, F> LayeredStore<P, F> {
    pub fn new(primary: P, fallback: F) -> Self {
        Self { primary, fallback }
    }
}

#[async_trait]
impl<P, F> Credentials for LayeredStore<P, F>
where
    P: Credentials,
    F: Credentials,
{
    async fn get(&self, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        if let Some(v) = self.primary.get(provider_id).await? {
            return Ok(Some(v));
        }
        self.fallback.get(provider_id).await
    }

    async fn set(&self, provider_id: &str, value: SecretString) -> Result<(), ForgeError> {
        // Writes only ever target the primary. The fallback is read-only by
        // contract; a `set` against it would either silently drop or error.
        self.primary.set(provider_id, value).await
    }

    async fn remove(&self, provider_id: &str) -> Result<(), ForgeError> {
        self.primary.remove(provider_id).await
    }

    async fn has(&self, provider_id: &str) -> Result<bool, ForgeError> {
        if self.primary.has(provider_id).await? {
            return Ok(true);
        }
        self.fallback.has(provider_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::credentials::{EnvFallbackStore, MemoryStore};
    use secrecy::ExposeSecret;
    use std::collections::HashMap;

    fn env_with(values: HashMap<String, String>) -> EnvFallbackStore {
        let mut mapping = HashMap::new();
        mapping.insert(
            "anthropic".to_string(),
            crate::credentials::env::ANTHROPIC_ENV_VAR.to_string(),
        );
        EnvFallbackStore::with_reader(mapping, move |name| values.get(name).cloned())
    }

    #[tokio::test]
    async fn primary_hit_short_circuits_fallback() {
        let primary = MemoryStore::new();
        primary
            .set("anthropic", SecretString::from("from-keyring"))
            .await
            .unwrap();

        let mut env = HashMap::new();
        env.insert(
            crate::credentials::env::ANTHROPIC_ENV_VAR.to_string(),
            "from-env".to_string(),
        );
        let layered = LayeredStore::new(primary, env_with(env));

        let got = layered.get("anthropic").await.unwrap().unwrap();
        assert_eq!(got.expose_secret(), "from-keyring");
    }

    #[tokio::test]
    async fn primary_miss_falls_through_to_env() {
        let primary = MemoryStore::new();
        let mut env = HashMap::new();
        env.insert(
            crate::credentials::env::ANTHROPIC_ENV_VAR.to_string(),
            "from-env".to_string(),
        );
        let layered = LayeredStore::new(primary, env_with(env));

        let got = layered.get("anthropic").await.unwrap().unwrap();
        assert_eq!(got.expose_secret(), "from-env");
    }

    #[tokio::test]
    async fn both_miss_returns_none() {
        let layered = LayeredStore::new(MemoryStore::new(), env_with(HashMap::new()));
        assert!(layered.get("anthropic").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn set_writes_to_primary_only() {
        let primary = MemoryStore::new();
        let layered = LayeredStore::new(primary, env_with(HashMap::new()));

        layered
            .set("anthropic", SecretString::from("k"))
            .await
            .unwrap();

        let got = layered.primary.get("anthropic").await.unwrap().unwrap();
        assert_eq!(got.expose_secret(), "k");
    }

    #[tokio::test]
    async fn has_short_circuits_on_primary_hit() {
        let primary = MemoryStore::new();
        primary
            .set("anthropic", SecretString::from("k"))
            .await
            .unwrap();
        let layered = LayeredStore::new(primary, env_with(HashMap::new()));
        assert!(layered.has("anthropic").await.unwrap());
    }

    #[tokio::test]
    async fn has_falls_through_to_env() {
        let primary = MemoryStore::new();
        let mut env = HashMap::new();
        env.insert(
            crate::credentials::env::ANTHROPIC_ENV_VAR.to_string(),
            "k".to_string(),
        );
        let layered = LayeredStore::new(primary, env_with(env));
        assert!(layered.has("anthropic").await.unwrap());
    }
}
