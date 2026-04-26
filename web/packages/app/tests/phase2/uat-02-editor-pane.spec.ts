// UAT-02 — F-121/122/123/148 — Monaco-in-iframe + LSP diagnostics + go-to-definition.
// Plan: docs/testing/phase2-uat.md §UAT-02
//
// Blocked on `tauri-driver` + a real `forge-lsp` against a TS fixture file.
// The iframe renders Monaco's full editor (workers, AMD globals); jsdom
// cannot mount it (per monaco-host/README), so even the mocked-IPC harness
// can't drive diagnostic / hover / go-to-definition assertions. Steps that
// scrape Monaco internals additionally need an `editor-pane-diagnostic-count`
// data attribute on the host wrapper (instrumentation gap, plan §UAT-02).

import { test } from '@playwright/test';

test.describe('UAT-02 — F-121/122/123/148 — Monaco-in-iframe + LSP diagnostics + go-to-definition', () => {
  test.skip(
    true,
    'Blocked on tauri-driver + iframe diagnostic introspection hook. See docs/testing/phase2-uat.md §UAT-02.',
  );

  test('Step 1: open src/example.ts -> editor-pane-iframe visible after editor-pane-file-loading clears', () => {
    // await driver.byTestId('files-sidebar-row', { path: 'src/example.ts' }).dblclick();
    // await expect(driver.byTestId('editor-pane-file-loading')).toBeVisible();
    // await expect(driver.byTestId('editor-pane-iframe')).toBeVisible();
    // await expect(driver.byTestId('editor-pane-file-loading')).toHaveCount(0);
  });

  test('Step 2: wait for LSP attach (≤3s) -> diagnostic underline on `unused`', () => {
    // const iframe = driver.frame({ name: 'Monaco editor host' });
    // await expect(iframe.locator('.squiggly-error')).toBeVisible({ timeout: 3000 });
  });

  test('Step 3: hover diagnostic -> tooltip with LSP message renders', () => {
    // await iframe.locator('.squiggly-error').hover();
    // await expect(iframe.locator('.monaco-hover')).toContainText('Type \'string\' is not assignable to type \'number\'');
  });

  test('Step 4: Cmd/Ctrl-click `greet` second call -> jumps to definition (line 1)', () => {
    // await iframe.locator('text=greet').nth(1).click({ modifiers: ['Control'] });
    // await expect(iframe.locator('.cursor-line')).toHaveAttribute('data-line', '1');
  });

  test('Step 5: edit file -> editor-pane-dirty mounts with aria-label="unsaved changes"', () => {
    // await iframe.locator('.monaco-editor textarea').type('x');
    // await expect(driver.byTestId('editor-pane-dirty')).toBeVisible();
    // await expect(driver.byTestId('editor-pane-dirty')).toHaveAttribute('aria-label', 'unsaved changes');
  });

  test('Step 6: Cmd/Ctrl+S -> editor-pane-dirty unmounts; disk reflects edit', () => {
    // await driver.page.keyboard.press('Control+S');
    // await expect(driver.byTestId('editor-pane-dirty')).toHaveCount(0);
    // expect(await fs.readFile(`${WS}/src/example.ts`, 'utf8')).toContain('x');
  });

  test('Step 7: kill LSP process -> editor-pane-error[role="alert"] mounts with Reload', () => {
    // await driver.killLspProcess();
    // await expect(driver.byTestId('editor-pane-error')).toHaveAttribute('role', 'alert');
    // await expect(driver.locator('button', { hasText: 'Reload' })).toBeVisible();
  });

  test('Step 8: click Reload -> error clears; iframe re-attaches; diagnostics re-flow', () => {
    // await driver.locator('button', { hasText: 'Reload' }).click();
    // await expect(driver.byTestId('editor-pane-error')).toHaveCount(0);
    // await expect(iframe.locator('.squiggly-error')).toBeVisible();
  });
});
