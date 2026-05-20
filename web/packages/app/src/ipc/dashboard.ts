// F-365: typed wrappers for Dashboard-level Tauri commands that previously
// reached through raw `invoke()`. Centralising them here makes the command
// surface discoverable and prevents arg-key drift when the Rust signatures
// change.

import { invoke } from '../lib/tauri';
import type { CredentialState, GitBranchOutput } from '@forge/ipc';

export type { CredentialState };

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
  /**
   * Canonical absolute path of the workspace owning the session. Surfaced
   * on every row so the workspace window's ChatPane can filter the full
   * session list to its workspace; the dashboard reads it to label rows
   * by workspace name.
   */
  workspaceRoot: string;
  /**
   * Stable workspace id (from the workspaces registry). Maps 1:1 to the
   * Tauri window label `workspace-<workspaceId>`.
   */
  workspaceId: string;
}

/** Fetch the list of all sessions known to the shell. */
export async function sessionList(): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>('session_list');
}

/**
 * F-727: Fetch the subset of sessions the hero's `Attach to session` picker
 * can re-open — wire-state `stopped` (the daemon process is gone). The
 * shell filters server-side; the webview consumes the result directly with
 * no additional state pass.
 */
export async function listAttachableSessions(): Promise<SessionSummary[]> {
  return invoke<SessionSummary[]>('list_sessions');
}

/**
 * Reopen the Session window for `id`. The shell brings an existing window to
 * the front or spawns a new one when the window was previously closed.
 */
export async function openSession(id: string): Promise<void> {
  await invoke('open_session', { id });
}

/**
 * Seed `~/.config/forge/workspaces.toml` with `workspaceRoot` so the
 * registry gate inside `resolve_workspace_root_for_command` will accept it
 * on subsequent dashboard list commands.
 *
 * Required for dashboard surfaces that let the user pick a workspace
 * outside the `+ New session` flow (today: `EnabledAssetsCard`'s
 * "Open workspace" CTA). `session_start` registers internally, so
 * `NewSessionDialog` does not need to call this.
 *
 * Returns the canonical form of `workspaceRoot` — callers should persist
 * the canonical string (matching what `session_hello` would return).
 */
export async function registerWorkspace(workspaceRoot: string): Promise<string> {
  return invoke<string>('register_workspace', { workspaceRoot });
}

// ---------------------------------------------------------------------------
// Provider selection (F-586)
// ---------------------------------------------------------------------------

/**
 * One row of `dashboard_list_providers`. Stable id (slug), display name,
 * and the dashboard's state model:
 *   - `credential_required && credential_state !== 'present'` → warning glyph
 *   - `model_available` false                                 → secondary "no model" hint
 *   - `model` populated                                       → secondary line shows it
 *
 * Mirrors the Rust `ProviderEntry` shape one-for-one.
 */
export interface ProviderEntry {
  id: string;
  display_name: string;
  credential_required: boolean;
  /**
   * Pre-F-755 boolean credential signal — kept for back-compat. Equal to
   * `credential_state === 'present'` when `credential_required` is true.
   * New consumers should read `credential_state` directly so the
   * locked-keyring case can be distinguished from the missing-entry case.
   */
  has_credential: boolean;
  /**
   * F-755: tri-state probe outcome. Optional on the TS surface so test
   * fixtures that pre-date F-755 still type-check; readers MUST default
   * `undefined` to `'missing'` when `credential_required` is true (the
   * conservative bias the Rust shell already applies for unprobed ids).
   */
  credential_state?: CredentialState;
  model_available: boolean;
  model?: string;
  /** `base_url` of the underlying custom_openai section (custom rows only). */
  endpoint?: string;
  /**
   * F-733: per-provider enable flag. The Providers page toggle is the only
   * writer; absent settings default to `true` so historical configs (pre
   * F-730) keep their built-ins live. The active-provider selector and the
   * new-session picker filter rows where `enabled === false`. Optional on
   * the TS surface so legacy IPC payloads — and test fixtures that pre-date
   * F-733 — still type-check; readers MUST treat `undefined` as enabled.
   */
  enabled?: boolean;
  /**
   * Phase B: authentication mode for named built-in instances. `'vertex'`
   * means the row resolves auth via gcloud Application Default
   * Credentials at request time — the dashboard suppresses the
   * "ADD CREDENTIAL" CTA and the orange auth pill for these entries.
   * `'api_key'` (or `undefined` for legacy payloads) keeps the existing
   * keychain-backed flow.
   */
  auth_kind?: 'api_key' | 'vertex';
}

/**
 * F-733: read-site helper. The `enabled` flag is optional on the wire so
 * pre-F-733 payloads (where the field is absent) treat as enabled. Use
 * this anywhere a UI must filter disabled rows.
 */
export function isProviderEnabled(entry: ProviderEntry): boolean {
  return entry.enabled !== false;
}

/**
 * List the built-in providers (Anthropic, OpenAI) plus one row per
 * user-configured `custom_openai:<name>` entry. Wraps the
 * `dashboard_list_providers` Tauri command — the
 * `dashboard_` prefix disambiguates from F-591's planned roster catalog
 * `list_providers` command (Tauri rejects duplicate command names).
 */
export async function listProviders(): Promise<ProviderEntry[]> {
  return invoke<ProviderEntry[]>('dashboard_list_providers');
}

/** Read the persisted `[providers.active]` setting (user-tier, global). */
export async function getActiveProvider(): Promise<string | null> {
  return invoke<string | null>('get_active_provider');
}

/**
 * Persist the active provider id and emit `provider:changed` app-wide so
 * any open session window's bridge can swap its inner Provider for the
 * next turn.
 */
export async function setActiveProvider(providerId: string): Promise<void> {
  await invoke('set_active_provider', { providerId });
}

// ---------------------------------------------------------------------------
// F-741: status-bar git branch feed
// ---------------------------------------------------------------------------

/**
 * Resolve the active branch name for `workspaceRoot` by shelling out to
 * `git rev-parse --abbrev-ref HEAD` inside the directory. Returns the
 * branch name on success; `null` when the working tree has a detached
 * HEAD. IPC rejections propagate to the caller, which paints `unknown`
 * per the status-bar contract.
 */
export async function gitBranch(workspaceRoot: string): Promise<string | null> {
  // The IPC contract returns `GitBranchOutput { branch }`, but test invoke
  // stubs and a (hypothetical) misshaped daemon response can land here as
  // `undefined`. Optional chaining keeps the wrapper honest about its
  // `string | null` return shape without a runtime TypeError.
  const out = await invoke<GitBranchOutput | null | undefined>('git_branch', {
    input: { workspace_root: workspaceRoot },
  });
  return out?.branch ?? null;
}
