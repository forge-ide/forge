// F-412: ActivityBar disabled-state CSS contract.
//
// `docs/design/component-principles.md §Buttons` bans opacity as a disabled
// affordance: "Disabled buttons use iron-600 background and text. Never
// reduce opacity on a button to show disabled state — opacity makes elements
// appear interactive." Mirror the `ChatPane.css` composer-disabled precedent
// by asserting against the CSS source directly — jsdom does not resolve
// external stylesheets at render time, so the file itself is the contract.
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const cssSource = readFileSync(resolve(__dirname, 'ActivityBar.css'), 'utf-8');

// Match `.activity-bar__item:disabled { ... }` and capture the rule body.
const ruleMatch = cssSource.match(
  /\.activity-bar__item:disabled\s*\{([^}]*)\}/,
);

describe('ActivityBar disabled state CSS (F-412)', () => {
  it('declares a `:disabled` rule for the activity-bar item', () => {
    expect(ruleMatch).not.toBeNull();
  });

  it('does not reduce opacity to signal disabled (component-principles.md)', () => {
    const body = ruleMatch?.[1] ?? '';
    expect(body).not.toMatch(/\bopacity\s*:/);
  });

  it('uses --color-text-disabled (iron-600) for the disabled text color', () => {
    const body = ruleMatch?.[1] ?? '';
    expect(body).toMatch(/color\s*:\s*var\(--color-text-disabled\)/);
  });

  it('keeps `cursor: default` to signal the non-interactive state', () => {
    const body = ruleMatch?.[1] ?? '';
    expect(body).toMatch(/cursor\s*:\s*default/);
  });
});

describe('ActivityBar.css global opacity hygiene (F-412)', () => {
  it('declares no opacity rule anywhere in the file', () => {
    expect(cssSource).not.toMatch(/\bopacity\s*:/);
  });
});
