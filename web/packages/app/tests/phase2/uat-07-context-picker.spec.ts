// UAT-07 — F-141/142/147/357/536 — @-context picker + truncation notice.
// Plan: docs/testing/phase2-uat.md §UAT-07
//
// Blocked on `tauri-driver` (real `forge-fs` walker over a 5000-file
// fixture to trigger the truncation cap, plus end-to-end provider IPC for
// Step 8's content-injection assertion). The in-picker truncation selector
// landed under issue #536; Step 4 now verifies both surfaces.

import { test } from '@playwright/test';

test.describe('UAT-07 — F-141/142/147/357/536 — @-context picker + truncation notice', () => {
  test.skip(
    true,
    'Blocked on tauri-driver + 5000-file fixture. See docs/testing/phase2-uat.md §UAT-07.',
  );

  test('Step 1: focus chat composer', () => {
    // await driver.composer().focus();
  });

  test('Step 2: type @ -> context-picker[role="combobox"] mounts; aria-expanded=true; aria-haspopup=listbox', () => {
    // await driver.composer().press('@');
    // const picker = driver.byTestId('context-picker');
    // await expect(picker).toHaveAttribute('role', 'combobox');
    // await expect(picker).toHaveAttribute('aria-expanded', 'true');
    // await expect(picker).toHaveAttribute('aria-haspopup', 'listbox');
  });

  test('Step 3: type "read" after @ -> query reflects "read"; results list filters', () => {
    // await driver.composer().type('read');
    // await expect(driver.byTestId('context-picker-query')).toHaveText('read');
    // await expect(driver.byTestId('context-picker-results').locator('li')).toHaveCount.greaterThan(0);
  });

  test('Step 4: truncation notice -> sidebar AND picker both show "N files not shown" (F-536)', () => {
    // await expect(driver.byTestId('files-sidebar-stats-notice')).toContainText(/files not shown/);
    // // F-536: the picker also renders an inline notice on tree-backed tabs
    // // (file / directory). role="status" and wording mirrors the sidebar.
    // const pickerNotice = driver.byTestId('picker-truncation-notice');
    // await expect(pickerNotice).toContainText(/files not shown/);
    // await expect(pickerNotice).toHaveAttribute('role', 'status');
  });

  test('Step 5: ArrowDown navigates options -> aria-activedescendant advances', () => {
    // await driver.page.keyboard.press('ArrowDown');
    // const picker = driver.byTestId('context-picker');
    // const active = await picker.getAttribute('aria-activedescendant');
    // await driver.page.keyboard.press('ArrowDown');
    // expect(await picker.getAttribute('aria-activedescendant')).not.toBe(active);
  });

  test('Step 6: Tab to switch category to Folder -> context-picker-tab-folder active', () => {
    // await driver.page.keyboard.press('Tab');
    // await expect(driver.byTestId('context-picker-tab-folder')).toHaveAttribute('aria-selected', 'true');
  });

  test('Step 7: Enter on a result -> picker closes; composer contains chip', () => {
    // await driver.page.keyboard.press('Enter');
    // await expect(driver.byTestId('context-picker')).toHaveCount(0);
    // await expect(driver.composer().locator('.chip')).toBeVisible();
  });

  test('Step 8: submit message -> provider receives file contents inlined into prompt', () => {
    // await driver.composer().press('Enter');
    // const sent = await driver.mockProvider().lastUserMessage();
    // expect(sent).toContain(filePickedContents);
  });

  test('Step 9: Escape with picker open -> closes without inserting; composer focus restored', () => {
    // await driver.composer().press('@');
    // await driver.page.keyboard.press('Escape');
    // await expect(driver.byTestId('context-picker')).toHaveCount(0);
    // await expect(driver.composer()).toBeFocused();
  });
});
