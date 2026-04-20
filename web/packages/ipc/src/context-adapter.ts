// F-142: provider-shaped serialization for resolved context blocks.
//
// The @-context picker (F-141) produces `ContextBlock[]` at send time via the
// F-142 resolvers. Different providers expect different prompt shapes:
//
//   - Anthropic messages: inline XML tags Claude's training rewards, one per
//     block, stacked above the user's typed text.
//   - OpenAI chat: function-style `[context(...)]` tool-context blocks — the
//     shape Codex / Cursor / gpt-4 system prompts conventionally use.
//
// Both serializers are pure (string-in / string-out) so they're trivially
// testable and can run equally well from the send-time code path or from a
// future server-side compactor. Shape lives in `@forge/ipc` because both the
// app package and (eventually) the shell-side IPC plumbing for `fetch_url`
// may need it; keeping it in the IPC package avoids a cycle.

import type { ProviderId } from './generated/ProviderId';

/**
 * Category of a resolved context block. Mirrors `ContextCategory` in the
 * picker (`web/packages/app/src/components/ContextPicker.tsx`) — duplicated
 * here so the IPC package has no dependency on the app package.
 */
export type ContextBlockType =
  | 'file'
  | 'directory'
  | 'selection'
  | 'terminal'
  | 'agent'
  | 'skill'
  | 'url';

/**
 * A resolved context block. Produced by a resolver's `resolve(ref)` call at
 * send time. Consumed by `adaptContextBlocks` to produce provider-specific
 * text that is prepended to the user's composer text.
 *
 * `content` is always populated (even if empty). `path` is set for blocks
 * whose location is meaningful to the model (file, directory, url) and omitted
 * for blocks whose location is ephemeral (selection, terminal, agent, skill).
 * `meta` is optional and only used by specialised serializers; the default
 * Anthropic/OpenAI shapes ignore it.
 */
export interface ContextBlock {
  type: ContextBlockType;
  /** Optional path / URL / identifier shown to the model. */
  path?: string;
  /** Textual content of the block. May be empty for pointer-style blocks. */
  content: string;
  /** Free-form extra metadata. Not serialized by default adapters. */
  meta?: Record<string, unknown>;
}

/**
 * Serialize a block as an Anthropic XML tag. Attributes are limited to `type`
 * and `path` (when present); quotes in the path are escaped — Claude is happy
 * with non-strict XML but the serializer keeps its output parseable so
 * downstream log scrapers don't trip on embedded quotes.
 *
 * Example:
 *   <context type="file" path="src/app.ts">
 *   …file contents…
 *   </context>
 */
export function toAnthropicXml(block: ContextBlock): string {
  const attrs: string[] = [`type="${escapeAttr(block.type)}"`];
  if (block.path !== undefined && block.path.length > 0) {
    attrs.push(`path="${escapeAttr(block.path)}"`);
  }
  const open = `<context ${attrs.join(' ')}>`;
  const close = `</context>`;
  return `${open}\n${block.content}\n${close}`;
}

/**
 * Serialize a block as an OpenAI-style function-context shape. The format is
 * a pseudo function call the model recognises as tool-injected context:
 *
 *   [context(type="file", path="src/app.ts")]
 *   …file contents…
 *   [/context]
 *
 * This is the convention Cursor/Continue/Codex use when lowering attached
 * files into a gpt-4-class chat turn. It is not a wire-level function-calling
 * payload — serialization stays in-text so the message shape matches the
 * Anthropic path and the Rust send path can prepend it verbatim.
 */
export function toOpenAiFunctionContext(block: ContextBlock): string {
  const attrs: string[] = [`type="${escapeAttr(block.type)}"`];
  if (block.path !== undefined && block.path.length > 0) {
    attrs.push(`path="${escapeAttr(block.path)}"`);
  }
  const open = `[context(${attrs.join(', ')})]`;
  const close = `[/context]`;
  return `${open}\n${block.content}\n${close}`;
}

function escapeAttr(value: string): string {
  return value.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

/**
 * Active provider flavour for adapter selection. `openai` covers the Codex /
 * gpt-4 family; `anthropic` covers Claude. Unknown providers fall back to the
 * Anthropic shape — it degrades better (the XML reads as text to non-Claude
 * models) than the function-call shape.
 */
export type ProviderFlavour = 'anthropic' | 'openai';

/**
 * Coerce a `ProviderId` into the flavour the adapter understands. Keeps the
 * mapping explicit and testable — the raw id is free-form (`ProviderId =
 * string`) so the caller shouldn't guess.
 */
export function providerFlavour(id: ProviderId | null | undefined): ProviderFlavour {
  if (id === null || id === undefined) return 'anthropic';
  const lower = String(id).toLowerCase();
  if (lower.includes('openai')) return 'openai';
  if (lower.includes('anthropic') || lower.includes('claude')) return 'anthropic';
  // OpenAI-compatible endpoints often use names like `groq`, `together`,
  // `mistral`, `ollama` — fall back to the OpenAI shape for those since they
  // expose the same chat-completions contract.
  if (
    lower.includes('groq') ||
    lower.includes('together') ||
    lower.includes('mistral') ||
    lower.includes('ollama') ||
    lower.includes('deepseek') ||
    lower.includes('gpt')
  ) {
    return 'openai';
  }
  return 'anthropic';
}

/**
 * Serialize a list of resolved context blocks into a single string, shaped
 * for the active provider. Returns an empty string when `blocks` is empty —
 * callers can prepend the result unconditionally without needing a guard.
 */
export function adaptContextBlocks(
  blocks: ContextBlock[],
  provider: ProviderId | ProviderFlavour | null | undefined,
): string {
  if (blocks.length === 0) return '';
  const flavour =
    provider === 'anthropic' || provider === 'openai'
      ? provider
      : providerFlavour(provider);
  const serialize = flavour === 'openai' ? toOpenAiFunctionContext : toAnthropicXml;
  return blocks.map(serialize).join('\n');
}
