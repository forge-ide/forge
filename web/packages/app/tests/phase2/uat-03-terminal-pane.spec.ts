// UAT-03 — F-124/125/146 — forge-term + xterm.js + ANSI/resize.
// Plan: docs/testing/phase2-uat.md §UAT-03
//
// Blocked on `tauri-driver` (real PTY spawned by `forge-shell`) AND on
// SessionWindow grid-wiring TerminalPane (today the `terminal` pane_type
// renders the `terminal-pane-stub` placeholder — see SessionWindow.tsx).
// Cell-level ANSI assertions also need either xterm DOM scrape or a
// postMessage instrumentation hook (instrumentation gap, plan §UAT-03).

import { test } from '@playwright/test';

test.describe('UAT-03 — F-124/125/146 — forge-term + xterm.js + ANSI/resize', () => {
  test.skip(
    true,
    'Blocked on tauri-driver + grid-wiring TerminalPane + xterm cell scrape. See docs/testing/phase2-uat.md §UAT-03.',
  );

  test('Step 1: open terminal pane -> terminal-pane-loading then terminal-pane-host with .xterm', () => {
    // await driver.openTerminalPane();
    // await expect(driver.byTestId('terminal-pane-loading')).toHaveAttribute('role', 'status');
    // await expect(driver.byTestId('terminal-pane-host')).toBeVisible();
    // await expect(driver.byTestId('terminal-pane-host').locator('.xterm')).toBeVisible();
  });

  test('Step 2: type `echo hello` Enter -> hello renders; cursor advances', () => {
    // await driver.typeIntoTerminal('echo hello\n');
    // await expect(driver.byTestId('terminal-pane-host')).toContainText('hello');
  });

  test('Step 3: ANSI red escape -> rendered span has red color', () => {
    // await driver.typeIntoTerminal("printf '\\x1b[31mred\\x1b[0m\\n'\n");
    // const span = driver.byTestId('terminal-pane-host').locator('span', { hasText: 'red' });
    // expect(await span.evaluate((el) => getComputedStyle(el).color)).toMatch(/rgb\(.*0.*0\)/);
  });

  test('Step 4: resize pane -> fitAddon.fit() runs; terminal_resize IPC fires', () => {
    // await driver.dragSplitter('terminal-pane', 'editor-pane', { dx: 60 });
    // expect(driver.ipcCalls('terminal_resize').length).toBeGreaterThan(0);
  });

  test('Step 5: type `clear` Enter -> xterm buffer resets; cursor at top', () => {
    // await driver.typeIntoTerminal('clear\n');
    // await expect(driver.byTestId('terminal-pane-host')).not.toContainText('hello');
  });

  test('Step 6: spawn failure ($SHELL=nonexistent) -> error variant with role="alert"', () => {
    // await driver.spawnTerminalWithEnv({ SHELL: '/no/such/shell' });
    // await expect(driver.byTestId('terminal-pane-loading')).toHaveCount(0);
    // // Instrumentation gap: scope by class until terminal-pane-error data-testid lands.
    // await expect(driver.locator('.terminal-pane__error')).toHaveAttribute('role', 'alert');
  });
});
