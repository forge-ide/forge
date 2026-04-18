import { createStore, produce, reconcile } from 'solid-js/store';
import type { SessionId, ApprovalScope } from '@forge/ipc';

// ---------------------------------------------------------------------------
// Whitelist key derivation
// ---------------------------------------------------------------------------

/**
 * Derive the whitelist key for a given scope + tool context.
 * Keys must be deterministic so subsequent matching calls can check membership.
 *
 * Once      → never added to whitelist (per-call only)
 * ThisFile   → "file:<toolName>:<path>"
 * ThisPattern → "pattern:<toolName>:<glob>"
 * ThisTool   → "tool:<toolName>"
 */
export function makeWhitelistKey(
  scope: Exclude<ApprovalScope, 'Once'>,
  toolName: string,
  path: string,
  pattern?: string,
): string {
  switch (scope) {
    case 'ThisFile':
      return `file:${toolName}:${path}`;
    case 'ThisPattern':
      return `pattern:${toolName}:${pattern ?? path}`;
    case 'ThisTool':
      return `tool:${toolName}`;
  }
}

/**
 * Derive the default glob pattern for a file path.
 * e.g. /home/user/src/foo.ts → /home/user/src/*
 */
export function defaultPatternForPath(path: string): string {
  const slash = path.lastIndexOf('/');
  if (slash > 0) {
    return path.slice(0, slash) + '/*';
  }
  return '*';
}

/**
 * Check whether a given tool call + scope combination is whitelisted.
 * Returns the matching key if whitelisted, null otherwise.
 */
export function matchWhitelistKey(
  whitelistKeys: Set<string>,
  toolName: string,
  path: string,
): string | null {
  // Check ThisTool first (broadest)
  const toolKey = `tool:${toolName}`;
  if (whitelistKeys.has(toolKey)) return toolKey;

  // Check ThisFile
  if (path) {
    const fileKey = `file:${toolName}:${path}`;
    if (whitelistKeys.has(fileKey)) return fileKey;

    // Check any ThisPattern entry that matches the path
    for (const key of whitelistKeys) {
      if (key.startsWith(`pattern:${toolName}:`)) {
        const glob = key.slice(`pattern:${toolName}:`.length);
        if (matchGlob(glob, path)) return key;
      }
    }
  }

  return null;
}

/**
 * Simple glob matcher supporting * wildcard (not recursive).
 * e.g. /src/* matches /src/foo.ts but not /src/sub/bar.ts
 */
export function matchGlob(glob: string, path: string): boolean {
  // Escape all regex special chars except *
  const regex = new RegExp(
    '^' +
      glob
        .replace(/[.+^${}()|[\]\\]/g, '\\$&')
        .replace(/\*/g, '[^/]*') +
      '$',
  );
  return regex.test(path);
}

// ---------------------------------------------------------------------------
// Per-session whitelist store
//
// We use a plain object record keyed by whitelist-key for SolidJS reactive
// tracking. Storing Set/Map inside a store would not be tracked reactively.
// ---------------------------------------------------------------------------

export interface ApprovalWhitelist {
  /**
   * Record of whitelist-key → human-readable label.
   * A key is present iff it is whitelisted. Use Object.keys() to enumerate.
   */
  entries: Record<string, string>;
}

const [approvalsStore, setApprovalsStore] = createStore<
  Record<string, ApprovalWhitelist>
>({});

function ensureSession(sessionId: SessionId): void {
  if (!approvalsStore[sessionId]) {
    setApprovalsStore(sessionId, { entries: {} });
  }
}

export function getApprovalWhitelist(sessionId: SessionId): ApprovalWhitelist {
  ensureSession(sessionId);
  return approvalsStore[sessionId]!;
}

/**
 * Record a whitelist entry for a scope > Once.
 * Returns the key that was added.
 */
export function addWhitelistEntry(
  sessionId: SessionId,
  scope: Exclude<ApprovalScope, 'Once'>,
  toolName: string,
  path: string,
  pattern?: string,
): string {
  ensureSession(sessionId);
  const key = makeWhitelistKey(scope, toolName, path, pattern);
  const label = scopeLabel(scope, pattern ?? path);
  setApprovalsStore(
    produce((s) => {
      s[sessionId]!.entries[key] = label;
    }),
  );
  return key;
}

/**
 * Revoke a whitelist entry by its key.
 */
export function revokeWhitelistEntry(sessionId: SessionId, key: string): void {
  ensureSession(sessionId);
  setApprovalsStore(
    produce((s) => {
      delete s[sessionId]!.entries[key];
    }),
  );
}

function scopeLabel(
  scope: Exclude<ApprovalScope, 'Once'>,
  pathOrPattern: string,
): string {
  switch (scope) {
    case 'ThisFile':
      return 'this file';
    case 'ThisPattern':
      return `pattern ${pathOrPattern}`;
    case 'ThisTool':
      return 'this tool';
  }
}

/** Test helper — clears all approval whitelist state. */
export function resetApprovalsStore(): void {
  setApprovalsStore(reconcile({}));
}
