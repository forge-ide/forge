// UAT-06 — Tool call card rendering (F-026).
// Plan: docs/testing/phase1-uat.md §UAT-06

import { test, expect, type TauriMockHandle } from './fixtures/tauri-mock';
import {
  toolCallCompleted,
  toolCallStarted,
  userMessage,
} from './fixtures/events';

const SESSION_ID = 'aaa';

async function mountSession(page: import('@playwright/test').Page, tauri: TauriMockHandle) {
  await tauri.onInvoke('session_hello', async () => ({
    session_id: SESSION_ID,
    workspace: '/tmp/uat-ws',
    started_at: '2026-04-18T10:00:00Z',
    event_seq: 0,
    schema_version: 1,
  }));
  await tauri.onInvoke('session_subscribe', async () => undefined);
  await tauri.onInvoke('session_send_message', async () => undefined);
  await page.goto(`/session/${SESSION_ID}`);
}

test.describe('UAT-06 — tool call card', () => {
  test('collapsed card renders icon, name, arg summary, status', async ({ tauri, page }) => {
    await mountSession(page, tauri);
    await tauri.emit('session:event', userMessage(SESSION_ID, 'read it'));
    await tauri.emit(
      'session:event',
      toolCallStarted(SESSION_ID, 'tc-1', 'fs.read', { path: 'readable.txt' }),
    );
    await tauri.emit(
      'session:event',
      toolCallCompleted(SESSION_ID, 'tc-1', { ok: true, preview: 'hello from forge' }),
    );

    await expect(page.getByTestId('tool-call-card-tc-1')).toBeVisible();
    await expect(page.getByTestId('tool-call-card-tc-1')).toContainText('fs.read');
    // F-041: the collapsed card header also shows a one-line path summary
    // next to the tool name.
    await expect(page.getByTestId('tool-call-card-tc-1')).toContainText('readable.txt');
  });

  test('expand/collapse toggle persists while window is open', async () => {
    test.skip(true, 'interaction selector pending — see phase1-uat.md §UAT-06 step 2-3');
  });

  test('three read-only calls with shared batch_id render as a group', async ({ tauri, page }) => {
    await mountSession(page, tauri);
    // Rust's parallel_group is u32; fixture takes a number and the adapter
    // stringifies it for the store's batch_id field.
    for (const id of ['tc-1', 'tc-2', 'tc-3']) {
      await tauri.emit(
        'session:event',
        toolCallStarted(SESSION_ID, id, 'fs.read', { path: `${id}.txt` }, 7),
      );
    }
    // TODO: assert the group header. Selector not yet established.
    test.skip(true, 'group header selector pending — see phase1-uat.md §UAT-06 step 4');
  });

  test('errored status renders with error color token', async () => {
    test.skip(true, 'selector pending — see phase1-uat.md §UAT-06 step 5');
  });

  test('fs.edit completion shows unified-diff preview in expanded card', async () => {
    test.skip(true, 'selector pending — see phase1-uat.md §UAT-06 step 6');
  });
});
