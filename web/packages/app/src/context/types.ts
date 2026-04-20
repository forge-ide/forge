// F-142: shared resolver contracts.
//
// Each category under `context/` exports a module with two functions:
//
//   list(query) -> Promise<Candidate[]>      // populate picker results
//   resolve(ref) -> Promise<ContextBlock>    // materialize at send time
//
// `Candidate` is the shape the picker already consumes via `PickerResult`.
// The `ref` type is category-specific — we keep each resolver narrowly
// typed and rely on the picker's `value` field (round-tripped as an opaque
// string) to transport the category-specific identifier at send time.
//
// Keeping this file dependency-light: no imports from the picker component
// (avoids pulling solid-js into otherwise pure resolver modules).

import type { ContextBlock } from '@forge/ipc';
import type { ContextCategory } from '../components/ContextPicker';

export type { ContextBlock };

/** A single picker result. Matches `PickerResult` in the picker component. */
export interface Candidate {
  category: ContextCategory;
  label: string;
  value: string;
}

/**
 * Category resolver shape. `TRef` is the category-specific reference the
 * resolver needs at send time. Each concrete resolver narrows this type —
 * callers typically go through the picker (`value: string`) and route to
 * the right resolver by chip category.
 */
export interface Resolver<TRef = string> {
  list(query: string): Promise<Candidate[]>;
  resolve(ref: TRef): Promise<ContextBlock>;
}

/**
 * Substring-match case-insensitive. Common helper for candidate list filters.
 * Empty query matches everything.
 */
export function fuzzyMatch(query: string, value: string): boolean {
  if (!query) return true;
  return value.toLowerCase().includes(query.toLowerCase());
}
