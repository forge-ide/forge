// F-142: skill resolver.
//
// Skills are inserted *by reference*, not by content — the picker attaches a
// pointer and the agent loads the skill definition when it needs to. This
// keeps the prompt lean and lets the session-side skill registry choose its
// own loading shape (lazy fetch, caching, etc.).
//
// The `catalog.skills` store (seeded by the shell bridge) is the candidate
// source. `resolve` returns a pointer block — the `content` field is a
// compact `skill:<name>` reference that the receiving provider treats as a
// tool-context indicator.

import type { Candidate, ContextBlock, Resolver } from './types';
import { fuzzyMatch } from './types';
import { makeCandidateList } from './helpers';

export interface SkillCandidateSource {
  name: string;
  /** Optional one-line description. Surfaced in the candidate label. */
  description?: string;
}

export interface SkillResolverDeps {
  listSkills: () => SkillCandidateSource[];
}

export function createSkillResolver(deps: SkillResolverDeps): Resolver<string> {
  return {
    async list(query: string): Promise<Candidate[]> {
      return makeCandidateList({
        items: deps.listSkills(),
        match: (s) =>
          fuzzyMatch(query, s.name) ||
          (s.description !== undefined && fuzzyMatch(query, s.description)),
        toCandidate: (s) => ({
          category: 'skill',
          label:
            s.description !== undefined && s.description.length > 0
              ? `${s.name} — ${s.description}`
              : s.name,
          value: s.name,
        }),
      });
    },

    async resolve(name: string): Promise<ContextBlock> {
      return {
        type: 'skill',
        content: `skill:${name}`,
        meta: { skillName: name, pointer: true },
      };
    },
  };
}
