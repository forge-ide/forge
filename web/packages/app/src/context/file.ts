// F-142: file resolver.
//
// `list(query)` walks the session's workspace via the F-122 `tree` IPC, flattens
// the result to file-only entries, and applies a substring match against
// `query`. The shell enforces gitignore semantics inside `forge-fs::tree` so
// the webview does not re-implement that filter.
//
// `resolve(ref)` reads the file via `read_file`. `ref` is the absolute path
// round-tripped through the picker result's `value`. File contents are
// truncated to a byte budget so a single @-file cannot dominate the prompt.

import type { SessionId } from '@forge/ipc';
import {
  readFile as defaultReadFile,
  tree as defaultTree,
  type TreeNodeDto,
  type FileContent,
} from '../ipc/fs';
import type { Candidate, ContextBlock, Resolver } from './types';
import { fuzzyMatch } from './types';

/** Maximum bytes included in a resolved file block. Larger files are
 *  truncated with a trailing marker so the model can see the cut happened. */
export const FILE_RESOLVER_MAX_BYTES = 32 * 1024;

/** Ceiling on candidates surfaced to the picker. The tree IPC already caps
 *  entries server-side; this is a UI-side paging guard. */
export const FILE_RESOLVER_MAX_RESULTS = 50;

export interface FileResolverDeps {
  sessionId: SessionId;
  /** Workspace root path. Pass the session's cached root (`activeWorkspaceRoot`). */
  workspaceRoot: string;
  /** Injection seam — defaults to the real Tauri invoker. */
  tree?: typeof defaultTree;
  readFile?: typeof defaultReadFile;
}

/**
 * Flatten a tree node into `[path, name]` pairs for file entries only.
 * Directories are descended but not emitted. Exported for test visibility.
 */
export function flattenFiles(
  node: TreeNodeDto,
  out: Array<{ path: string; name: string }> = [],
): Array<{ path: string; name: string }> {
  // `kind` is the Rust-side enum: 'File' | 'Dir' | 'Symlink' | 'Other'. Only
  // plain files go into the picker — symlinks and special files are skipped
  // so the resolver cannot accidentally tunnel outside the gitignore filter.
  if (node.kind === 'File') {
    out.push({ path: node.path, name: node.name });
  }
  if (node.children) {
    for (const child of node.children) flattenFiles(child, out);
  }
  return out;
}

/** Truncate content to `maxBytes` UTF-8 bytes. Appends a visible marker when
 *  truncation occurs so the model can see the cut. */
export function truncateToBytes(content: string, maxBytes: number): string {
  const encoded = new TextEncoder().encode(content);
  if (encoded.length <= maxBytes) return content;
  const decoder = new TextDecoder();
  // Truncate at a codepoint boundary by passing `{ stream: false }` and
  // slicing enough bytes to give the decoder room to resync on invalid
  // trailing bytes.
  const slice = encoded.slice(0, maxBytes);
  const truncated = decoder.decode(slice, { stream: false });
  return `${truncated}\n… (truncated at ${maxBytes} bytes)`;
}

export function createFileResolver(deps: FileResolverDeps): Resolver<string> {
  const treeFn = deps.tree ?? defaultTree;
  const readFn = deps.readFile ?? defaultReadFile;

  return {
    async list(query: string): Promise<Candidate[]> {
      const node = await treeFn(deps.sessionId, deps.workspaceRoot);
      const files = flattenFiles(node);
      const matched = files.filter((f) => fuzzyMatch(query, f.path));
      return matched.slice(0, FILE_RESOLVER_MAX_RESULTS).map((f) => ({
        category: 'file' as const,
        label: f.name,
        value: f.path,
      }));
    },

    async resolve(path: string): Promise<ContextBlock> {
      const file: FileContent = await readFn(deps.sessionId, path);
      const content = truncateToBytes(file.content, FILE_RESOLVER_MAX_BYTES);
      return {
        type: 'file',
        path,
        content,
      };
    },
  };
}
