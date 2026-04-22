// F-384: shared pipeline + tree-walk helpers for context resolvers.
//
// Every list-style resolver applies the same `filter → slice → map` pipeline
// against an in-memory candidate set, and the file/directory resolvers both
// walk the same TreeNodeDto with different predicates. These helpers unify
// those patterns so an 8th resolver plugs in without re-deriving either.
//
// Budget policy — a single unified cap is deliberately NOT imposed. The
// tree-sourced resolvers (file/directory) cap at `FILE_RESOLVER_MAX_RESULTS`
// / `DIRECTORY_RESOLVER_MAX_RESULTS` (50 each) because `tree` can return
// thousands of nodes. The registry-sourced resolvers (agent/skill) pass no
// `max` because the in-memory catalogs are bounded at the source (session
// count, skill count) and any cap here would be cosmetic.

import type { TreeNodeDto } from '../ipc/fs';
import type { Candidate } from './types';

/** Recursively collect entries from a `TreeNodeDto`. `predicate` selects
 *  which nodes are emitted; `toItem` projects a matching node into the
 *  caller's item shape. Children are always descended — a false predicate
 *  skips the node itself, not its subtree. */
export function walkTree<T>(
  node: TreeNodeDto,
  predicate: (n: TreeNodeDto) => boolean,
  toItem: (n: TreeNodeDto) => T,
  out: T[] = [],
): T[] {
  if (predicate(node)) out.push(toItem(node));
  if (node.children) {
    for (const child of node.children) walkTree(child, predicate, toItem, out);
  }
  return out;
}

export interface MakeCandidateListArgs<T> {
  items: readonly T[];
  match: (item: T) => boolean;
  toCandidate: (item: T) => Candidate;
  /** Optional ceiling; omit for unlimited. */
  max?: number;
}

/** Canonical `filter → slice → map` pipeline for candidate lists. */
export function makeCandidateList<T>(args: MakeCandidateListArgs<T>): Candidate[] {
  const matched = args.items.filter(args.match);
  const limited = args.max === undefined ? matched : matched.slice(0, args.max);
  return limited.map(args.toCandidate);
}
