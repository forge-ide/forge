import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// PaneHeader provider pill (F-091): per ai-patterns.md §7 the pill color must
// vary by provider. The implementation plumbs an inline CSS custom property
// (`--pane-header-provider-accent`) and the rule reads it with the steel
// `--color-provider-local` token as a Phase-1 default. Source-string asserts
// guard the contract — jsdom can't resolve var() against an external sheet,
// and a regression to the old hardcoded value would silently un-color other
// providers in production.

const cssPath = resolve(__dirname, 'PaneHeader.css');
const css = readFileSync(cssPath, 'utf-8');

function ruleBody(selector: string): string {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`selector not found in PaneHeader.css: ${selector}`);
  const open = css.indexOf('{', at);
  const close = css.indexOf('}', open);
  if (open < 0 || close < 0) {
    throw new Error(`malformed rule block for selector: ${selector}`);
  }
  return css.slice(open + 1, close).trim();
}

describe('PaneHeader.css `.pane-header__provider` (F-091)', () => {
  const body = ruleBody('.pane-header__provider');

  it('reads the per-pane accent from a CSS custom property with a steel fallback', () => {
    const normalised = body.replace(/\s+/g, ' ');
    expect(normalised).toContain(
      'color: var(--pane-header-provider-accent, var(--color-provider-local));',
    );
  });

  it('no longer hardcodes `color: var(--color-provider-local)` (would re-introduce F-091)', () => {
    const normalised = body.replace(/\s+/g, ' ');
    // Bare `color: var(--color-provider-local);` (without the custom-property
    // fallback wrapper) is the pre-F-091 regression shape — block it.
    expect(normalised).not.toMatch(/color: var\(--color-provider-local\);/);
  });
});
