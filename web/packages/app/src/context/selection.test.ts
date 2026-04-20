import { describe, expect, it } from 'vitest';
import {
  createSelectionResolver,
  type SelectionSnapshot,
} from './selection';

const SNAP: SelectionSnapshot = {
  path: '/ws/src/app.ts',
  startLine: 14,
  endLine: 22,
  text: 'const x = 1;\nconst y = 2;',
};

describe('createSelectionResolver.list', () => {
  it('returns an empty list when no selection is active', async () => {
    const resolver = createSelectionResolver({ getSelection: () => null });
    expect(await resolver.list('')).toEqual([]);
  });

  it('returns one candidate whose label names the file and line range', async () => {
    const resolver = createSelectionResolver({ getSelection: () => SNAP });
    const out = await resolver.list('');
    expect(out).toHaveLength(1);
    expect(out[0]!.category).toBe('selection');
    expect(out[0]!.label).toBe('app.ts @ ln 14-22');
  });
});

describe('createSelectionResolver.resolve', () => {
  it('round-trips the snapshot through the ref JSON', async () => {
    const resolver = createSelectionResolver({ getSelection: () => SNAP });
    const [candidate] = await resolver.list('');
    const block = await resolver.resolve(candidate!.value);
    expect(block.type).toBe('selection');
    expect(block.path).toBe('/ws/src/app.ts');
    expect(block.content).toBe(SNAP.text);
    expect(block.meta).toEqual({ startLine: 14, endLine: 22 });
  });
});
