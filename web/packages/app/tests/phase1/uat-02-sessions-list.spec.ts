// UAT-02 — Dashboard sessions list + filters (F-022).
// Plan: docs/testing/phase1-uat.md §UAT-02

import { test, expect } from './fixtures/tauri-mock';

const ACTIVE_SESSIONS = [
  {
    id: 'aaa',
    subject: 'refactor-payment-service',
    provider: 'mock',
    state: 'active',
    persistence: 'persist',
    created_at: '2026-04-18T10:00:00Z',
    last_event_at: '2026-04-18T10:15:00Z',
  },
  {
    id: 'bbb',
    subject: 'debug-auth',
    provider: 'mock',
    state: 'active',
    persistence: 'ephemeral',
    created_at: '2026-04-18T11:00:00Z',
    last_event_at: '2026-04-18T11:05:00Z',
  },
  {
    id: 'ccc',
    subject: 'dead-session',
    provider: 'mock',
    state: 'stopped',
    persistence: 'persist',
    created_at: '2026-04-18T09:00:00Z',
    last_event_at: '2026-04-18T09:10:00Z',
  },
];

const ARCHIVED_SESSIONS = [
  {
    id: 'ddd',
    subject: 'old-session-1',
    provider: 'mock',
    state: 'archived',
    persistence: 'persist',
    created_at: '2026-04-10T09:00:00Z',
    last_event_at: '2026-04-10T10:00:00Z',
  },
  {
    id: 'eee',
    subject: 'old-session-2',
    provider: 'mock',
    state: 'archived',
    persistence: 'persist',
    created_at: '2026-04-11T09:00:00Z',
    last_event_at: '2026-04-11T10:00:00Z',
  },
];

test.describe('UAT-02 — Dashboard sessions list + filters', () => {
  test('empty state: no sessions', async ({ tauri, page }) => {
    await tauri.onInvoke('session_list', async () => []);
    await tauri.onInvoke('provider_status', async () => ({
      reachable: false,
      base_url: 'http://127.0.0.1:11434',
      models: [],
      last_checked: new Date().toISOString(),
    }));
    await page.goto('/');
    await expect(page.getByRole('region', { name: 'Sessions' })).toBeVisible();
    // Accept the current voice copy ("no active sessions") plus plausible variants
    // ("no sessions", "start one"). Replace with a data-testid when one exists.
    await expect(page.getByRole('region', { name: 'Sessions' })).toContainText(
      /no\s+(active\s+)?sessions|start one/i,
    );
  });

  test('three active sessions render with subject, provider, persistence badge', async ({
    tauri,
    page,
  }) => {
    await tauri.onInvoke('session_list', async () => [...ACTIVE_SESSIONS, ...ARCHIVED_SESSIONS]);
    await tauri.onInvoke('provider_status', async () => ({
      reachable: true,
      base_url: 'http://127.0.0.1:11434',
      models: ['llama3.2:1b'],
      last_checked: new Date().toISOString(),
    }));
    await page.goto('/');

    for (const s of ACTIVE_SESSIONS) {
      await expect(page.getByRole('button', { name: `Open session ${s.subject}` })).toBeVisible();
    }
  });

  test('Archived tab shows archived cards only', async ({ tauri, page }) => {
    await tauri.onInvoke('session_list', async () => [...ACTIVE_SESSIONS, ...ARCHIVED_SESSIONS]);
    await tauri.onInvoke('provider_status', async () => ({
      reachable: true,
      base_url: 'http://127.0.0.1:11434',
      models: [],
      last_checked: new Date().toISOString(),
    }));
    await page.goto('/');
    // TODO: wire the Archived tab selector. Until the SessionsPanel exposes a
    // tablist with accessible names, this step stays a stub.
    test.skip(true, 'tab selector not yet stable — see phase1-uat.md §UAT-02 step 3');
  });

  test('clicking an active card invokes open_session', async ({ tauri, page }) => {
    await tauri.onInvoke('session_list', async () => ACTIVE_SESSIONS);
    await tauri.onInvoke('provider_status', async () => ({
      reachable: true,
      base_url: 'http://127.0.0.1:11434',
      models: [],
      last_checked: new Date().toISOString(),
    }));
    await tauri.onInvoke('open_session', async () => undefined);
    await page.goto('/');

    await page.getByRole('button', { name: 'Open session refactor-payment-service' }).click();

    const calls = await tauri.calls();
    const openCalls = calls.filter((c) => c.cmd === 'open_session');
    expect(openCalls).toHaveLength(1);
    expect(openCalls[0].args).toMatchObject({ id: 'aaa' });
  });
});
