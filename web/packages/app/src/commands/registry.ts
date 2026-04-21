// F-157: command-palette registry.
//
// A plain module with a module-scoped Map keyed by command id. Components
// that want to expose an action call `registerCommand({ id, title, run })`
// and get back a disposer. The palette component reads the live list via
// `listCommands()` and filters it with `filterCommandsByQuery(query)`.
//
// `fuzzyMatch` is the scorer backing the filter: case-insensitive
// subsequence match with a small prefix/contiguity bonus so obvious matches
// rank first. The scorer is pure and tested directly.

export interface Command {
  /** Stable unique identifier. Re-registering the same id replaces the entry. */
  id: string;
  /** Human-readable title shown in the palette. */
  title: string;
  /** Invoked when the user selects the command. */
  run: () => void;
}

const commands = new Map<string, Command>();

/** Register a command. Returns a disposer that unregisters it. */
export function registerCommand(cmd: Command): () => void {
  commands.set(cmd.id, cmd);
  return () => {
    unregisterCommand(cmd.id);
  };
}

/** Remove the command with the given id. Returns `true` if one was removed. */
export function unregisterCommand(id: string): boolean {
  return commands.delete(id);
}

/** Current list of commands in registration order. */
export function listCommands(): Command[] {
  return Array.from(commands.values());
}

/** Clear the registry. Exposed for tests; do not call from app code. */
export function __resetRegistryForTests(): void {
  commands.clear();
}

// ---------------------------------------------------------------------------
// Fuzzy match
// ---------------------------------------------------------------------------

export interface FuzzyMatchResult {
  matched: true;
  /** Higher = better match. */
  score: number;
  /** Character indices in the target that matched the query, in order. */
  indices: number[];
}

/**
 * Case-insensitive subsequence scorer. Returns `null` when the query chars
 * cannot be found in order within the target. Returns a score otherwise —
 * higher means a better match, with the following bonuses:
 *   +100 per query char matched
 *   +50  when the first match is at index 0 (prefix match)
 *   +10  per contiguous adjacent match (tightly clustered matches win)
 *   -1   per gap between matches (penalty for scattered matches)
 */
export function fuzzyMatch(query: string, target: string): FuzzyMatchResult | null {
  if (query.length === 0) {
    return { matched: true, score: 0, indices: [] };
  }
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  const indices: number[] = [];
  let qi = 0;
  for (let ti = 0; ti < t.length && qi < q.length; ti += 1) {
    if (t[ti] === q[qi]) {
      indices.push(ti);
      qi += 1;
    }
  }
  if (qi < q.length) return null;

  let score = indices.length * 100;
  if (indices[0] === 0) score += 50;
  for (let i = 1; i < indices.length; i += 1) {
    const gap = indices[i]! - indices[i - 1]! - 1;
    if (gap === 0) score += 10;
    else score -= gap;
  }
  return { matched: true, score, indices };
}

/**
 * Filter the live command list against a query and return matches sorted by
 * descending score. An empty query returns all commands in registration
 * order.
 */
export function filterCommandsByQuery(query: string): Command[] {
  const cmds = listCommands();
  if (query.trim().length === 0) return cmds;
  const scored: Array<{ cmd: Command; score: number; rank: number }> = [];
  for (let i = 0; i < cmds.length; i += 1) {
    const cmd = cmds[i]!;
    const m = fuzzyMatch(query, cmd.title);
    if (m) scored.push({ cmd, score: m.score, rank: i });
  }
  scored.sort((a, b) => b.score - a.score || a.rank - b.rank);
  return scored.map((s) => s.cmd);
}
