// UAT-01 — F-117/118/119/120/126/150 — 4-pane layout, drag-to-dock, persistence.
// Plan: docs/testing/phase2-uat.md §UAT-01
//
// Blocked on `tauri-driver`: this UAT exercises real drag-to-dock pointer
// gestures and on-disk layout persistence (`<workspace>/.forge/layouts.json`
// round-trip across a window restart). Both require the Tauri shell, not
// the Vite-only mocked-IPC harness Phase 1 / UAT-09 use.
// Instrumentation gap (per plan): drop-zone visual feedback has no
// `data-testid="drop-zone-{zone}"`. Steps 5-6 verify the *outcome* of a
// drop, not the in-flight visual state.

import { test } from '@playwright/test';

test.describe('UAT-01 — F-117/118/119/120/126/150 — 4-pane layout, drag-to-dock, persistence', () => {
  test.skip(
    true,
    'Blocked on tauri-driver real-shell harness + drop-zone selector. See docs/testing/phase2-uat.md §UAT-01.',
  );

  test('Step 1: launch from forge-shell, click seeded session card -> default chat layout', () => {
    // const session = await driver.launchShell();
    // await session.clickSessionCard('test-agent');
    // await expect(driver.byTestId('chat-pane')).toBeVisible();
  });

  test('Step 2: toggle Files sidebar -> [data-testid="files-sidebar"] mounts with $WS tree', () => {
    // await driver.toggleSidebar('files');
    // await expect(driver.byTestId('files-sidebar')).toBeVisible();
    // await expect(driver.byTestId('files-sidebar-row').first()).toBeVisible();
  });

  test('Step 3: open readable.txt from sidebar -> editor-pane + editor-pane-iframe visible', () => {
    // await driver.byTestId('files-sidebar-row', { name: 'readable.txt' }).dblclick();
    // await expect(driver.byTestId('editor-pane')).toBeVisible();
    // await expect(driver.byTestId('editor-pane-iframe')).toBeVisible();
  });

  test('Step 4: open terminal pane -> terminal-pane + terminal-pane-host mount', () => {
    // await driver.openTerminalPane();
    // await expect(driver.byTestId('terminal-pane')).toBeVisible();
    // await expect(driver.byTestId('terminal-pane-host')).toBeVisible();
  });

  test('Step 5: drag EditorPane title-bar onto right-half drop zone of TerminalPane', () => {
    // await driver.dragHeader('editor-pane', 'terminal-pane', 'right');
    // const leaves = driver.locator('[data-leaf-id]');
    // expect(await leaves.count()).toBeGreaterThanOrEqual(2);
  });

  test('Step 6: drag TerminalPane onto bottom-half of EditorPane -> 2x2 grid via grid-leaf-*', () => {
    // await driver.dragHeader('terminal-pane', 'editor-pane', 'bottom');
    // const leaves = driver.locator('[data-testid^="grid-leaf-"]');
    // expect(await leaves.count()).toBe(4);
  });

  test('Step 7: resize splitter between EditorPane and TerminalPane -> terminal_resize IPC fires', () => {
    // const before = await driver.boundingBox('terminal-pane');
    // await driver.dragSplitter('editor-pane', 'terminal-pane', { dx: -80 });
    // const after = await driver.boundingBox('terminal-pane');
    // expect(after.width).not.toBe(before.width);
    // expect(driver.ipcCalls('terminal_resize')).toHaveLength(1);
  });

  test('Step 8: close session window -> layoutStore persists tree to layouts.json', () => {
    // await driver.closeWindow();
    // const persisted = await fs.readJson(`${WS}/.forge/layouts.json`);
    // expect(persisted.named.default.tree).toMatchObject({ kind: 'split' });
  });

  test('Step 9: re-open session card -> 2x2 grid restores with same ratios + sidebar', () => {
    // await driver.clickSessionCard('test-agent');
    // expect(await driver.locator('[data-testid^="grid-leaf-"]').count()).toBe(4);
    // await expect(driver.byTestId('files-sidebar')).toBeVisible();
  });
});
