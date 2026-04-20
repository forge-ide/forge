import { describe, expect, it, vi } from 'vitest';
import type { SessionId } from '@forge/ipc';
import {
  createAgentResolver,
  summarizeTranscript,
  type TranscriptEntry,
} from './agent';

describe('summarizeTranscript', () => {
  it('keeps only the last 20 turns', () => {
    const entries: TranscriptEntry[] = Array.from({ length: 30 }, (_, i) => ({
      role: (i % 2 === 0 ? 'user' : 'assistant') as 'user' | 'assistant',
      text: `turn ${i}`,
    }));
    const out = summarizeTranscript(entries);
    expect(out.split('\n')).toHaveLength(20);
    expect(out.split('\n')[0]).toMatch(/turn 10/);
  });

  it('truncates long messages with an ellipsis', () => {
    const long = 'x'.repeat(200);
    const out = summarizeTranscript([{ role: 'user', text: long }]);
    expect(out.length).toBeLessThan(long.length + 10);
    expect(out.endsWith('…')).toBe(true);
  });
});

describe('createAgentResolver.list', () => {
  it('lists known agents and filters by query', async () => {
    const resolver = createAgentResolver({
      listAgents: () => [
        { id: 's-1' as SessionId, label: 'refactor-bot' },
        { id: 's-2' as SessionId, label: 'design-reviewer' },
      ],
    });
    const out = await resolver.list('refactor');
    expect(out).toEqual([
      { category: 'agent', label: 'refactor-bot', value: 's-1' },
    ]);
  });

  it('falls back to the session id when no label is given', async () => {
    const resolver = createAgentResolver({
      listAgents: () => [{ id: 'sess-xyz' as SessionId }],
    });
    const out = await resolver.list('');
    expect(out[0]!.label).toBe('sess-xyz');
  });
});

describe('createAgentResolver.resolve', () => {
  it('calls getTranscript and summarizes the result', async () => {
    const getTranscript = vi
      .fn<(id: SessionId) => Promise<TranscriptEntry[]>>()
      .mockResolvedValue([
        { role: 'user', text: 'Fix the bug' },
        { role: 'assistant', text: 'Patch applied.' },
      ]);
    const resolver = createAgentResolver({
      listAgents: () => [],
      getTranscript,
    });
    const block = await resolver.resolve('s-42');
    expect(getTranscript).toHaveBeenCalledWith('s-42');
    expect(block.type).toBe('agent');
    expect(block.content).toBe('user: Fix the bug\nassistant: Patch applied.');
    expect(block.meta).toEqual({ agentSessionId: 's-42' });
  });
});
