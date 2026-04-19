import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// ProviderPanel token adherence (F-090):
// voice-terminology.md §9 agent rule #1 — "Use design tokens, never raw hex
// or pixel values." `.provider-panel__btn--primary` previously declared
// `color: #fff` (raw hex). After F-090 it must use `var(--color-text-inverted)`,
// a new token added to `tokens.css` and `token-reference.md`. JSDOM cannot
// reliably resolve var() at the source level, so assert the source-string
// contract on the CSS rule block (matches F-084/F-085 pattern).

const cssPath = resolve(__dirname, 'ProviderPanel.css');
const css = readFileSync(cssPath, 'utf-8');

function ruleBody(selector: string): string {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`selector not found in ProviderPanel.css: ${selector}`);
  const open = css.indexOf('{', at);
  const close = css.indexOf('}', open);
  if (open < 0 || close < 0) {
    throw new Error(`malformed rule block for selector: ${selector}`);
  }
  return css.slice(open + 1, close).trim();
}

function hasDecl(body: string, name: string, value: string): boolean {
  const normalised = body.replace(/\s+/g, ' ');
  return normalised.includes(`${name}: ${value};`);
}

describe('ProviderPanel — design-token adherence (F-090)', () => {
  it('.provider-panel__btn--primary uses --color-text-inverted, not raw #fff', () => {
    const body = ruleBody('.provider-panel__btn--primary');
    expect(hasDecl(body, 'color', 'var(--color-text-inverted)')).toBe(true);
    expect(body).not.toMatch(/color:\s*#fff\b/i);
    expect(body).not.toMatch(/color:\s*#ffffff\b/i);
  });
});
