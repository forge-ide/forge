// UAT-12 — F-036/151 — persistent settings + persistent approvals.
// Plan: docs/testing/phase2-uat.md §UAT-12
//
// Blocked on `tauri-driver` (real `forge-shell` + a tempdir workspace so
// the bash + Playwright pieces can round-trip writes through
// `$XDG_CONFIG_HOME/forge/settings.toml` and `$WS/.forge/{settings,approvals}.toml`
// across a window restart). The atomic-write invariant additionally needs
// either a crash-injection harness or per-write fsync inspection.

import { test } from '@playwright/test';

test.describe('UAT-12 — F-036/151 — persistent settings + persistent approvals', () => {
  test.skip(
    true,
    'Blocked on tauri-driver + tempdir workspace + atomic-write spy. See docs/testing/phase2-uat.md §UAT-12.',
  );

  test('Step 1: change a user-scoped setting via Settings UI -> $XDG_CONFIG_HOME/forge/settings.toml updated', () => {
    // await driver.openSettings();
    // await driver.byLabel('Density').selectOption('compact');
    // const toml = await fs.readFile(`${XDG}/forge/settings.toml`, 'utf8');
    // expect(toml).toMatch(/density\s*=\s*"compact"/);
  });

  test('Step 2: quit + relaunch -> setting persists; UI reads same value at startup', () => {
    // await driver.relaunch();
    // await driver.openSettings();
    // await expect(driver.byLabel('Density')).toHaveValue('compact');
  });

  test('Step 3: tool call requiring approval; approve "This tool" -> $WS/.forge/approvals.toml created', () => {
    // await driver.runMockToolCall('write_file', { path: 'foo.txt' });
    // await driver.locator('button', { hasText: /this tool/i }).click();
    // const toml = await fs.readFile(`${WS}/.forge/approvals.toml`, 'utf8');
    // expect(toml).toContain('write_file');
  });

  test('Step 4: relaunch + same tool call -> auto-approves; pill reads "whitelisted · this tool"', () => {
    // await driver.relaunch();
    // await driver.runMockToolCall('write_file', { path: 'foo.txt' });
    // await expect(driver.locator('.tool-call-card .pill')).toContainText(/whitelisted.*this tool/);
  });

  test('Step 5: delete approvals.toml; rerun -> tool re-prompts; whitelist starts empty', () => {
    // await fs.rm(`${WS}/.forge/approvals.toml`);
    // await driver.relaunch();
    // await driver.runMockToolCall('write_file', { path: 'foo.txt' });
    // await expect(driver.byTestId('approval-prompt')).toBeVisible();
  });

  test('Step 6: workspace-scoped setting -> written to $WS/.forge/settings.toml, NOT user file', () => {
    // await driver.openSettings();
    // await driver.byLabel('Workspace theme').selectOption('dark');
    // const wsToml = await fs.readFile(`${WS}/.forge/settings.toml`, 'utf8');
    // const userToml = await fs.readFile(`${XDG}/forge/settings.toml`, 'utf8');
    // expect(wsToml).toContain('theme');
    // expect(userToml).not.toContain('theme = "dark"');
  });
});
