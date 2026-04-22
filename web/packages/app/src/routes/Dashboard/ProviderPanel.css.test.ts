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

// F-413: the reachable (live-connected) dot must follow the provider accent
// per ai-patterns.md §Provider identity and carry the §11.3 glow. The --down
// rule keeps its error tint. Contract is asserted on the source string because
// jsdom can't resolve var() against a separate sheet.
describe('ProviderPanel health indicator — provider accent + live-connected glow (F-413)', () => {
  it('.provider-panel__health--ok paints from --provider-accent with a steel fallback', () => {
    const body = ruleBody('.provider-panel__health--ok');
    const normalised = body.replace(/\s+/g, ' ');
    expect(normalised).toContain(
      'background: var(--provider-accent, var(--color-provider-local));',
    );
  });

  it('.provider-panel__health--ok renders the §11.3 live-connected glow from the same accent', () => {
    const body = ruleBody('.provider-panel__health--ok');
    const normalised = body.replace(/\s+/g, ' ');
    expect(normalised).toContain(
      'box-shadow: 0 0 6px var(--provider-accent, var(--color-provider-local));',
    );
  });

  it('.provider-panel__health--ok no longer hardcodes --color-success (would re-introduce F-413)', () => {
    const body = ruleBody('.provider-panel__health--ok');
    expect(body).not.toMatch(/background:\s*var\(--color-success\)/);
  });

  it('.provider-panel__health--down keeps the error tint for unreachable providers', () => {
    const body = ruleBody('.provider-panel__health--down');
    const normalised = body.replace(/\s+/g, ' ');
    expect(normalised).toContain('background: var(--color-ember-400);');
    expect(normalised).toContain('box-shadow: 0 0 6px var(--color-error-border);');
  });
});
