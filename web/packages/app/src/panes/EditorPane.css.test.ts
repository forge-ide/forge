import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// F-417: dirty-dot indicator must remain visually distinctive against
// `--color-surface-1`. Previously `.editor-pane__dirty-dot` used
// `background: var(--color-accent-warn, var(--color-text-primary))` — the
// warn token was undefined, so the fallback resolved to near-white text
// primary, indistinguishable from any adjacent text against the dark
// surface. `--color-accent-warn` is now a first-class ember-100 token in
// tokens.css + token-reference.md; the fallback is removed so the dot
// always paints the ember accent.
//
// Source-string assertions mirror the F-090 / F-084 pattern
// (ProviderPanel.css.test.ts) — JSDOM cannot reliably resolve var()
// chains at the source level.

const cssPath = resolve(__dirname, 'EditorPane.css');
const css = readFileSync(cssPath, 'utf-8');

const tokensCssPath = resolve(
  __dirname,
  '../../../../packages/design/src/tokens.css',
);
const tokensCss = readFileSync(tokensCssPath, 'utf-8');

const tokenRefPath = resolve(
  __dirname,
  '../../../../../docs/design/token-reference.md',
);
const tokenRef = readFileSync(tokenRefPath, 'utf-8');

function ruleBody(selector: string): string {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`selector not found in EditorPane.css: ${selector}`);
  const open = css.indexOf('{', at);
  const close = css.indexOf('}', open);
  if (open < 0 || close < 0) {
    throw new Error(`malformed rule block for selector: ${selector}`);
  }
  return css.slice(open + 1, close).trim();
}

describe('EditorPane dirty-dot — F-417 fallback no longer defeats the signal', () => {
  it('.editor-pane__dirty-dot uses --color-accent-warn with no text-primary fallback', () => {
    const body = ruleBody('.editor-pane__dirty-dot');
    const normalised = body.replace(/\s+/g, ' ');
    expect(normalised).toContain('background: var(--color-accent-warn);');
    expect(normalised).not.toMatch(/var\(--color-accent-warn,\s*var\(--color-text-primary\)\)/);
  });

  it('tokens.css defines --color-accent-warn as an ember-100 alias', () => {
    expect(tokensCss).toMatch(/--color-accent-warn:\s*var\(--color-ember-100\);/);
  });

  it('docs/design/token-reference.md declares --color-accent-warn alongside tokens.css', () => {
    expect(tokenRef).toMatch(/--color-accent-warn:\s*var\(--color-ember-100\);/);
  });
});
