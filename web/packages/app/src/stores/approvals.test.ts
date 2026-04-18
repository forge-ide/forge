import { describe, expect, it, beforeEach } from 'vitest';
import type { SessionId } from '@forge/ipc';
import {
  makeWhitelistKey,
  defaultPatternForPath,
  matchGlob,
  matchWhitelistKey,
  addWhitelistEntry,
  revokeWhitelistEntry,
  getApprovalWhitelist,
  resetApprovalsStore,
} from './approvals';

const SID = 'session-approvals-test' as SessionId;

beforeEach(() => {
  resetApprovalsStore();
});

// ---------------------------------------------------------------------------
// makeWhitelistKey
// ---------------------------------------------------------------------------

describe('makeWhitelistKey', () => {
  it('produces a file key for ThisFile scope', () => {
    expect(makeWhitelistKey('ThisFile', 'fs.write', '/src/foo.ts')).toBe('file:fs.write:/src/foo.ts');
  });

  it('produces a pattern key for ThisPattern scope with explicit pattern', () => {
    expect(makeWhitelistKey('ThisPattern', 'fs.edit', '/src/foo.ts', '/src/*')).toBe('pattern:fs.edit:/src/*');
  });

  it('falls back to path when pattern is undefined for ThisPattern', () => {
    expect(makeWhitelistKey('ThisPattern', 'fs.edit', '/src/foo.ts')).toBe('pattern:fs.edit:/src/foo.ts');
  });

  it('produces a tool key for ThisTool scope', () => {
    expect(makeWhitelistKey('ThisTool', 'shell.exec', '')).toBe('tool:shell.exec');
  });
});

// ---------------------------------------------------------------------------
// defaultPatternForPath
// ---------------------------------------------------------------------------

describe('defaultPatternForPath', () => {
  it('returns directory glob for a full path', () => {
    expect(defaultPatternForPath('/home/user/src/foo.ts')).toBe('/home/user/src/*');
  });

  it('returns * when no directory component', () => {
    expect(defaultPatternForPath('foo.ts')).toBe('*');
  });

  it('handles trailing slash gracefully', () => {
    expect(defaultPatternForPath('/src/')).toBe('/src/*');
  });
});

// ---------------------------------------------------------------------------
// matchGlob
// ---------------------------------------------------------------------------

describe('matchGlob', () => {
  it('matches path with single-level *', () => {
    expect(matchGlob('/src/*', '/src/foo.ts')).toBe(true);
  });

  it('does not match across directory boundaries with *', () => {
    expect(matchGlob('/src/*', '/src/sub/bar.ts')).toBe(false);
  });

  it('exact path matches itself', () => {
    expect(matchGlob('/src/foo.ts', '/src/foo.ts')).toBe(true);
  });

  it('does not match different paths', () => {
    expect(matchGlob('/src/*', '/other/foo.ts')).toBe(false);
  });

  it('handles dot in path', () => {
    expect(matchGlob('/src/*.ts', '/src/foo.ts')).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// matchWhitelistKey
// ---------------------------------------------------------------------------

describe('matchWhitelistKey', () => {
  it('returns null when whitelist is empty', () => {
    const wl = new Set<string>();
    expect(matchWhitelistKey(wl, 'fs.write', '/src/foo.ts')).toBeNull();
  });

  it('matches ThisTool key', () => {
    const wl = new Set(['tool:fs.write']);
    expect(matchWhitelistKey(wl, 'fs.write', '/src/foo.ts')).toBe('tool:fs.write');
  });

  it('matches ThisFile key', () => {
    const wl = new Set(['file:fs.write:/src/foo.ts']);
    expect(matchWhitelistKey(wl, 'fs.write', '/src/foo.ts')).toBe('file:fs.write:/src/foo.ts');
  });

  it('does not match a file key for a different path', () => {
    const wl = new Set(['file:fs.write:/src/bar.ts']);
    expect(matchWhitelistKey(wl, 'fs.write', '/src/foo.ts')).toBeNull();
  });

  it('matches a pattern key via glob', () => {
    const wl = new Set(['pattern:fs.edit:/src/*']);
    expect(matchWhitelistKey(wl, 'fs.edit', '/src/foo.ts')).toBe('pattern:fs.edit:/src/*');
  });

  it('does not match a pattern key when glob does not apply', () => {
    const wl = new Set(['pattern:fs.edit:/src/*']);
    expect(matchWhitelistKey(wl, 'fs.edit', '/other/foo.ts')).toBeNull();
  });

  it('does not match keys for a different tool', () => {
    const wl = new Set(['tool:fs.write', 'file:fs.write:/src/foo.ts']);
    expect(matchWhitelistKey(wl, 'fs.edit', '/src/foo.ts')).toBeNull();
  });

  it('prefers ThisTool over ThisFile when both present', () => {
    const wl = new Set(['tool:fs.write', 'file:fs.write:/src/foo.ts']);
    expect(matchWhitelistKey(wl, 'fs.write', '/src/foo.ts')).toBe('tool:fs.write');
  });
});

// ---------------------------------------------------------------------------
// Whitelist store operations
// ---------------------------------------------------------------------------

describe('addWhitelistEntry', () => {
  it('adds a ThisFile entry and records label', () => {
    addWhitelistEntry(SID, 'ThisFile', 'fs.write', '/src/foo.ts');
    const wl = getApprovalWhitelist(SID);
    expect('file:fs.write:/src/foo.ts' in wl.entries).toBe(true);
    expect(wl.entries['file:fs.write:/src/foo.ts']).toBe('this file');
  });

  it('adds a ThisPattern entry with explicit pattern', () => {
    addWhitelistEntry(SID, 'ThisPattern', 'fs.edit', '/src/foo.ts', '/src/*');
    const wl = getApprovalWhitelist(SID);
    expect('pattern:fs.edit:/src/*' in wl.entries).toBe(true);
    expect(wl.entries['pattern:fs.edit:/src/*']).toBe('pattern /src/*');
  });

  it('adds a ThisTool entry', () => {
    addWhitelistEntry(SID, 'ThisTool', 'shell.exec', '');
    const wl = getApprovalWhitelist(SID);
    expect('tool:shell.exec' in wl.entries).toBe(true);
    expect(wl.entries['tool:shell.exec']).toBe('this tool');
  });

  it('returns the generated key', () => {
    const key = addWhitelistEntry(SID, 'ThisFile', 'fs.write', '/src/foo.ts');
    expect(key).toBe('file:fs.write:/src/foo.ts');
  });
});

describe('revokeWhitelistEntry', () => {
  it('removes a key from the whitelist', () => {
    addWhitelistEntry(SID, 'ThisFile', 'fs.write', '/src/foo.ts');
    revokeWhitelistEntry(SID, 'file:fs.write:/src/foo.ts');
    const wl = getApprovalWhitelist(SID);
    expect('file:fs.write:/src/foo.ts' in wl.entries).toBe(false);
  });

  it('is a no-op for a key that does not exist', () => {
    expect(() => revokeWhitelistEntry(SID, 'tool:noop')).not.toThrow();
  });
});

describe('multi-session isolation', () => {
  it('keeps whitelist entries isolated per session', () => {
    const SID2 = 'session-approvals-other' as SessionId;
    addWhitelistEntry(SID, 'ThisTool', 'fs.write', '');
    expect('tool:fs.write' in getApprovalWhitelist(SID).entries).toBe(true);
    expect('tool:fs.write' in getApprovalWhitelist(SID2).entries).toBe(false);
  });
});
