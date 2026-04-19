import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// ApprovalPrompt button :active transform (F-085):
// component-principles.md "Buttons" — "Active state always includes
// `transform: translateY(1px)`." F-027 shipped two button variants without
// this rule. Both must declare it on their `:active` selector. JSDOM cannot
// reliably compute pseudo-class styles, so assert the source-string contract
// on the CSS rule blocks themselves (matches the F-084 pattern in
// ApprovalPrompt.css.test.ts).

const cssPath = resolve(__dirname, 'ApprovalPrompt.css');
const css = readFileSync(cssPath, 'utf-8');

/**
 * Extract the declarations inside the first rule block matching `selector`.
 * Matches the helper in ApprovalPrompt.css.test.ts.
 */
function ruleBody(selector: string): string {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`selector not found in ApprovalPrompt.css: ${selector}`);
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

describe.each([
  '.approval-prompt__btn--primary:active',
  '.approval-prompt__btn--ghost:active',
])('ApprovalPrompt active state — %s', (selector) => {
  it(`declares transform: translateY(1px)`, () => {
    const body = ruleBody(selector);
    expect(hasDecl(body, 'transform', 'translateY(1px)')).toBe(true);
  });
});
