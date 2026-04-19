import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// SessionsPanel token adherence (F-090):
// voice-terminology.md §9 agent rule #1 — "Use design tokens, never raw hex
// or pixel values." `.session-card__badge--persist` previously declared
// raw `rgba(255, 74, 18, 0.08)` for its background and `rgba(255, 74, 18, 0.22)`
// for its border. After F-090 these must use `var(--color-error-bg)` and
// `var(--color-error-border)` (which already encode the same channels at the
// canonical 0.07/0.22 alpha). JSDOM cannot reliably resolve var() at the
// source level, so assert the source-string contract on the CSS rule block
// (matches the F-084/F-085 pattern).

const cssPath = resolve(__dirname, 'SessionsPanel.css');
const css = readFileSync(cssPath, 'utf-8');

function ruleBody(selector: string): string {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`selector not found in SessionsPanel.css: ${selector}`);
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

describe('SessionsPanel — design-token adherence (F-090)', () => {
  const body = ruleBody('.session-card__badge--persist');

  it('uses var(--color-error-bg) for background, not raw rgba', () => {
    expect(hasDecl(body, 'background', 'var(--color-error-bg)')).toBe(true);
  });

  it('uses var(--color-error-border) for border-color, not raw rgba', () => {
    expect(hasDecl(body, 'border-color', 'var(--color-error-border)')).toBe(true);
  });

  it('no longer contains the raw rgba(255, 74, 18, 0.08) literal', () => {
    expect(body).not.toMatch(/rgba\(\s*255\s*,\s*74\s*,\s*18\s*,\s*0\.08\s*\)/);
  });

  it('no longer contains the raw rgba(255, 74, 18, 0.22) literal', () => {
    expect(body).not.toMatch(/rgba\(\s*255\s*,\s*74\s*,\s*18\s*,\s*0\.22\s*\)/);
  });
});
