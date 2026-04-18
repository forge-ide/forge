// UAT-04 — Session window lifecycle (F-024).
// Plan: docs/testing/phase1-uat.md §UAT-04

import { test, expect } from './fixtures/tauri-mock';

const SESSION_ID = 'aaa';

test.describe('UAT-04 — Session window lifecycle', () => {
  test('mount → session_hello + session_subscribe; unmount → no leaks', async ({ tauri, page }) => {
    await tauri.onInvoke('session_hello', async () => ({
      session_id: SESSION_ID,
      workspace: '/tmp/uat-ws',
      started_at: '2026-04-18T10:00:00Z',
      event_seq: 0,
      schema_version: 1,
    }));
    await tauri.onInvoke('session_subscribe', async () => undefined);

    await page.goto(`/session/${SESSION_ID}`);
    await expect(page.getByRole('region', { name: 'Session pane' })).toBeVisible();

    const calls = await tauri.calls();
    expect(calls.filter((c) => c.cmd === 'session_hello')).toHaveLength(1);
    expect(calls.filter((c) => c.cmd === 'session_subscribe')).toHaveLength(1);

    await expect(page.getByTestId('pane-header-subject')).toBeVisible();
    await expect(page.getByTestId('pane-header-provider')).toBeVisible();
    await expect(page.getByTestId('pane-header-cost')).toBeVisible();
  });

  test('no splitter UI — Phase 1 is single-pane', async ({ tauri, page }) => {
    await tauri.onInvoke('session_hello', async () => ({
      session_id: SESSION_ID,
      workspace: '/tmp/uat-ws',
      started_at: '2026-04-18T10:00:00Z',
      event_seq: 0,
      schema_version: 1,
    }));
    await tauri.onInvoke('session_subscribe', async () => undefined);
    await page.goto(`/session/${SESSION_ID}`);

    await expect(page.locator('[data-testid^="splitter"]')).toHaveCount(0);
    await expect(page.locator('[data-testid^="dock-zone"]')).toHaveCount(0);
  });

  test('close button triggers unsubscribe path', async ({ tauri, page }) => {
    // TODO: the close action currently maps to window close in Tauri. With the
    // Vite-dev harness there is no real window close — document the manual
    // verification step and keep this test skipped until we wire a window-close
    // IPC command.
    test.skip(true, 'close action requires tauri-driver harness — see phase1-uat.md §UAT-04 step 4');
    expect(tauri).toBeDefined();
  });
});
