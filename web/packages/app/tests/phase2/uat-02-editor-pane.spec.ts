// UAT-02 — F-121/122/123/148 — Monaco-in-iframe + LSP diagnostics + go-to-definition.
// Plan: docs/testing/phase2-uat.md §UAT-02

import { test } from '@playwright/test';

test.describe('UAT-02 — F-121/122/123/148 — Monaco-in-iframe + LSP diagnostics + go-to-definition', () => {
  test.skip(true, 'Stub: blocked on tauri-driver; iframe diagnostic introspection hook. See docs/testing/phase2-uat.md §UAT-02 and the Instrumentation gap callout.');

  test('placeholder', () => {
    // Implementation pending. Steps and expected outcomes are documented in
    // docs/testing/phase2-uat.md §UAT-02. Once the blockers above are resolved,
    // remove the test.skip() at the top and translate each plan-step into a
    // concrete assertion using the data-testid selectors enumerated in the plan.
  });
});
