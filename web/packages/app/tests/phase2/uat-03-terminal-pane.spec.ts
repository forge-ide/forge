// UAT-03 — F-124/125/146 — forge-term + xterm.js + ANSI/resize.
// Plan: docs/testing/phase2-uat.md §UAT-03

import { test } from '@playwright/test';

test.describe('UAT-03 — F-124/125/146 — forge-term + xterm.js + ANSI/resize', () => {
  test.skip(true, 'Stub: blocked on tauri-driver; xterm cell-level scrape. See docs/testing/phase2-uat.md §UAT-03 and the Instrumentation gap callout.');

  test('placeholder', () => {
    // Implementation pending. Steps and expected outcomes are documented in
    // docs/testing/phase2-uat.md §UAT-03. Once the blockers above are resolved,
    // remove the test.skip() at the top and translate each plan-step into a
    // concrete assertion using the data-testid selectors enumerated in the plan.
  });
});
