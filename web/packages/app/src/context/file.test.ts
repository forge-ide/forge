import { describe, expect, it, vi } from 'vitest';
import type { SessionId } from '@forge/ipc';
import type { TreeNodeDto, FileContent } from '../ipc/fs';
import {
  createFileResolver,
  flattenFiles,
  truncateToBytes,
  FILE_RESOLVER_MAX_BYTES,
  FILE_RESOLVER_MAX_RESULTS,
} from './file';

const SESSION: SessionId = 'session-1' as SessionId;
const ROOT = '/tmp/ws';

function fileNode(path: string, name: string): TreeNodeDto {
  return { path, name, kind: 'File', children: null };
}
function dirNode(path: string, name: string, children: TreeNodeDto[]): TreeNodeDto {
  return { path, name, kind: 'Dir', children };
}

describe('flattenFiles', () => {
  it('collects only file entries and descends into directories', () => {
    const tree: TreeNodeDto = dirNode('/ws', 'ws', [
      fileNode('/ws/a.ts', 'a.ts'),
      dirNode('/ws/sub', 'sub', [
        fileNode('/ws/sub/b.ts', 'b.ts'),
        { path: '/ws/sub/link', name: 'link', kind: 'Symlink', children: null },
      ]),
    ]);
    const out = flattenFiles(tree);
    expect(out.map((f) => f.path)).toEqual(['/ws/a.ts', '/ws/sub/b.ts']);
  });
});

describe('truncateToBytes', () => {
  it('returns the original content when under budget', () => {
    expect(truncateToBytes('hello', 32)).toBe('hello');
  });

  it('appends a truncation marker when over budget', () => {
    const long = 'x'.repeat(1000);
    const out = truncateToBytes(long, 100);
    expect(out.endsWith('(truncated at 100 bytes)')).toBe(true);
    expect(new TextEncoder().encode(out.split('\n… ')[0]!).length).toBeLessThanOrEqual(100);
  });
});

describe('createFileResolver.list', () => {
  it('calls tree with (sessionId, workspaceRoot) and filters by query', async () => {
    const treeFn = vi.fn().mockResolvedValue(
      dirNode('/ws', 'ws', [
        fileNode('/ws/app.ts', 'app.ts'),
        fileNode('/ws/lib.ts', 'lib.ts'),
        fileNode('/ws/README.md', 'README.md'),
      ]),
    );
    const resolver = createFileResolver({
      sessionId: SESSION,
      workspaceRoot: ROOT,
      tree: treeFn,
    });
    const out = await resolver.list('lib');
    expect(treeFn).toHaveBeenCalledWith(SESSION, ROOT);
    expect(out).toEqual([
      { category: 'file', label: 'lib.ts', value: '/ws/lib.ts' },
    ]);
  });

  it('returns all files on empty query, up to the cap', async () => {
    const many = Array.from({ length: 100 }, (_, i) =>
      fileNode(`/ws/f${i}.ts`, `f${i}.ts`),
    );
    const treeFn = vi.fn().mockResolvedValue(dirNode('/ws', 'ws', many));
    const resolver = createFileResolver({
      sessionId: SESSION,
      workspaceRoot: ROOT,
      tree: treeFn,
    });
    const out = await resolver.list('');
    expect(out).toHaveLength(FILE_RESOLVER_MAX_RESULTS);
  });
});

describe('createFileResolver.resolve', () => {
  it('reads the file and returns a file ContextBlock', async () => {
    const readFn = vi.fn<(s: SessionId, p: string) => Promise<FileContent>>()
      .mockResolvedValue({
        path: '/ws/app.ts',
        content: 'export const x = 1;',
        bytes: 19,
        sha256: 'deadbeef',
      });
    const resolver = createFileResolver({
      sessionId: SESSION,
      workspaceRoot: ROOT,
      tree: vi.fn(),
      readFile: readFn,
    });
    const block = await resolver.resolve('/ws/app.ts');
    expect(readFn).toHaveBeenCalledWith(SESSION, '/ws/app.ts');
    expect(block).toEqual({
      type: 'file',
      path: '/ws/app.ts',
      content: 'export const x = 1;',
    });
  });

  it('truncates oversize file content', async () => {
    const readFn = vi.fn<(s: SessionId, p: string) => Promise<FileContent>>()
      .mockResolvedValue({
        path: '/ws/big.ts',
        content: 'x'.repeat(FILE_RESOLVER_MAX_BYTES * 2),
        bytes: FILE_RESOLVER_MAX_BYTES * 2,
        sha256: 'aa',
      });
    const resolver = createFileResolver({
      sessionId: SESSION,
      workspaceRoot: ROOT,
      tree: vi.fn(),
      readFile: readFn,
    });
    const block = await resolver.resolve('/ws/big.ts');
    expect(block.content).toMatch(/\(truncated at \d+ bytes\)$/);
  });
});
