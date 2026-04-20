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
import type { ContextCategory, PickerResult } from '../components/ContextPicker';
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
 */
export async function listCandidates(
  registry: ResolverRegistry,
  query: string,
): Promise<Partial<Record<ContextCategory, PickerResult[]>>> {
  const entries = Object.entries(registry) as Array<[
    ContextCategory,
    Resolver<string> | undefined,
  ]>;
  const settled = await Promise.all(
    entries.map(async ([cat, resolver]): Promise<[ContextCategory, PickerResult[]]> => {
      if (!resolver) return [cat, []];
      try {
        const list = await resolver.list(query);
        return [cat, candidatesToPickerResults(list)];
      } catch {
        return [cat, []];
      }
    }),
  );
  const out: Partial<Record<ContextCategory, PickerResult[]>> = {};
  for (const [cat, list] of settled) {
    if (list.length > 0) out[cat] = list;
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
