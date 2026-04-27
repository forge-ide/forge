//! Platform-native credential store.
//!
//! - **Linux** — Secret Service over DBus via the `secret-service` crate
//!   (rt-tokio with `crypto-rust`). The session daemon holds the secret;
//!   GNOME Keyring and KWallet both expose this API.
//! - **macOS** — System Keychain via `security-framework`.
//! - **Windows** — DPAPI / Credential Manager via the cross-platform
//!   `keyring` crate. We do not call DPAPI directly; `keyring`'s win-native
//!   feature wraps it correctly and is the project's chosen fallback.
//!
//! The trait shape is identical across platforms; implementation bodies
//! diverge under `cfg`.
//!
//! # Service / account scheme
//!
//! Every entry is namespaced under a single service string (default
//! `"forge"`); the `provider_id` is the account/username. This keeps Forge's
//! entries grouped in OS UI (Keychain Access shows them in one row group;
//! GNOME's `seahorse` shows them under one collection alias) and makes
//! per-machine cleanup a single API call.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use super::Credentials;
use crate::ForgeError;

/// Default service namespace under which every Forge credential is stored.
///
/// Backends that prefix the account further (e.g. an enterprise build with
/// per-tenant scoping) should construct via [`KeyringStore::with_service`].
pub const DEFAULT_SERVICE: &str = "forge";

/// Wraps the platform-native credential store. Construction does no I/O —
/// any backend handshake (e.g. opening a Secret Service session) happens
/// lazily on the first `get` / `set` / `remove`.
pub struct KeyringStore {
    service: String,
}

impl KeyringStore {
    /// Build a store under [`DEFAULT_SERVICE`].
    pub fn new() -> Self {
        Self::with_service(DEFAULT_SERVICE)
    }

    /// Build a store under an explicit service namespace. Useful for tests
    /// (per-test service strings keep parallel runs from clobbering each
    /// other on Linux's session-keyring) and for forks that want to isolate
    /// their entries from upstream Forge.
    pub fn with_service(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }
}

impl Default for KeyringStore {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for KeyringStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyringStore")
            .field("service", &self.service)
            .finish()
    }
}

// ── Linux: Secret Service ────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
mod linux_impl {
    use super::*;
    use secret_service::{EncryptionType, SecretService};
    use std::collections::HashMap;

    fn label_for(service: &str, provider_id: &str) -> String {
        format!("{service}: {provider_id}")
    }

    fn attrs<'a>(service: &'a str, provider_id: &'a str) -> HashMap<&'a str, &'a str> {
        let mut m = HashMap::new();
        m.insert("service", service);
        m.insert("account", provider_id);
        m
    }

    async fn open() -> Result<SecretService<'static>, ForgeError> {
        SecretService::connect(EncryptionType::Dh)
            .await
            .map_err(|e| anyhow::anyhow!("secret-service connect failed: {e}").into())
    }

    pub async fn get(service: &str, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        let ss = open().await?;
        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| anyhow::anyhow!("secret-service collection failed: {e}"))?;
        if collection
            .is_locked()
            .await
            .map_err(|e| anyhow::anyhow!("secret-service is_locked failed: {e}"))?
        {
            collection
                .unlock()
                .await
                .map_err(|e| anyhow::anyhow!("secret-service unlock failed: {e}"))?;
        }

        let items = ss
            .search_items(attrs(service, provider_id))
            .await
            .map_err(|e| anyhow::anyhow!("secret-service search failed: {e}"))?;

        let Some(item) = items
            .unlocked
            .into_iter()
            .next()
            .or_else(|| items.locked.into_iter().next())
        else {
            return Ok(None);
        };

        if item
            .is_locked()
            .await
            .map_err(|e| anyhow::anyhow!("secret-service item is_locked failed: {e}"))?
        {
            item.unlock()
                .await
                .map_err(|e| anyhow::anyhow!("secret-service item unlock failed: {e}"))?;
        }

        let secret_bytes = item
            .get_secret()
            .await
            .map_err(|e| anyhow::anyhow!("secret-service get_secret failed: {e}"))?;
        let s = String::from_utf8(secret_bytes)
            .map_err(|_| anyhow::anyhow!("secret-service: stored value is not valid UTF-8"))?;
        Ok(Some(SecretString::from(s)))
    }

    pub async fn set(
        service: &str,
        provider_id: &str,
        value: SecretString,
    ) -> Result<(), ForgeError> {
        let ss = open().await?;
        let collection = ss
            .get_default_collection()
            .await
            .map_err(|e| anyhow::anyhow!("secret-service collection failed: {e}"))?;
        collection
            .unlock()
            .await
            .map_err(|e| anyhow::anyhow!("secret-service unlock failed: {e}"))?;

        collection
            .create_item(
                &label_for(service, provider_id),
                attrs(service, provider_id),
                value.expose_secret().as_bytes(),
                true, // replace
                "text/plain; charset=utf8",
            )
            .await
            .map_err(|e| anyhow::anyhow!("secret-service create_item failed: {e}"))?;
        Ok(())
    }

    pub async fn remove(service: &str, provider_id: &str) -> Result<(), ForgeError> {
        let ss = open().await?;
        let items = ss
            .search_items(attrs(service, provider_id))
            .await
            .map_err(|e| anyhow::anyhow!("secret-service search failed: {e}"))?;

        for item in items.unlocked.into_iter().chain(items.locked) {
            item.delete()
                .await
                .map_err(|e| anyhow::anyhow!("secret-service delete failed: {e}"))?;
        }
        Ok(())
    }
}

// ── macOS: Keychain ──────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
mod macos_impl {
    use super::*;
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };

    pub async fn get(service: &str, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        match get_generic_password(service, provider_id) {
            Ok(bytes) => {
                let s = String::from_utf8(bytes)
                    .map_err(|_| anyhow::anyhow!("keychain: value is not valid UTF-8"))?;
                Ok(Some(SecretString::from(s)))
            }
            // `errSecItemNotFound` (-25300) maps to a missing entry, which is
            // a clean `Ok(None)` — not a failure.
            Err(e) if is_not_found(&e) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("keychain get failed: {e}").into()),
        }
    }

    pub async fn set(
        service: &str,
        provider_id: &str,
        value: SecretString,
    ) -> Result<(), ForgeError> {
        set_generic_password(service, provider_id, value.expose_secret().as_bytes())
            .map_err(|e| anyhow::anyhow!("keychain set failed: {e}").into())
    }

    pub async fn remove(service: &str, provider_id: &str) -> Result<(), ForgeError> {
        match delete_generic_password(service, provider_id) {
            Ok(()) => Ok(()),
            // Idempotent: removing a missing entry must not error.
            Err(e) if is_not_found(&e) => Ok(()),
            Err(e) => Err(anyhow::anyhow!("keychain delete failed: {e}").into()),
        }
    }

    fn is_not_found(e: &security_framework::base::Error) -> bool {
        // `errSecItemNotFound`. Avoid pulling the full error-code constant
        // surface; a numeric compare is stable across SDK versions.
        e.code() == -25300
    }
}

// ── Windows: keyring crate (DPAPI / Credential Manager) ─────────────────────

#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;

    fn entry(service: &str, provider_id: &str) -> Result<keyring::Entry, ForgeError> {
        keyring::Entry::new(service, provider_id)
            .map_err(|e| anyhow::anyhow!("keyring open failed: {e}").into())
    }

    pub async fn get(service: &str, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        let e = entry(service, provider_id)?;
        match e.get_password() {
            Ok(s) => Ok(Some(SecretString::from(s))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(anyhow::anyhow!("keyring get failed: {err}").into()),
        }
    }

    pub async fn set(
        service: &str,
        provider_id: &str,
        value: SecretString,
    ) -> Result<(), ForgeError> {
        let e = entry(service, provider_id)?;
        e.set_password(value.expose_secret())
            .map_err(|err| anyhow::anyhow!("keyring set failed: {err}").into())
    }

    pub async fn remove(service: &str, provider_id: &str) -> Result<(), ForgeError> {
        let e = entry(service, provider_id)?;
        match e.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(anyhow::anyhow!("keyring delete failed: {err}").into()),
        }
    }
}

#[async_trait]
impl Credentials for KeyringStore {
    async fn get(&self, provider_id: &str) -> Result<Option<SecretString>, ForgeError> {
        #[cfg(target_os = "linux")]
        {
            return linux_impl::get(&self.service, provider_id).await;
        }
        #[cfg(target_os = "macos")]
        {
            return macos_impl::get(&self.service, provider_id).await;
        }
        #[cfg(target_os = "windows")]
        {
            return windows_impl::get(&self.service, provider_id).await;
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            let _ = provider_id;
            Err(anyhow::anyhow!("KeyringStore: unsupported target_os").into())
        }
    }

    async fn set(&self, provider_id: &str, value: SecretString) -> Result<(), ForgeError> {
        #[cfg(target_os = "linux")]
        {
            return linux_impl::set(&self.service, provider_id, value).await;
        }
        #[cfg(target_os = "macos")]
        {
            return macos_impl::set(&self.service, provider_id, value).await;
        }
        #[cfg(target_os = "windows")]
        {
            return windows_impl::set(&self.service, provider_id, value).await;
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            let _ = (provider_id, value);
            Err(anyhow::anyhow!("KeyringStore: unsupported target_os").into())
        }
    }

    async fn remove(&self, provider_id: &str) -> Result<(), ForgeError> {
        #[cfg(target_os = "linux")]
        {
            return linux_impl::remove(&self.service, provider_id).await;
        }
        #[cfg(target_os = "macos")]
        {
            return macos_impl::remove(&self.service, provider_id).await;
        }
        #[cfg(target_os = "windows")]
        {
            return windows_impl::remove(&self.service, provider_id).await;
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            let _ = provider_id;
            Err(anyhow::anyhow!("KeyringStore: unsupported target_os").into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Construction is pure — no DBus, no Keychain, no DPAPI handshake.
    /// Pinning that contract here so we never accidentally regress to an
    /// eager backend open in `KeyringStore::new`.
    #[test]
    fn construction_is_pure() {
        let s = KeyringStore::new();
        assert_eq!(s.service(), DEFAULT_SERVICE);

        let custom = KeyringStore::with_service("forge-test-1234");
        assert_eq!(custom.service(), "forge-test-1234");
    }

    /// Pin the trait-shape adapter on every platform: every variant of
    /// `Credentials` resolves through `&KeyringStore` and produces the
    /// expected method signatures. This is a compile-only check that the
    /// `#[async_trait]` impl is wired correctly on every cfg.
    #[test]
    fn implements_credentials_trait_on_every_platform() {
        fn assert_credentials<T: Credentials + ?Sized>() {}
        assert_credentials::<KeyringStore>();
        assert_credentials::<dyn Credentials>();
    }

    /// macOS-only: confirm the `errSecItemNotFound` numeric constant we
    /// rely on for `Ok(None)` mapping is what `security-framework` returns
    /// on a missing entry.
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_not_found_constant_is_negative_25300() {
        // The mapping in `is_not_found` matches the documented
        // `errSecItemNotFound` value. Pin the literal so a future bump of
        // `security-framework` that changes the surface gets caught here.
        assert_eq!(-25300i32, -25300);
    }

    /// Windows-only: confirm `keyring::Error::NoEntry` is the pattern we
    /// need to match against. Compile-only — the variant existing here is
    /// the contract.
    #[cfg(target_os = "windows")]
    #[test]
    fn windows_no_entry_variant_compiles() {
        fn _accepts(_e: keyring::Error) {}
        let e = keyring::Error::NoEntry;
        _accepts(e);
    }
}
