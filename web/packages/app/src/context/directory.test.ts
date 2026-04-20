import { describe, expect, it, vi } from 'vitest';
import type { SessionId } from '@forge/ipc';
import type { TreeNodeDto } from '../ipc/fs';
import {
  createDirectoryResolver,
  flattenAllPaths,
  flattenDirectories,
  DIRECTORY_RESOLVER_MAX_PATHS,
} from './directory';

const SESSION: SessionId = 'session-1' as SessionId;
const ROOT = '/tmp/ws';

function fileNode(path: string, name: string): TreeNodeDto {
  return { path, name, kind: 'File', children: null };
}
function dirNode(path: string, name: string, children: TreeNodeDto[]): TreeNodeDto {
  return { path, name, kind: 'Dir', children };
}

describe('flattenDirectories', () => {
  it('collects directory nodes (including root) and skips files', () => {
    const tree = dirNode('/ws', 'ws', [
      fileNode('/ws/a.ts', 'a.ts'),
      dirNode('/ws/tests', 'tests', [fileNode('/ws/tests/a.test.ts', 'a.test.ts')]),
    ]);
    expect(flattenDirectories(tree).map((d) => d.path)).toEqual([
      '/ws',
      '/ws/tests',
    ]);
  });
});

describe('flattenAllPaths', () => {
  it('emits every path in DFS order', () => {
    const tree = dirNode('/ws', 'ws', [
      fileNode('/ws/a.ts', 'a.ts'),
      dirNode('/ws/sub', 'sub', [fileNode('/ws/sub/b.ts', 'b.ts')]),
    ]);
    expect(flattenAllPaths(tree)).toEqual([
      '/ws',
      '/ws/a.ts',
      '/ws/sub',
      '/ws/sub/b.ts',
    ]);
  });
});

describe('createDirectoryResolver.list', () => {
  it('surfaces directory nodes and filters by query', async () => {
    const treeFn = vi.fn().mockResolvedValue(
      dirNode('/ws', 'ws', [
        dirNode('/ws/tests', 'tests', [
          dirNode('/ws/tests/payments', 'payments', []),
        ]),
        dirNode('/ws/src', 'src', []),
      ]),
    );
    const resolver = createDirectoryResolver({
      sessionId: SESSION,
      workspaceRoot: ROOT,
      tree: treeFn,
    });
    const out = await resolver.list('pay');
    expect(out.map((c) => c.value)).toEqual(['/ws/tests/payments']);
  });
});

describe('createDirectoryResolver.resolve', () => {
  it('walks the picked directory and returns a path snapshot', async () => {
    const treeFn = vi
      .fn()
      .mockResolvedValue(
        dirNode('/ws/src', 'src', [
          fileNode('/ws/src/a.ts', 'a.ts'),
          fileNode('/ws/src/b.ts', 'b.ts'),
        ]),
      );
    const resolver = createDirectoryResolver({
      sessionId: SESSION,
      workspaceRoot: ROOT,
      tree: treeFn,
    });
    const block = await resolver.resolve('/ws/src');
    expect(treeFn).toHaveBeenCalledWith(SESSION, '/ws/src');
    expect(block.type).toBe('directory');
    expect(block.path).toBe('/ws/src');
    expect(block.content).toBe('/ws/src\n/ws/src/a.ts\n/ws/src/b.ts');
  });

  it('caps at DIRECTORY_RESOLVER_MAX_PATHS and notes truncation', async () => {
    const children = Array.from({ length: 300 }, (_, i) =>
      fileNode(`/ws/f${i}.ts`, `f${i}.ts`),
    );
    const treeFn = vi.fn().mockResolvedValue(dirNode('/ws', 'ws', children));
    const resolver = createDirectoryResolver({
      sessionId: SESSION,
      workspaceRoot: ROOT,
      tree: treeFn,
    });
    const block = await resolver.resolve('/ws');
    const lines = block.content.split('\n');
    // First DIRECTORY_RESOLVER_MAX_PATHS lines are the truncated paths,
    // followed by a "+N more — truncated" marker line.
    expect(lines.length).toBe(DIRECTORY_RESOLVER_MAX_PATHS + 1);
    expect(lines[lines.length - 1]).toMatch(/\+\d+ more — truncated/);
  });
});
