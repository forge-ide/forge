// UAT-08 — F-143/144/145 — Re-run Replace + Branch scaffolded.
// Plan: docs/testing/phase2-uat.md §UAT-08
//
// Blocked on `tauri-driver` + a re-run-aware MockProvider that scripts a
// distinct turn A' on the second invocation. The Branch button additionally
// has no `data-testid` yet (instrumentation gap, plan §UAT-08); Step 6 must
// locate it by accessible name until `data-testid="message-branch-action"`
// lands.

import { test } from '@playwright/test';

test.describe('UAT-08 — F-143/144/145 — Re-run Replace + Branch scaffolded', () => {
  test.skip(
    true,
    'Blocked on tauri-driver + re-run-aware MockProvider + Branch button selector. See docs/testing/phase2-uat.md §UAT-08.',
  );

  test('Step 1: send prompt -> branch-turn-<msg_id> mounts containing assistant message', () => {
    // await driver.composer().fill('hello');
    // await driver.composer().press('Enter');
    // await expect(driver.locator('[data-testid^="branch-turn-"]')).toBeVisible();
  });

  test('Step 2: Re-run / Replace -> new variant; branch-selector-strip[role="group"] mounts with 2 variants', () => {
    // await driver.locator('button', { hasText: 'Re-run' }).click();
    // const strip = driver.byTestId('branch-selector-strip');
    // await expect(strip).toHaveAttribute('role', 'group');
  });

  test('Step 3: strip exposes prev/next/label with aria-label', () => {
    // await expect(driver.byTestId('branch-strip-prev')).toHaveAttribute('aria-label', /previous/i);
    // await expect(driver.byTestId('branch-strip-next')).toHaveAttribute('aria-label', /next/i);
    // await expect(driver.byTestId('branch-strip-label')).toBeVisible();
  });

  test('Step 4: Next -> active variant flips to A\'; ChatPane re-renders with A\' visible (Replace semantics)', () => {
    // await driver.byTestId('branch-strip-next').click();
    // await expect(driver.byTestId('branch-strip-label')).toContainText('2 of 2');
  });

  test('Step 5: branch-strip-info -> branch-metadata-popover opens', () => {
    // await driver.byTestId('branch-strip-info').click();
    // await expect(driver.byTestId('branch-metadata-popover')).toBeVisible();
  });

  test('Step 6: Branch button is rendered (visible / focusable) but inert in Phase 2', () => {
    // // Instrumentation gap: locate by accessible name until message-branch-action ships.
    // const branchBtn = driver.locator('button', { name: /branch/i });
    // await expect(branchBtn).toBeVisible();
    // const beforeUrl = driver.page.url();
    // await branchBtn.click();
    // expect(driver.page.url()).toBe(beforeUrl);
    // // No new variant should appear and no fork IPC should have fired.
  });

  test('Step 7: ArrowLeft / ArrowRight while strip focused -> cycle prev/next per F-160', () => {
    // await driver.byTestId('branch-selector-strip').focus();
    // await driver.page.keyboard.press('ArrowRight');
    // // assert variant advanced
    // await driver.page.keyboard.press('ArrowLeft');
    // // assert variant returned
  });

  test('Step 8: Escape closes popover; focus returns to info button', () => {
    // await driver.byTestId('branch-strip-info').click();
    // await driver.page.keyboard.press('Escape');
    // await expect(driver.byTestId('branch-metadata-popover')).toHaveCount(0);
    // await expect(driver.byTestId('branch-strip-info')).toBeFocused();
  });
});
