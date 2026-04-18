// UAT-07 — Four-scope inline approval (F-027).
// Plan: docs/testing/phase1-uat.md §UAT-07

import { test, expect, type TauriMockHandle } from './fixtures/tauri-mock';
import {
  toolCallApprovalRequested,
  toolCallStarted,
} from './fixtures/events';

const SESSION_ID = 'aaa';

async function mountSessionWithApproval(
  page: import('@playwright/test').Page,
  tauri: TauriMockHandle,
  toolName: string,
  path: string,
  diff?: string,
) {
  await tauri.onInvoke('session_hello', async () => ({
    session_id: SESSION_ID,
    workspace: '/tmp/uat-ws',
    started_at: '2026-04-18T10:00:00Z',
    event_seq: 0,
    schema_version: 1,
  }));
  await tauri.onInvoke('session_subscribe', async () => undefined);
  await tauri.onInvoke('session_approve_tool', async () => undefined);
  await tauri.onInvoke('session_reject_tool', async () => undefined);
  await page.goto(`/session/${SESSION_ID}`);
  await tauri.emit('session:event', toolCallStarted(SESSION_ID, 'tc-1', toolName, { path, diff }));
  // Rust's ApprovalPreview is `{description: String}` only — the shell's
  // preview builder embeds any diff text inside the description so the UI's
  // single-string renderer shows it verbatim.
  const description = diff ? `${toolName} on ${path}\n${diff}` : `${toolName} on ${path}`;
  await tauri.emit(
    'session:event',
    toolCallApprovalRequested(SESSION_ID, 'tc-1', { description }),
  );
}

test.describe('UAT-07 — four-scope inline approval', () => {
  test('approval renders inline inside the tool call card, not as a modal', async ({
    tauri,
    page,
  }) => {
    await mountSessionWithApproval(page, tauri, 'fs.edit', 'src/a.ts', '--- before\n+++ after\n');
    const card = page.getByTestId('tool-call-card-tc-1');
    await expect(card).toBeVisible();
    await expect(card.getByTestId('approval-prompt')).toBeVisible();
    // Negative: no dialog role elsewhere.
    await expect(page.getByRole('dialog')).toHaveCount(0);
  });

  test('diff preview renders from ApprovalPreview', async ({ tauri, page }) => {
    await mountSessionWithApproval(
      page,
      tauri,
      'fs.edit',
      'src/a.ts',
      '--- before\n+++ after\n@@ -1 +1 @@\n-old\n+new\n',
    );
    const preview = page.getByTestId('approval-preview');
    await expect(preview).toBeVisible();
    // NOTE: these string assertions assume ApprovalPrompt renders raw diff text.
    // If the component runs the diff through a tokenised syntax highlighter,
    // swap to role-/aria- based selectors that inspect the rendered tree.
    await expect(preview).toContainText('+new');
    await expect(preview).toContainText('-old');
  });

  test('keyboard shortcuts R/A/F/P/T dispatch the correct scope', async () => {
    test.skip(true, 'per-shortcut flow pending — see phase1-uat.md §UAT-07 steps 4-9');
  });

  test('whitelist pill appears after scope approval', async () => {
    test.skip(true, 'whitelist state needs second tool call — see phase1-uat.md §UAT-07 step 6');
  });
});
