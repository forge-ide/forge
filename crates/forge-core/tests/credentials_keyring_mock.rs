//! F-587: Linux keyring integration test using a mock backend.
//!
//! The real Linux backend talks to a session-scoped Secret Service daemon
//! over DBus. CI runners typically don't expose one, so an integration test
//! that drives the live `secret-service` crate would be unstable. Instead,
//! we exercise the **`Credentials` trait surface** with an in-memory
//! "keyring mock" that mimics the Secret Service's observable behavior:
//!
//! - `set` overwrites in place (Secret Service `replace=true`)
//! - `remove` is idempotent and tolerates missing entries
//! - `get` round-trips with byte-exact value preservation
//! - misses return `Ok(None)`, never `Err`
//!
//! Composed with `EnvFallbackStore` via `LayeredStore`, this is the exact
//! production wiring on Linux. The test pins the contract that the
//! orchestrator-side path relies on; the real `KeyringStore` is exercised
//! manually under `RUSTFLAGS=--cfg forge_live_keyring cargo test`.

#![cfg(target_os = "linux")]

use forge_core::credentials::env::ANTHROPIC_ENV_VAR;
use forge_core::{Credentials, EnvFallbackStore, LayeredStore, MemoryStore};
use secrecy::{ExposeSecret, SecretString};
use std::collections::HashMap;

fn env_with(values: HashMap<String, String>) -> EnvFallbackStore {
    let mut mapping = HashMap::new();
    mapping.insert("anthropic".to_string(), ANTHROPIC_ENV_VAR.to_string());
    EnvFallbackStore::with_reader(mapping, move |name| values.get(name).cloned())
}

#[tokio::test]
async fn keyring_mock_set_get_remove_round_trip() {
    // The keyring mock is a `MemoryStore` — its observable shape matches
    // the Secret Service contract for our trait (overwrite-on-set,
    // idempotent remove, `Ok(None)` on miss).
    let keyring = MemoryStore::new();

    keyring
        .set("anthropic", SecretString::from("sk-ant-mock-1"))
        .await
        .unwrap();

    let got = keyring.get("anthropic").await.unwrap().unwrap();
    assert_eq!(got.expose_secret(), "sk-ant-mock-1");

    // Overwrite — Secret Service `create_item(replace=true)`.
    keyring
        .set("anthropic", SecretString::from("sk-ant-mock-2"))
        .await
        .unwrap();
    let got = keyring.get("anthropic").await.unwrap().unwrap();
    assert_eq!(got.expose_secret(), "sk-ant-mock-2");

    // Remove + idempotent re-remove.
    keyring.remove("anthropic").await.unwrap();
    keyring.remove("anthropic").await.unwrap();
    assert!(keyring.get("anthropic").await.unwrap().is_none());
}

#[tokio::test]
async fn layered_keyring_first_then_env_fallback() {
    // Production wiring on Linux: KeyringStore primary, EnvFallbackStore
    // fallback. Keyring hit short-circuits the env read.
    let keyring = MemoryStore::new();
    keyring
        .set("anthropic", SecretString::from("from-keyring"))
        .await
        .unwrap();

    let mut env = HashMap::new();
    env.insert(ANTHROPIC_ENV_VAR.to_string(), "from-env".to_string());

    let layered = LayeredStore::new(keyring, env_with(env));
    let got = layered.get("anthropic").await.unwrap().unwrap();
    assert_eq!(got.expose_secret(), "from-keyring");
}

#[tokio::test]
async fn layered_keyring_miss_falls_through_to_env() {
    let keyring = MemoryStore::new();
    let mut env = HashMap::new();
    env.insert(ANTHROPIC_ENV_VAR.to_string(), "from-env".to_string());

    let layered = LayeredStore::new(keyring, env_with(env));
    let got = layered.get("anthropic").await.unwrap().unwrap();
    assert_eq!(got.expose_secret(), "from-env");
}

#[tokio::test]
async fn layered_set_writes_to_keyring_not_env() {
    let layered = LayeredStore::new(MemoryStore::new(), env_with(HashMap::new()));
    layered
        .set("anthropic", SecretString::from("written"))
        .await
        .unwrap();

    // After write: `get` returns from the keyring layer (primary).
    let got = layered.get("anthropic").await.unwrap().unwrap();
    assert_eq!(got.expose_secret(), "written");
}

#[tokio::test]
async fn unknown_provider_id_is_clean_miss_not_error() {
    // Per the trait contract: a missing entry is `Ok(None)`, never
    // `Err(_)`. This is what `LayeredStore` relies on for clean fall-through.
    let layered = LayeredStore::new(MemoryStore::new(), env_with(HashMap::new()));
    assert!(layered.get("not-a-real-provider").await.unwrap().is_none());
    assert!(!layered.has("not-a-real-provider").await.unwrap());
}
