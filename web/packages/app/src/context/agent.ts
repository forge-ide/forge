// F-142: agent resolver.
//
// The spec (§7.6) says an agent reference should insert "a summary + inline
// references, not full copy" of the other agent's transcript. The full
// compaction pipeline lives in forge-session (not shipped here); the resolver
// therefore accepts an injected `getTranscript(sessionId)` closure and a
// `summarize(turns)` helper. For v1 the default summarizer takes the last
// 20 turns' first 120 chars each — terse, but enough to prove the shape.
//
// Candidates come from the `sessions` store (populated by the shell bridge).
// Filtering on `query` is substring-match on the session id or label.

import type { SessionId } from '@forge/ipc';
import type { Candidate, ContextBlock, Resolver } from './types';
import { fuzzyMatch } from './types';

export interface AgentCandidateSource {
  id: SessionId;
  /** Human-readable label. Falls back to the session id. */
  label?: string;
}

/** Simple transcript shape — each entry is a role-tagged line. */
export interface TranscriptEntry {
  role: 'user' | 'assistant';
  text: string;
}

export interface AgentResolverDeps {
  /** Returns currently-known agent sessions (excluding the caller). */
  listAgents: () => AgentCandidateSource[];
  /** Fetches a transcript. Default never-available — inject for tests. */
  getTranscript?: (id: SessionId) => Promise<TranscriptEntry[]>;
}

/**
 * Default transcript summarizer — last 20 turns, each trimmed to 120 chars.
 * Exported so tests can drive the summarizer directly without an IPC stub.
 */
export function summarizeTranscript(entries: TranscriptEntry[]): string {
  const MAX_TURNS = 20;
  const MAX_CHARS = 120;
  const recent = entries.slice(-MAX_TURNS);
  return recent
    .map((e) => {
      const truncated =
        e.text.length > MAX_CHARS
          ? `${e.text.slice(0, MAX_CHARS - 1)}…`
          : e.text;
      return `${e.role}: ${truncated}`;
    })
    .join('\n');
}

export function createAgentResolver(deps: AgentResolverDeps): Resolver<string> {
  const getTranscript =
    deps.getTranscript ?? (async (_id: SessionId) => [] as TranscriptEntry[]);

  return {
    async list(query: string): Promise<Candidate[]> {
      const all = deps.listAgents();
      const withLabels = all.map((a) => ({
        id: a.id,
        label: a.label ?? String(a.id),
      }));
      const matched = withLabels.filter(
        (a) => fuzzyMatch(query, a.label) || fuzzyMatch(query, String(a.id)),
      );
      return matched.map((a) => ({
        category: 'agent' as const,
        label: a.label,
        value: String(a.id),
      }));
    },

    async resolve(id: string): Promise<ContextBlock> {
      const entries = await getTranscript(id as SessionId);
      return {
        type: 'agent',
        content: summarizeTranscript(entries),
        meta: { agentSessionId: id },
      };
    },
  };
}
