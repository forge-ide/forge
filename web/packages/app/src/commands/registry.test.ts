// F-157: command-palette registry — pure unit tests.
//
// The registry is a plain module: it owns a module-scoped map of commands
// keyed by `id`. Tests reset it between cases via `__resetRegistryForTests`.
// `fuzzyMatch` is a pure function (subsequence scorer) kept next to the
// registry so the palette component can filter without a runtime dep.

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  filterCommandsByQuery,
  fuzzyMatch,
  listCommands,
  registerCommand,
  unregisterCommand,
  __resetRegistryForTests,
} from './registry';

afterEach(() => {
  __resetRegistryForTests();
});

describe('command registry (F-157)', () => {
  it('registerCommand adds a command that listCommands returns', () => {
    const run = vi.fn();
    registerCommand({ id: 'a', title: 'Alpha', run });
    const cmds = listCommands();
    expect(cmds).toHaveLength(1);
    expect(cmds[0]).toMatchObject({ id: 'a', title: 'Alpha' });
  });

  it('registerCommand with a duplicate id replaces the previous entry', () => {
    const first = vi.fn();
    const second = vi.fn();
    registerCommand({ id: 'dup', title: 'First', run: first });
    registerCommand({ id: 'dup', title: 'Second', run: second });
    const cmds = listCommands();
    expect(cmds).toHaveLength(1);
    expect(cmds[0]?.title).toBe('Second');
  });

  it('unregisterCommand removes the command by id and is a no-op for unknown ids', () => {
    registerCommand({ id: 'a', title: 'Alpha', run: vi.fn() });
    registerCommand({ id: 'b', title: 'Bravo', run: vi.fn() });
    unregisterCommand('a');
    expect(listCommands().map((c) => c.id)).toEqual(['b']);
    // unknown id — just returns false
    expect(unregisterCommand('nope')).toBe(false);
  });

  it('registerCommand returns a disposer that removes the command', () => {
    const dispose = registerCommand({ id: 'a', title: 'Alpha', run: vi.fn() });
    expect(listCommands()).toHaveLength(1);
    dispose();
    expect(listCommands()).toHaveLength(0);
  });
});

describe('fuzzyMatch (F-157)', () => {
  it('returns null when the query characters are not a subsequence of the title', () => {
    expect(fuzzyMatch('xyz', 'Alpha Bravo')).toBeNull();
  });

  it('returns a non-null score when all query chars appear in order (case-insensitive)', () => {
    const out = fuzzyMatch('ab', 'Alpha Bravo');
    expect(out).not.toBeNull();
    expect(out?.matched).toBe(true);
  });

  it('empty query matches every title with a neutral score', () => {
    const out = fuzzyMatch('', 'anything');
    expect(out).not.toBeNull();
    expect(out?.matched).toBe(true);
  });

  it('scores a prefix match higher than a scattered match', () => {
    const prefix = fuzzyMatch('open', 'Open Agent Monitor');
    const scattered = fuzzyMatch('open', 'Compile Projects Entirely Now');
    expect(prefix).not.toBeNull();
    expect(scattered).not.toBeNull();
    // Higher score = better match.
    expect(prefix!.score).toBeGreaterThan(scattered!.score);
  });

  it('filterCommandsByQuery sorts by descending score and drops non-matches', () => {
    registerCommand({ id: 'one', title: 'Open Agent Monitor', run: vi.fn() });
    registerCommand({ id: 'two', title: 'Open Settings', run: vi.fn() });
    registerCommand({ id: 'three', title: 'Restart Session', run: vi.fn() });
    const results = filterCommandsByQuery('open');
    // Both "Open …" titles survive; "Restart Session" drops.
    expect(results.map((c) => c.id)).toEqual(['one', 'two']);
  });
});
