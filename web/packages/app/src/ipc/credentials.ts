// F-588: typed wrappers for the F-587 credential Tauri commands.
//
// The backend authoritatively gates these commands to the `dashboard` window
// label, so callers must originate from the Dashboard route. The actual key
// only flows through `loginProvider` and never leaves the Rust boundary —
// `hasCredential` returns presence and `logoutProvider` returns void.

import type { ProviderId } from '@forge/ipc';
import { invoke } from '../lib/tauri';

/**
 * Persist `key` for `providerId` into the active credential store
 * (production wires `LayeredStore<KeyringStore, EnvFallbackStore>`). The
 * value is wrapped in `SecretString` on the Rust side immediately on
 * arrival; this wrapper does no additional caching.
 *
 * Empty `providerId` or `key` are rejected locally before the IPC call.
 * The Rust side enforces the same shape, but failing fast on the
 * renderer avoids a round-trip on a degenerate request and keeps the
 * error surface consistent with this wrapper's contract.
 *
 * Callers MUST clear `key` from any local state immediately after the
 * Promise resolves — see voice-rule §"Stored-key state shown via masked
 * indicator only — never reveals the value".
 */
export async function loginProvider(providerId: ProviderId, key: string): Promise<void> {
  if (!providerId) throw new Error('loginProvider: providerId must not be empty');
  if (!key) throw new Error('loginProvider: key must not be empty');
  await invoke('login_provider', { providerId, key });
}

/**
 * Remove the credential for `providerId` from the active store. The
 * keyring layer drops the entry; the env-var fallback is read-only and
 * silently ignores the call (see `EnvFallbackStore::remove`).
 */
export async function logoutProvider(providerId: ProviderId): Promise<void> {
  if (!providerId) throw new Error('logoutProvider: providerId must not be empty');
  await invoke('logout_provider', { providerId });
}

/**
 * Presence probe — returns `true` when any layer of the credential store
 * holds an entry for `providerId`. The credential value is never returned
 * across the IPC boundary by this command.
 */
export async function hasCredential(providerId: ProviderId): Promise<boolean> {
  if (!providerId) throw new Error('hasCredential: providerId must not be empty');
  return invoke<boolean>('has_credential', { providerId });
}
