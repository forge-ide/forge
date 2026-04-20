// F-142: directory resolver.
//
// `list(query)` shares the tree IPC with the file resolver but emits directory
// nodes instead. `resolve(ref)` inserts a tree snapshot — paths only, no file
// contents, capped at 200 entries per the spec (§7.4 "max 200 paths in v1").

import type { SessionId } from '@forge/ipc';
import {
  tree as defaultTree,
  type TreeNodeDto,
} from '../ipc/fs';
import type { Candidate, ContextBlock, Resolver } from './types';
import { fuzzyMatch } from './types';

/** Spec-mandated cap: directory snapshots include at most this many paths. */
export const DIRECTORY_RESOLVER_MAX_PATHS = 200;

/** UI-side paging for the candidate list. */
export const DIRECTORY_RESOLVER_MAX_RESULTS = 50;

export interface DirectoryResolverDeps {
  sessionId: SessionId;
  workspaceRoot: string;
  tree?: typeof defaultTree;
}

/** Emit `[path, name]` pairs for directory nodes only (root included). */
export function flattenDirectories(
  node: TreeNodeDto,
  out: Array<{ path: string; name: string }> = [],
): Array<{ path: string; name: string }> {
  // `kind` is the Rust-side enum 'File' | 'Dir' | 'Symlink' | 'Other'.
  if (node.kind === 'Dir') {
    out.push({ path: node.path, name: node.name });
  }
  if (node.children) {
    for (const child of node.children) flattenDirectories(child, out);
  }
  return out;
}

/**
 * Flatten every path (file or directory) beneath `node`. Used by `resolve` to
 * produce the 200-path snapshot. The cap is applied by the caller so the
 * helper stays generic.
 */
export function flattenAllPaths(
  node: TreeNodeDto,
  out: string[] = [],
): string[] {
  out.push(node.path);
  if (node.children) {
    for (const child of node.children) flattenAllPaths(child, out);
  }
  return out;
}

export function createDirectoryResolver(deps: DirectoryResolverDeps): Resolver<string> {
  const treeFn = deps.tree ?? defaultTree;

  return {
    async list(query: string): Promise<Candidate[]> {
      const node = await treeFn(deps.sessionId, deps.workspaceRoot);
      const dirs = flattenDirectories(node);
      const matched = dirs.filter((d) => fuzzyMatch(query, d.path));
      return matched.slice(0, DIRECTORY_RESOLVER_MAX_RESULTS).map((d) => ({
        category: 'directory' as const,
        label: d.name || d.path,
        value: d.path,
      }));
    },

    async resolve(dirPath: string): Promise<ContextBlock> {
      // Walk from the picked directory specifically — not the workspace root —
      // so the snapshot reflects the directory the user @-mentioned.
      const node = await treeFn(deps.sessionId, dirPath);
      const all = flattenAllPaths(node);
      const truncated = all.length > DIRECTORY_RESOLVER_MAX_PATHS;
      const shown = all.slice(0, DIRECTORY_RESOLVER_MAX_PATHS);
      const body =
        shown.join('\n') +
        (truncated
          ? `\n… (+${all.length - DIRECTORY_RESOLVER_MAX_PATHS} more — truncated)`
          : '');
      return {
        type: 'directory',
        path: dirPath,
        content: body,
      };
    },
  };
}
