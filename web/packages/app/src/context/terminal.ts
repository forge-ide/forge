// F-142: terminal resolver.
//
// The spec calls for the "last N lines of the focused terminal pane". F-125
// owns the xterm.js byte buffer; F-142 does not tap into it directly — the
// resolver instead accepts an injected `getTerminalSnapshot()` closure, which
// the ChatPane wires to the focused TerminalPane at mount time (or supplies
// a null provider when no terminal is mounted).
//
// This keeps the resolver pure and the coupling to F-125 one-directional:
// TerminalPane exposes a `snapshot()` method upstream, not downstream.

import type { Candidate, ContextBlock, Resolver } from './types';

export interface TerminalSnapshot {
  /** Identifier of the focused terminal (for labelling only). */
  terminalId: string;
  /** Display name: usually the shell path's final segment (`zsh`, `bash`). */
  shellName: string;
  /** Lines already joined with `\n`. The picker labels the count. */
  text: string;
  /** Number of lines `text` covers — used in the candidate label. */
  lineCount: number;
}

export interface TerminalResolverDeps {
  /** Returns the focused terminal's recent output, or `null` when none. */
  getSnapshot: () => TerminalSnapshot | null;
}

export function createTerminalResolver(deps: TerminalResolverDeps): Resolver<string> {
  return {
    async list(_query: string): Promise<Candidate[]> {
      const snap = deps.getSnapshot();
      if (!snap) return [];
      return [
        {
          category: 'terminal' as const,
          label: `${snap.shellName} — last ${snap.lineCount} lines`,
          value: JSON.stringify(snap),
        },
      ];
    },

    async resolve(ref: string): Promise<ContextBlock> {
      const snap = JSON.parse(ref) as TerminalSnapshot;
      return {
        type: 'terminal',
        content: snap.text,
        meta: { terminalId: snap.terminalId, shellName: snap.shellName },
      };
    },
  };
}
