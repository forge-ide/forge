// F-365: typed wrappers for Dashboard-level Tauri commands that previously
// reached through raw `invoke()`. Centralising them here makes the command
// surface discoverable and prevents arg-key drift when the Rust signatures
// change.

import { invoke } from '../lib/tauri';

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

export type SessionWireState = 'active' | 'archived' | 'stopped';

export interface SessionSummary {
  id: string;
  subject: string;
  state: SessionWireState;
  persistence: 'persist' | 'ephemeral';
  createdAt: string;
  lastEventAt: string;
  /** Optional; provider chip is shown when present. */
  provider?: string;
}

/** Fetch the list of all sessions known to the shell. */
export async function sessionList(): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>('session_list');
}

/**
 * Reopen the Session window for `id`. The shell brings an existing window to
 * the front or spawns a new one when the window was previously closed.
 */
export async function openSession(id: string): Promise<void> {
  await invoke('open_session', { id });
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

export interface ProviderStatus {
  reachable: boolean;
  base_url: string;
  models: string[];
  last_checked: string;
  error_kind?: string;
}

/** Probe the configured AI provider and return its current status. */
export async function providerStatus(): Promise<ProviderStatus> {
  return invoke<ProviderStatus>('provider_status');
}
