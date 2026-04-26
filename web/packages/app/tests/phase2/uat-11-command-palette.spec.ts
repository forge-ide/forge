// UAT-11 — F-157 — Cmd/Ctrl+Shift+P open/search/dispatch/dismiss.
// Plan: docs/testing/phase2-uat.md §UAT-11
//
// Vehicle: Vite dev server. The palette is purely client-side (Cmd/Ctrl+K
// or Cmd/Ctrl+Shift+P shortcut, fuzzy filter, Enter dispatch). The only
// IPC the dispatched built-in needs is `useNavigate('/agents')`, which is
// pure router state.
//
// Note on built-ins: at this commit the only built-in command registered
// by `registerBuiltins()` is "Open Agent Monitor" (`commands/registerBuiltins.ts`).
// The plan's example "Toggle Files Sidebar" command is not yet wired into
// the registry — the toggle exists in `SessionWindow` as a `Cmd+Shift+E`
// handler, not as a palette entry. This test therefore drives the palette
// against the actual built-in surface rather than the plan's illustrative
// example. When more built-ins land, extend `Step 4` to assert the
// dispatched IPC for each.
//
// We mount on `/` (Dashboard) so neither SessionWindow's session-IPC chain
// nor AgentMonitor's IPC chain is required to stand up the palette.

import { test, expect } from './fixtures/tauri-mock';

test.describe('UAT-11 — F-157 — Cmd/Ctrl+Shift+P open/search/dispatch/dismiss', () => {
  test('Step 1: shortcut opens the dialog with focus in the input', async ({
    page,
  }) => {
    await page.goto('/');

    await page.keyboard.press('Control+Shift+P');

    const dialog = page.getByTestId('command-palette');
    await expect(dialog).toBeVisible();
    await expect(dialog).toHaveAttribute('role', 'dialog');
    await expect(dialog).toHaveAttribute('aria-modal', 'true');

    const input = page.getByTestId('command-palette-input');
    await expect(input).toBeFocused();
  });

  test('Step 2: typing filters via fuzzy match', async ({ page }) => {
    await page.goto('/');
    await page.keyboard.press('Control+Shift+P');

    // The default registry has at least the F-153/F-157 built-in
    // "Open Agent Monitor". A fuzzy query that subsequence-matches it
    // should keep it; an unrelated query should drop it.
    await page.getByTestId('command-palette-input').fill('opn');
    const items = page.getByTestId('command-palette-item');
    await expect(items.first()).toContainText('Open Agent Monitor');

    await page.getByTestId('command-palette-input').fill('agent');
    await expect(items.first()).toContainText('Open Agent Monitor');
  });

  test('Step 3: ArrowDown / ArrowUp move aria-selected with wrap', async ({
    page,
  }) => {
    await page.goto('/');
    await page.keyboard.press('Control+Shift+P');

    // Empty query -> all built-ins surface in registration order. The
    // palette wraps at the list edges per CP.5.
    const items = page.getByTestId('command-palette-item');
    const count = await items.count();
    expect(count).toBeGreaterThan(0);

    await expect(items.nth(0)).toHaveAttribute('aria-selected', 'true');

    // ArrowDown advances selection; from the last row it wraps to row 0.
    for (let i = 0; i < count - 1; i += 1) {
      await page.keyboard.press('ArrowDown');
    }
    await expect(items.nth(count - 1)).toHaveAttribute('aria-selected', 'true');
    await page.keyboard.press('ArrowDown');
    await expect(items.nth(0)).toHaveAttribute('aria-selected', 'true');
    // ArrowUp from row 0 wraps to the last row.
    await page.keyboard.press('ArrowUp');
    await expect(items.nth(count - 1)).toHaveAttribute('aria-selected', 'true');
  });

  test('Step 4: Enter dispatches the active command and closes the palette', async ({
    page,
  }) => {
    await page.goto('/');
    await page.keyboard.press('Control+Shift+P');

    // Filter to the single built-in we want to dispatch, then Enter.
    await page.getByTestId('command-palette-input').fill('Open Agent Monitor');
    await expect(
      page.getByTestId('command-palette-item').first(),
    ).toContainText('Open Agent Monitor');
    await page.keyboard.press('Enter');

    // Palette closes and focus is restored.
    await expect(page.getByTestId('command-palette')).toHaveCount(0);
    // The dispatched command navigates to `/agents` (the AgentMonitor
    // route per registerBuiltins.ts). Confirm the URL changed.
    await expect(page).toHaveURL(/\/agents$/);
  });

  test('Step 5: empty-state row is rendered with aria-disabled and Enter is a no-op', async ({
    page,
  }) => {
    await page.goto('/');
    await page.keyboard.press('Control+Shift+P');

    await page
      .getByTestId('command-palette-input')
      .fill('zzzzz-no-such-command-zzzzz');

    const empty = page.getByTestId('command-palette-empty');
    await expect(empty).toBeVisible();
    await expect(empty).toHaveAttribute('aria-disabled', 'true');

    // Enter on empty state must NOT navigate or dismiss the palette.
    const urlBefore = page.url();
    await page.keyboard.press('Enter');
    expect(page.url()).toBe(urlBefore);
    await expect(page.getByTestId('command-palette')).toBeVisible();
  });

  test('Step 6: shortcut while open toggles the palette closed', async ({
    page,
  }) => {
    await page.goto('/');
    await page.keyboard.press('Control+Shift+P');
    await expect(page.getByTestId('command-palette')).toBeVisible();

    await page.keyboard.press('Control+Shift+P');
    await expect(page.getByTestId('command-palette')).toHaveCount(0);
  });

  test('Step 7: Escape closes the palette', async ({ page }) => {
    await page.goto('/');
    await page.keyboard.press('Control+Shift+P');
    await expect(page.getByTestId('command-palette')).toBeVisible();

    await page.keyboard.press('Escape');
    await expect(page.getByTestId('command-palette')).toHaveCount(0);
  });
});
