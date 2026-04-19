import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// ApprovalPrompt button typography (F-084):
// component-principles.md "Buttons" — "All buttons use Barlow Condensed 700,
// uppercase, letter-spacing: 0.1em." Both `.approval-prompt__btn` and
// `.approval-prompt__menu-item` were declared with `--font-mono` and no
// weight/transform/letter-spacing. Assert source-string remediation in the
// CSS rule blocks; jsdom computed-style assertions are unreliable.

const cssPath = resolve(__dirname, 'ApprovalPrompt.css');
const css = readFileSync(cssPath, 'utf-8');

/**
 * Extract the declarations inside the first rule block matching `selector`.
 * Returns the body string (between `{` and `}`), trimmed.
 *
 * We split on the literal selector text rather than building a regex; CSS
 * selectors contain dots and dashes that are awkward to escape and we don't
 * need any regex matching here — a literal substring locates the rule.
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

const REQUIRED_DECLS: Array<{ name: string; value: string }> = [
  { name: 'font-family', value: 'var(--font-display)' },
  { name: 'font-weight', value: '700' },
  { name: 'text-transform', value: 'uppercase' },
  { name: 'letter-spacing', value: '0.1em' },
];

/** Returns true if `body` contains a declaration matching `name: value;`
 *  (whitespace-tolerant). Substring match avoids regex-escape pitfalls. */
function hasDecl(body: string, name: string, value: string): boolean {
  // Normalise: collapse runs of whitespace, then look for "name: value;".
  const normalised = body.replace(/\s+/g, ' ');
  return normalised.includes(`${name}: ${value};`);
}

describe.each(['.approval-prompt__btn', '.approval-prompt__menu-item'])(
  'ApprovalPrompt button typography — %s',
  (selector) => {
    const body = ruleBody(selector);

    for (const decl of REQUIRED_DECLS) {
      it(`declares ${decl.name}: ${decl.value}`, () => {
        expect(hasDecl(body, decl.name, decl.value)).toBe(true);
      });
    }

    it('no longer uses --font-mono (would re-introduce the F-084 regression)', () => {
      expect(hasDecl(body, 'font-family', 'var(--font-mono)')).toBe(false);
    });
  },
);
