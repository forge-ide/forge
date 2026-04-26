// F-142: resolver registry + helpers.
//
// The Composer talks to this module rather than to each resolver directly:
//
//   - `listCandidates(query)` fans out to all active resolvers and returns a
//     `Partial<Record<ContextCategory, PickerResult[]>>` shape the existing
//     ContextPicker `items` prop consumes.
//   - `resolveChips(chips, providerId)` routes each chip to its resolver,
//     collects the `ContextBlock[]`, and serializes via `adaptContextBlocks`.
//
// Registry is built from injected deps so the Composer keeps its simple
// constructor seam (tests pass stubs; production passes real stores).

import { adaptContextBlocks, type ContextBlock, type ProviderId } from '@forge/ipc';
import type {
  CategoryState,
  ContextCategory,
  PickerResult,
} from '../components/ContextPicker';
import type { Candidate, Resolver } from './types';
import { createFileResolver, type FileResolverDeps } from './file';
import { createDirectoryResolver, type DirectoryResolverDeps } from './directory';
import { createSelectionResolver, type SelectionResolverDeps } from './selection';
import { createTerminalResolver, type TerminalResolverDeps } from './terminal';
import { createAgentResolver, type AgentResolverDeps } from './agent';
import { createSkillResolver, type SkillResolverDeps } from './skill';
import { createUrlResolver, type UrlResolverDeps } from './url';

/** A chip as it lives in the composer: the picker result round-tripped. */
export interface Chip {
  category: ContextCategory;
  label: string;
  value: string;
}

export interface ResolverRegistry {
  file?: Resolver<string>;
  directory?: Resolver<string>;
  selection?: Resolver<string>;
  terminal?: Resolver<string>;
  agent?: Resolver<string>;
  skill?: Resolver<string>;
  url?: Resolver<string>;
}

export interface BuildRegistryDeps {
  file?: FileResolverDeps;
  directory?: DirectoryResolverDeps;
  selection?: SelectionResolverDeps;
  terminal?: TerminalResolverDeps;
  agent?: AgentResolverDeps;
  skill?: SkillResolverDeps;
  url?: UrlResolverDeps;
}

/** Construct a resolver registry from per-category deps. Any category whose
 *  deps are omitted is left absent — its picker tab will show "No results". */
export function buildRegistry(deps: BuildRegistryDeps): ResolverRegistry {
  const registry: ResolverRegistry = {};
  if (deps.file) registry.file = createFileResolver(deps.file);
  if (deps.directory) registry.directory = createDirectoryResolver(deps.directory);
  if (deps.selection) registry.selection = createSelectionResolver(deps.selection);
  if (deps.terminal) registry.terminal = createTerminalResolver(deps.terminal);
  if (deps.agent) registry.agent = createAgentResolver(deps.agent);
  if (deps.skill) registry.skill = createSkillResolver(deps.skill);
  if (deps.url) registry.url = createUrlResolver(deps.url);
  return registry;
}

/**
 * Fan out `list(query)` across every active resolver. Returns the shape the
 * ContextPicker's `items` prop expects. Resolvers that throw produce an
 * empty list for their tab rather than short-circuiting the whole fan-out —
 * one misbehaving source should not dismiss the picker.
 *
 * F-536: tree-backed resolvers (file, directory) expose `listStats()`. The
 * registry calls it after `list()` resolves and threads the root
 * `TreeNodeDto.stats` onto `CategoryState.stats` so the picker can render a
 * "files not shown" / "tree truncated" / "N read errors" notice. Tabs with
 * stats are kept even when the candidate list is empty, so the user still
 * sees the notice on a query that filtered every candidate out.
 */
export async function listCandidates(
  registry: ResolverRegistry,
  query: string,
): Promise<Partial<Record<ContextCategory, CategoryState>>> {
  const entries = Object.entries(registry) as Array<[
    ContextCategory,
    Resolver<string> | undefined,
  ]>;
  const settled = await Promise.all(
    entries.map(
      async ([cat, resolver]): Promise<[ContextCategory, CategoryState]> => {
        if (!resolver) return [cat, { status: 'success', items: [] }];
        try {
          const list = await resolver.list(query);
          const stats = resolver.listStats?.() ?? null;
          return [
            cat,
            {
              status: 'success',
              items: candidatesToPickerResults(list),
              stats,
            },
          ];
        } catch {
          return [cat, { status: 'success', items: [] }];
        }
      },
    ),
  );
  const out: Partial<Record<ContextCategory, CategoryState>> = {};
  for (const [cat, state] of settled) {
    if (state.status !== 'success') {
      out[cat] = state;
      continue;
    }
    const hasItems = state.items.length > 0;
    const hasStats = !!state.stats;
    if (hasItems || hasStats) out[cat] = state;
  }
  return out;
}

function candidatesToPickerResults(list: Candidate[]): PickerResult[] {
  return list.map((c) => ({
    category: c.category,
    label: c.label,
    value: c.value,
  }));
}

/**
 * Resolve every chip into a `ContextBlock` and serialize for the active
 * provider. Chips whose category has no registered resolver are dropped
 * silently — the composer should not have allowed them to be picked, but
 * dropping is safer than injecting an unresolved reference at send time.
 */
export async function resolveChips(
  registry: ResolverRegistry,
  chips: Chip[],
  provider: ProviderId | null | undefined,
): Promise<string> {
  const blocks: ContextBlock[] = [];
  for (const chip of chips) {
    const resolver = registry[chip.category];
    if (!resolver) continue;
    try {
      blocks.push(await resolver.resolve(chip.value));
    } catch {
      // Drop a failed resolve — the user's typed text still sends.
    }
  }
  return adaptContextBlocks(blocks, provider);
}
