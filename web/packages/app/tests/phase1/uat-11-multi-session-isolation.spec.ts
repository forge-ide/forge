// UAT-11 — Multi-session isolation.
// Plan: docs/testing/phase1-uat.md §UAT-11
//
// Requires two real `forged` daemons (one per session) so whitelists and
// event streams can be observed independently. Pending the forged bridge
// fixture.

import { test } from '@playwright/test';

test.describe('UAT-11 — multi-session isolation', () => {
  test.skip(true, 'requires two forged daemons — see phase1-uat.md §UAT-11');

  test('events from session A never appear in session B', async () => {
    // Steps 2-3
  });

  test('whitelists are session-local, not global', async () => {
    // Steps 4-5
  });
});
