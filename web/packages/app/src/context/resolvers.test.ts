import { describe, expect, it, vi } from 'vitest';
import type { SessionId } from '@forge/ipc';
import type { TreeNodeDto } from '../ipc/fs';
import { buildRegistry, listCandidates, resolveChips } from './resolvers';

const SESSION: SessionId = 'session-1' as SessionId;
const ROOT = '/tmp/ws';

function file(path: string, name: string): TreeNodeDto {
  return { path, name, kind: 'File', children: null };
}
function dir(path: string, name: string, children: TreeNodeDto[]): TreeNodeDto {
  return { path, name, kind: 'Dir', children };
}

describe('buildRegistry', () => {
  it('includes only categories whose deps were passed', () => {
    const reg = buildRegistry({
      skill: { listSkills: () => [] },
    });
    expect(reg.skill).toBeDefined();
    expect(reg.file).toBeUndefined();
    expect(reg.url).toBeUndefined();
  });
});

describe('listCandidates', () => {
  it('fans out across resolvers and drops empty tabs', async () => {
    const treeFn = vi
      .fn()
      .mockResolvedValue(dir('/ws', 'ws', [file('/ws/app.ts', 'app.ts')]));
    const reg = buildRegistry({
      file: { sessionId: SESSION, workspaceRoot: ROOT, tree: treeFn },
      skill: { listSkills: () => [{ name: 'ts-review' }] },
    });
    const items = await listCandidates(reg, '');
    expect(items.file?.status).toBe('success');
    expect(items.file?.status === 'success' ? items.file.items : []).toHaveLength(1);
    expect(items.skill?.status === 'success' ? items.skill.items : []).toHaveLength(1);
    // Categories without registered resolvers are absent
    expect(items.agent).toBeUndefined();
    expect(items.url).toBeUndefined();
  });

  it('a failing resolver yields an empty tab, not a thrown promise', async () => {
    const treeFn = vi.fn().mockRejectedValue(new Error('tree failed'));
    const reg = buildRegistry({
      file: { sessionId: SESSION, workspaceRoot: ROOT, tree: treeFn },
    });
    await expect(listCandidates(reg, '')).resolves.toEqual({});
  });

  // F-536: tree-backed resolvers thread the root TreeStatsDto through the
  // CategoryState so the picker can render a truncation/error notice.
  it('threads TreeStatsDto from tree-backed resolvers onto CategoryState.stats', async () => {
    const treeFn = vi.fn().mockResolvedValue({
      ...dir('/ws', 'ws', [file('/ws/a.ts', 'a.ts')]),
      stats: { truncated: true, omitted_count: 42, error_count: 0 },
    });
    const reg = buildRegistry({
      file: { sessionId: SESSION, workspaceRoot: ROOT, tree: treeFn },
    });
    const items = await listCandidates(reg, '');
    expect(items.file?.status).toBe('success');
    if (items.file?.status === 'success') {
      expect(items.file.stats).toEqual({
        truncated: true,
        omitted_count: 42,
        error_count: 0,
      });
    }
  });

  // F-536: keep a tab whose resolver returned no items but reported stats —
  // otherwise the user filters every candidate out and loses the notice.
  it('keeps a tab when items are empty but stats are present', async () => {
    const treeFn = vi.fn().mockResolvedValue({
      ...dir('/ws', 'ws', [file('/ws/a.ts', 'a.ts')]),
      stats: { truncated: true, omitted_count: 1, error_count: 0 },
    });
    const reg = buildRegistry({
      file: { sessionId: SESSION, workspaceRoot: ROOT, tree: treeFn },
    });
    const items = await listCandidates(reg, 'no-such-file-matches-this');
    expect(items.file).toBeDefined();
    if (items.file?.status === 'success') {
      expect(items.file.items).toEqual([]);
      expect(items.file.stats?.truncated).toBe(true);
    }
  });
});

describe('resolveChips', () => {
  it('resolves each chip to a block and serializes for Anthropic', async () => {
    const treeFn = vi.fn();
    const readFn = vi.fn().mockResolvedValue({
      path: '/ws/a.ts',
      content: 'A',
      bytes: 1,
      sha256: 'x',
    });
    const reg = buildRegistry({
      file: {
        sessionId: SESSION,
        workspaceRoot: ROOT,
        tree: treeFn,
        readFile: readFn,
      },
    });
    const out = await resolveChips(
      reg,
      [{ category: 'file', label: 'a.ts', value: '/ws/a.ts' }],
      'anthropic',
    );
    expect(out).toContain('<context type="file" path="/ws/a.ts">');
    expect(out).toContain('A');
  });

  it('uses OpenAI function-context shape for openai provider id', async () => {
    const reg = buildRegistry({ skill: { listSkills: () => [] } });
    const out = await resolveChips(
      reg,
      [{ category: 'skill', label: 'ts', value: 'ts' }],
      'gpt-4o',
    );
    expect(out).toContain('[context(type="skill")]');
  });

  it('returns empty string when no chips', async () => {
    expect(await resolveChips({}, [], 'anthropic')).toBe('');
  });

  it('drops chips whose category has no registered resolver', async () => {
    // file chip, no file resolver -> dropped silently -> empty adapter output
    const reg = buildRegistry({ skill: { listSkills: () => [] } });
    const out = await resolveChips(
      reg,
      [{ category: 'file', label: 'x.ts', value: '/ws/x.ts' }],
      'anthropic',
    );
    expect(out).toBe('');
  });
});
