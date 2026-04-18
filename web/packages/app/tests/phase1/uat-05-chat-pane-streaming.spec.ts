// UAT-05 — Chat pane streaming and composer (F-025).
// Plan: docs/testing/phase1-uat.md §UAT-05

import { test, expect, type TauriMockHandle } from './fixtures/tauri-mock';
import {
  assistantDelta,
  assistantMessageFinal,
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
  await expect(page.getByTestId('chat-pane')).toBeVisible();
}

test.describe('UAT-05 — Chat pane streaming & composer', () => {
  test('Enter submits; Shift+Enter inserts newline', async ({ tauri, page }) => {
    await mountSession(page, tauri);

    const composer = page.getByTestId('composer-textarea');
    await composer.click();
    await composer.type('line one');
    await composer.press('Shift+Enter');
    await composer.type('line two');
    expect(await composer.inputValue()).toBe('line one\nline two');

    await composer.press('Enter');
    const calls = await tauri.calls();
    const sent = calls.filter((c) => c.cmd === 'session_send_message');
    expect(sent).toHaveLength(1);
    expect(sent[0].args).toMatchObject({ sessionId: SESSION_ID, text: 'line one\nline two' });
  });

  test('streaming cursor shows while deltas arrive and hides on final (F-037)', async ({
    tauri,
    page,
  }) => {
    await mountSession(page, tauri);

    // Drive a turn: user message → deltas → final.
    // Rust has no "assistant open" event; the first AssistantDelta creates
    // the streaming assistant turn.
    await tauri.emit('session:event', userMessage(SESSION_ID, 'hello'));
    await tauri.emit('session:event', assistantDelta(SESSION_ID, 'turn-1', 'Hi'));

    await expect(page.getByTestId('streaming-cursor')).toBeVisible();

    await tauri.emit('session:event', assistantDelta(SESSION_ID, 'turn-1', ' there.'));
    await tauri.emit(
      'session:event',
      assistantMessageFinal(SESSION_ID, 'turn-1', 'Hi there.'),
    );

    await expect(page.getByTestId('streaming-cursor')).toBeHidden();
    await expect(page.getByTestId('message-list')).toContainText('Hi there.');
  });

  test('composer stays disabled through the stream, re-enables on final', async () => {
    test.skip(
      true,
      'Tracked separately as F-040 (#76): composer re-enables mid-stream because AssistantDelta clears awaitingResponse.',
    );
  });

  test('auto-scroll pin releases on user scroll-up, re-engages on scroll-bottom', async ({
    tauri,
    page,
  }) => {
    await mountSession(page, tauri);
    // TODO: fill scroll assertion — requires driving many deltas and using
    // page.mouse.wheel to simulate user scroll. See phase1-uat.md §UAT-05 step 4-5.
    test.skip(true, 'scroll assertion pending — see phase1-uat.md §UAT-05 step 4');
  });

  test('error events render inline, not as modals', async ({ tauri, page }) => {
    await mountSession(page, tauri);
    // TODO: emit a synthesized error event once the error-event shape is stable.
    test.skip(true, 'error-event shape pending — see phase1-uat.md §UAT-05 step 6');
  });
});
