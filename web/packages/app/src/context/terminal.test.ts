import { describe, expect, it } from 'vitest';
import {
  createTerminalResolver,
  type TerminalSnapshot,
} from './terminal';

const SNAP: TerminalSnapshot = {
  terminalId: 'abc123',
  shellName: 'zsh',
  text: 'line-1\nline-2\nline-3',
  lineCount: 3,
};

describe('createTerminalResolver.list', () => {
  it('returns [] when no terminal is focused', async () => {
    const resolver = createTerminalResolver({ getSnapshot: () => null });
    expect(await resolver.list('')).toEqual([]);
  });

  it('returns one candidate with shell + line count', async () => {
    const resolver = createTerminalResolver({ getSnapshot: () => SNAP });
    const out = await resolver.list('');
    expect(out).toHaveLength(1);
    expect(out[0]!.label).toBe('zsh — last 3 lines');
  });
});

describe('createTerminalResolver.resolve', () => {
  it('carries the text through and annotates meta', async () => {
    const resolver = createTerminalResolver({ getSnapshot: () => SNAP });
    const [candidate] = await resolver.list('');
    const block = await resolver.resolve(candidate!.value);
    expect(block.type).toBe('terminal');
    expect(block.content).toBe(SNAP.text);
    expect(block.meta).toEqual({ terminalId: 'abc123', shellName: 'zsh' });
  });
});
