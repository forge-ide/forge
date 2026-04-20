// F-142: selection resolver.
//
// The active editor's current selection is supplied by F-122's EditorPane
// (future integration) or an injected provider for tests today. The picker
// surfaces at most one entry — "current selection" — whose label reflects
// the active file and line range. Returns `[]` when there is no selection
// to keep the tab coherent with the spec ("otherwise absent").
//
// No IPC call is involved. The selection snapshot lives entirely in the
// webview — EditorPane (or a future selection bus) owns it.

import type { Candidate, ContextBlock, Resolver } from './types';

export interface SelectionSnapshot {
  /** Absolute path of the file the selection was taken from. */
  path: string;
  /** 1-based inclusive start line. */
  startLine: number;
  /** 1-based inclusive end line. */
  endLine: number;
  /** The selected text (already sliced by the editor). */
  text: string;
}

export interface SelectionResolverDeps {
  /** Returns the current selection snapshot, or `null` when none. */
  getSelection: () => SelectionSnapshot | null;
}

/**
 * Selection refs are the snapshot itself, serialized as JSON in the picker's
 * `value` field. This keeps `resolve` self-contained — the snapshot at
 * list-time is the one the user sees, even if the editor's live selection
 * changes between @-pick and send.
 */
export function createSelectionResolver(deps: SelectionResolverDeps): Resolver<string> {
  return {
    async list(_query: string): Promise<Candidate[]> {
      const snap = deps.getSelection();
      if (!snap) return [];
      const leaf = snap.path.split('/').pop() ?? snap.path;
      return [
        {
          category: 'selection' as const,
          label: `${leaf} @ ln ${snap.startLine}-${snap.endLine}`,
          value: JSON.stringify(snap),
        },
      ];
    },

    async resolve(ref: string): Promise<ContextBlock> {
      const snap = JSON.parse(ref) as SelectionSnapshot;
      return {
        type: 'selection',
        path: snap.path,
        content: snap.text,
        meta: { startLine: snap.startLine, endLine: snap.endLine },
      };
    },
  };
}
