// UAT-06 — Tool call card rendering (F-026 / F-447).
// Plan: docs/testing/phase1-uat.md §UAT-06
//
// F-447 re-enables the Phase 2 skipped interactions:
//   • expand/collapse toggle on completed cards
//   • parallel-reads group header for read-only calls sharing a batch_id
//   • errored-state rendering
//   • fs.edit diff preview in the expanded body
//
// All selectors below are authoritative — the component uses them too.

import { test, expect, type TauriMockHandle } from './fixtures/tauri-mock';
import {
  toolCallApprovalRequested,
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
    await expect(page.getByTestId('tool-call-card-tc-1')).toContainText('readable.txt');
  });

  test('expand/collapse toggle persists while window is open', async ({ tauri, page }) => {
    await mountSession(page, tauri);
    await tauri.emit('session:event', userMessage(SESSION_ID, 'open it'));
    await tauri.emit(
      'session:event',
      toolCallStarted(SESSION_ID, 'tc-xp', 'fs.read', { path: 'a.txt' }),
    );
    await tauri.emit(
      'session:event',
      toolCallCompleted(SESSION_ID, 'tc-xp', { ok: true, preview: 'payload' }),
    );

    const card = page.getByTestId('tool-call-card-tc-xp');
    await expect(card).toHaveAttribute('data-expanded', 'false');
    await page.getByTestId('tool-call-row-tc-xp').click();
    await expect(card).toHaveAttribute('data-expanded', 'true');
    await expect(page.getByTestId('tool-call-body-tc-xp')).toBeVisible();
    await page.getByTestId('tool-call-row-tc-xp').click();
    await expect(card).toHaveAttribute('data-expanded', 'false');
  });

  test('three read-only calls with shared batch_id render as a group', async ({ tauri, page }) => {
    await mountSession(page, tauri);
    // Rust's parallel_group is u32; the adapter stringifies it into the
    // store's batch_id field (see ipc/events.ts).
    for (const id of ['tc-1', 'tc-2', 'tc-3']) {
      await tauri.emit(
        'session:event',
        toolCallStarted(SESSION_ID, id, 'fs.read', { path: `${id}.txt` }, 7),
      );
    }
    await expect(page.getByTestId('tool-call-group-7')).toBeVisible();
    await expect(page.getByTestId('tool-call-group-count-7')).toContainText('3 calls');
  });

  test('errored status renders with the ✗ glyph', async ({ tauri, page }) => {
    await mountSession(page, tauri);
    await tauri.emit(
      'session:event',
      toolCallStarted(SESSION_ID, 'tc-err', 'fs.read', { path: 'missing.txt' }),
    );
    await tauri.emit(
      'session:event',
      toolCallCompleted(SESSION_ID, 'tc-err', { ok: false, error: 'ENOENT' }),
    );
    await expect(page.getByTestId('tool-call-status-tc-err')).toHaveText('✗');
    await expect(page.getByTestId('tool-call-card-tc-err')).toHaveAttribute(
      'data-status',
      'errored',
    );
  });

  test('fs.edit approval surfaces a diff preview in the expanded body', async ({ tauri, page }) => {
    await mountSession(page, tauri);
    await tauri.emit(
      'session:event',
      toolCallStarted(SESSION_ID, 'tc-edit', 'fs.edit', {
        path: '/src/foo.ts',
        patch: '...',
      }),
    );
    await tauri.emit(
      'session:event',
      toolCallApprovalRequested(SESSION_ID, 'tc-edit', {
        description: '--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y',
      }),
    );
    await expect(page.getByTestId('tool-call-diff-tc-edit')).toContainText('+++ b');
  });
});
