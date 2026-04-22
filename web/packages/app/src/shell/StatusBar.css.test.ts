// F-427 drift guard + F-392 ember-400 spec-lock for StatusBar.css.
//
// The file previously referenced a `--fg-*` token namespace that was never
// declared in `web/packages/design/src/tokens.css` or
// `docs/design/token-reference.md`, with raw-hex fallbacks bypassing the
// token enforcement invariant documented in `docs/design/color-system.md`.
// These tests fail fast if either regression returns — any custom property
// used in the file must belong to the canonical `--color-*` / `--sp-*` /
// `--r-*` / `--font-*` / `--ease` namespace, and no raw color fallbacks
// may be supplied to `var(...)`.
//
// F-392 additionally pins the ember-400 spec-lock from
// `docs/design/component-principles.md §Status bar` and `shell.md §2`:
// "The status bar is always ember-400 background with white text … Do not
// change the status bar color under any circumstances." The `.status-bar`
// root rule must set `background: var(--color-ember-400)` and must NOT
// reintroduce any `--color-surface-*` or `--fg-surface-*` fallback on the
// background. Mirrors the `ActivityBar.css.test.ts` targeted-rule-match
// pattern (PR #469).
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

describe('StatusBar.css ember-400 spec-lock (F-392)', () => {
  // Targeted-rule-match pattern mirroring ActivityBar.css.test.ts (PR #469):
  // parse the `.status-bar { ... }` root rule body directly and assert against
  // it. jsdom does not resolve external stylesheets at render time, so the
  // CSS source file is the contract.
  const rootMatch = source.match(/\.status-bar\s*\{([^}]*)\}/);

  it('declares a root `.status-bar` rule', () => {
    expect(rootMatch).not.toBeNull();
  });

  it('sets background to --color-ember-400 (spec-lock)', () => {
    const body = rootMatch?.[1] ?? '';
    expect(body).toMatch(/background\s*:\s*var\(--color-ember-400\)/);
  });

  it('does not fall back to a neutral surface token on background', () => {
    const body = rootMatch?.[1] ?? '';
    // No --color-surface-*, --fg-surface-*, or --color-bg anywhere in the
    // root rule — the ember background must be unconditional.
    expect(body).not.toMatch(/--color-surface-/);
    expect(body).not.toMatch(/--fg-surface-/);
    expect(body).not.toMatch(/--color-bg\b/);
  });

  it('uses --color-text-inverted (white) for the bar text color', () => {
    const body = rootMatch?.[1] ?? '';
    expect(body).toMatch(/color\s*:\s*var\(--color-text-inverted\)/);
  });

  it('does not reintroduce a neutral top border (redundant on ember)', () => {
    const body = rootMatch?.[1] ?? '';
    expect(body).not.toMatch(/border-top\s*:/);
  });
});
