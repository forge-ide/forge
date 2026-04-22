import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// BranchGutter thread-line token (F-145 / F-418):
// docs/ui-specs/branching.md §15.4 and the BranchGutter.tsx docstring both
// assert the 2px vertical line uses `--color-ember-300`. Guard the CSS
// source against silent drift to a hardcoded hex or a different ember
// shade — jsdom can't resolve var() against the external design sheet,
// so a string-level contract test is the practical regression gate.

const cssPath = resolve(__dirname, 'BranchGutter.css');
const css = readFileSync(cssPath, 'utf-8');

function ruleBody(selector: string): string {
  const at = css.indexOf(selector);
  if (at < 0) throw new Error(`selector not found in BranchGutter.css: ${selector}`);
  const open = css.indexOf('{', at);
  const close = css.indexOf('}', open);
  if (open < 0 || close < 0) {
    throw new Error(`malformed rule block for selector: ${selector}`);
  }
  return css.slice(open + 1, close).trim();
}

describe('BranchGutter.css `.branch-gutter` (F-418 docstring/token contract)', () => {
  const body = ruleBody('.branch-gutter').replace(/\s+/g, ' ');

  it('paints the thread-line via `var(--color-ember-300, ...)` per branching.md §15.4', () => {
    expect(body).toMatch(/background:\s*var\(--color-ember-300/);
  });

  it('no longer hardcodes the #ff7a30 hex without going through the token', () => {
    // A raw `background: #ff7a30;` would bypass the design-token surface
    // and re-introduce the docstring-drift risk F-418 guards against.
    expect(body).not.toMatch(/background:\s*#ff7a30\s*;/i);
  });
});
