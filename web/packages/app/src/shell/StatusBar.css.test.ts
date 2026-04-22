// F-427 drift guard for StatusBar.css.
//
// The file previously referenced a `--fg-*` token namespace that was never
// declared in `web/packages/design/src/tokens.css` or
// `docs/design/token-reference.md`, with raw-hex fallbacks bypassing the
// token enforcement invariant documented in `docs/design/color-system.md`.
// These tests fail fast if either regression returns — any custom property
// used in the file must belong to the canonical `--color-*` / `--sp-*` /
// `--r-*` / `--font-*` / `--ease` namespace, and no raw color fallbacks
// may be supplied to `var(...)`.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const cssPath = resolve(__dirname, 'StatusBar.css');
const source = readFileSync(cssPath, 'utf-8');

const CANONICAL_PREFIXES = ['--color-', '--sp-', '--r-', '--font-', '--ease'];

interface VarCall {
  name: string;
  fallback: string | null;
  raw: string;
}

/**
 * Extract every `var(--name[, fallback])` call from the CSS source.
 * Handles nested `var(..., var(...))` by walking parentheses.
 */
function collectVarCalls(src: string): VarCall[] {
  const calls: VarCall[] = [];
  const re = /var\(\s*(--[a-zA-Z0-9_-]+)\s*(,([^)]|\([^)]*\))*)?\)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(src)) !== null) {
    const name = m[1] ?? '';
    const fallback = m[2] ? m[2].replace(/^,\s*/, '').trim() : null;
    calls.push({ name, fallback, raw: m[0] });
  }
  return calls;
}

describe('StatusBar.css token hygiene (F-427)', () => {
  const calls = collectVarCalls(source);

  it('references at least one custom property (sanity check)', () => {
    expect(calls.length).toBeGreaterThan(0);
  });

  it('uses only canonical design tokens — no --fg-* or other ad-hoc namespaces', () => {
    const offenders = calls
      .filter(({ name }) => !CANONICAL_PREFIXES.some((p) => name.startsWith(p)))
      .map(({ raw }) => raw);
    expect(offenders).toEqual([]);
  });

  it('does not supply raw-hex or rgb/rgba fallbacks to var()', () => {
    const rawValue = /^(#[0-9a-fA-F]{3,8}|rgba?\()/;
    const offenders = calls
      .filter(({ fallback }) => fallback !== null && rawValue.test(fallback!))
      .map(({ raw }) => raw);
    expect(offenders).toEqual([]);
  });
});
