import { describe, expect, it } from 'vitest';
import type { TreeNodeDto } from '../ipc/fs';
import { makeCandidateList, walkTree } from './helpers';
import { fuzzyMatch } from './types';

function fileNode(path: string, name: string): TreeNodeDto {
  return { path, name, kind: 'File', children: null };
}
function dirNode(path: string, name: string, children: TreeNodeDto[]): TreeNodeDto {
  return { path, name, kind: 'Dir', children };
}

describe('walkTree', () => {
  it('collects entries matching the predicate in DFS order', () => {
    const tree = dirNode('/ws', 'ws', [
      fileNode('/ws/a.ts', 'a.ts'),
      dirNode('/ws/sub', 'sub', [fileNode('/ws/sub/b.ts', 'b.ts')]),
    ]);
    const files = walkTree(
      tree,
      (n) => n.kind === 'File',
      (n) => n.path,
    );
    expect(files).toEqual(['/ws/a.ts', '/ws/sub/b.ts']);
  });

  it('accepts-all predicate emits every node', () => {
    const tree = dirNode('/ws', 'ws', [
      fileNode('/ws/a.ts', 'a.ts'),
      dirNode('/ws/sub', 'sub', [fileNode('/ws/sub/b.ts', 'b.ts')]),
    ]);
    const paths = walkTree(
      tree,
      () => true,
      (n) => n.path,
    );
    expect(paths).toEqual(['/ws', '/ws/a.ts', '/ws/sub', '/ws/sub/b.ts']);
  });

  it('skips nodes whose predicate is false but still descends', () => {
    const tree = dirNode('/ws', 'ws', [
      fileNode('/ws/a.ts', 'a.ts'),
      dirNode('/ws/sub', 'sub', [fileNode('/ws/sub/b.ts', 'b.ts')]),
    ]);
    const dirs = walkTree(
      tree,
      (n) => n.kind === 'Dir',
      (n) => n.path,
    );
    expect(dirs).toEqual(['/ws', '/ws/sub']);
  });

  it('handles nodes with null children without recursing', () => {
    const tree = fileNode('/ws/lone.ts', 'lone.ts');
    const out = walkTree(tree, () => true, (n) => n.path);
    expect(out).toEqual(['/ws/lone.ts']);
  });
});

describe('makeCandidateList', () => {
  const items = [
    { path: '/ws/app.ts', name: 'app.ts' },
    { path: '/ws/lib.ts', name: 'lib.ts' },
    { path: '/ws/README.md', name: 'README.md' },
  ];

  it('filters, slices, and maps items into Candidates', () => {
    const out = makeCandidateList({
      items,
      match: (i) => fuzzyMatch('lib', i.path),
      toCandidate: (i) => ({ category: 'file', label: i.name, value: i.path }),
    });
    expect(out).toEqual([{ category: 'file', label: 'lib.ts', value: '/ws/lib.ts' }]);
  });

  it('caps results at the provided max', () => {
    const many = Array.from({ length: 100 }, (_, i) => ({
      path: `/ws/f${i}.ts`,
      name: `f${i}.ts`,
    }));
    const out = makeCandidateList({
      items: many,
      match: () => true,
      toCandidate: (i) => ({ category: 'file', label: i.name, value: i.path }),
      max: 10,
    });
    expect(out).toHaveLength(10);
  });

  it('returns all matching items when max is omitted', () => {
    const out = makeCandidateList({
      items,
      match: () => true,
      toCandidate: (i) => ({ category: 'file', label: i.name, value: i.path }),
    });
    expect(out).toHaveLength(3);
  });

  it('passes the filtered item to toCandidate (not the raw index)', () => {
    const out = makeCandidateList({
      items,
      match: (i) => i.path.endsWith('.md'),
      toCandidate: (i) => ({ category: 'file', label: i.name, value: i.path }),
    });
    expect(out).toEqual([
      { category: 'file', label: 'README.md', value: '/ws/README.md' },
    ]);
  });
});
