// UAT-09 — F-400 — loading/empty/error states across panes.
// Plan: docs/testing/phase2-uat.md §UAT-09
//
// Vehicle: Vite dev server + mocked Tauri IPC (no `tauri-driver` needed).
// Each test mounts SessionWindow on the `/session/:id` route, then drives
// FilesSidebar / EditorPane through the loading -> success / -> empty /
// -> error / error-recovery transitions by registering different `tree` /
// `read_file` IPC handlers.
//
// Notes:
//   * TerminalPane is not yet wired into SessionWindow's grid (the
//     `terminal` pane_type renders a `terminal-pane-stub` placeholder, see
//     `routes/Session/SessionWindow.tsx`). Plan steps 7-9 (terminal
//     loading/error/recovery) are therefore not exercised here — they are
//     blocked on the wiring, not on instrumentation. They are tracked in
//     UAT-03 (real-shell) and the F-400 component-unit test for
//     TerminalPane (see `panes/TerminalPane.test.tsx`).
//   * The plan flags `terminal-pane-error` as an instrumentation gap
//     (class-only selector). Once the terminal pane is grid-wired, the
//     scoped selector for step 8 is `.terminal-pane__error[role="alert"]`.

import { test, expect } from './fixtures/tauri-mock';
import type { TauriMockHandle } from './fixtures/tauri-mock';

const SESSION_ID = 'uat09';
const WORKSPACE = '/tmp/uat09-ws';

interface PendingPromise<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (err: unknown) => void;
}

function deferred<T>(): PendingPromise<T> {
  let resolve!: (value: T) => void;
  let reject!: (err: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/**
 * Register the minimum IPC surface SessionWindow needs to reach the point
 * where FilesSidebar mounts: session_hello, session_subscribe,
 * read_layouts (default single-chat layout), get_persistent_approvals,
 * get_settings, plus the layouts write so the debounced flushes don't fail.
 */
async function bootstrapSession(tauri: TauriMockHandle): Promise<void> {
  await tauri.onInvoke('session_hello', async () => ({
    session_id: SESSION_ID,
    workspace: WORKSPACE,
    started_at: '2026-04-24T10:00:00Z',
    event_seq: 0,
    schema_version: 1,
  }));
  await tauri.onInvoke('session_subscribe', async () => undefined);
  await tauri.onInvoke('read_layouts', async () => ({
    active: 'default',
    named: {
      default: {
        tree: { kind: 'leaf', id: 'root', pane_type: 'chat' },
        pane_state: {},
      },
    },
  }));
  await tauri.onInvoke('write_layouts', async () => undefined);
  await tauri.onInvoke('get_persistent_approvals', async () => []);
  await tauri.onInvoke('get_settings', async () => ({
    notifications: { bg_agents: 'toast' },
    windows: { session_mode: 'single' },
  }));
}

test.describe('UAT-09 — F-400 — loading/empty/error states across panes', () => {
  test('Step 1: FilesSidebar loading state — `tree` never resolves', async ({
    tauri,
    page,
  }) => {
    await bootstrapSession(tauri);
    const pending = deferred<unknown>();
    await tauri.onInvoke('tree', async () => pending.promise);

    await page.goto(`/session/${SESSION_ID}`);
    // Toggle the sidebar via the activity-bar shortcut.
    await page.keyboard.press('Control+Shift+E');

    const loading = page.getByTestId('files-sidebar-loading');
    await expect(loading).toBeVisible();
    await expect(loading).toHaveAttribute('role', 'status');

    pending.resolve({
      path: WORKSPACE,
      name: 'ws',
      kind: 'Dir',
      children: [],
    });
  });

  test('Step 2: FilesSidebar empty state — `tree` resolves with no children', async ({
    tauri,
    page,
  }) => {
    await bootstrapSession(tauri);
    await tauri.onInvoke('tree', async () => ({
      path: WORKSPACE,
      name: 'ws',
      kind: 'Dir',
      children: [],
    }));

    await page.goto(`/session/${SESSION_ID}`);
    await page.keyboard.press('Control+Shift+E');

    const empty = page.getByTestId('files-sidebar-empty');
    await expect(empty).toBeVisible();
    await expect(empty).toHaveAttribute('role', 'status');
  });

  test('Step 3: FilesSidebar error state — `tree` rejects, retry recovers', async ({
    tauri,
    page,
  }) => {
    await bootstrapSession(tauri);
    // Always reject until the test explicitly flips the handler. Avoids the
    // race where a downstream reactive (e.g. activeWorkspaceRoot landing
    // post-hello) re-runs `refresh()` and clears the error before we
    // observe it.
    let succeed = false;
    await tauri.onInvoke('tree', async () => {
      if (!succeed) throw new Error('walker died');
      return {
        path: WORKSPACE,
        name: 'ws',
        kind: 'Dir',
        children: [
          { path: `${WORKSPACE}/readme.md`, name: 'readme.md', kind: 'File' },
        ],
      };
    });

    await page.goto(`/session/${SESSION_ID}`);
    await page.keyboard.press('Control+Shift+E');

    const errorBanner = page.getByTestId('files-sidebar-error');
    await expect(errorBanner).toBeVisible();
    await expect(errorBanner).toHaveAttribute('role', 'alert');
    await expect(errorBanner).toContainText('walker died');

    // Recovery: flip the handler, then click the sidebar's refresh button.
    succeed = true;
    await page.getByRole('button', { name: 'Refresh file tree' }).click();
    await expect(errorBanner).toBeHidden();
    await expect(page.locator('[data-testid="files-sidebar-row"]').first()).toBeVisible();
  });

  test('Step 4: EditorPane loading state — iframe `ready` never arrives', async ({
    tauri,
    page,
  }) => {
    await bootstrapSession(tauri);
    // Tree returns a single file we can click.
    await tauri.onInvoke('tree', async () => ({
      path: WORKSPACE,
      name: 'ws',
      kind: 'Dir',
      children: [
        { path: `${WORKSPACE}/notes.txt`, name: 'notes.txt', kind: 'File' },
      ],
    }));
    // read_file is registered but irrelevant — the iframe never reaches
    // `ready`, so EditorPane stays in `editor-pane-loading`.
    await tauri.onInvoke('read_file', async () => ({ content: '' }));

    await page.goto(`/session/${SESSION_ID}`);
    await page.keyboard.press('Control+Shift+E');
    await expect(page.locator('[data-testid="files-sidebar-row"]').first()).toBeVisible();
    // Double-click opens the file -> EditorPane mounts.
    await page.locator('[data-testid="files-sidebar-row"]').first().dblclick();

    const loading = page.getByTestId('editor-pane-loading');
    await expect(loading).toBeVisible();
    await expect(loading).toHaveAttribute('role', 'status');
  });

  test('Step 5: EditorPane file-loading state — `read_file` never resolves after iframe ready', async ({
    tauri,
    page,
  }) => {
    await bootstrapSession(tauri);
    await tauri.onInvoke('tree', async () => ({
      path: WORKSPACE,
      name: 'ws',
      kind: 'Dir',
      children: [
        { path: `${WORKSPACE}/notes.txt`, name: 'notes.txt', kind: 'File' },
      ],
    }));
    const filePending = deferred<{ content: string }>();
    await tauri.onInvoke('read_file', async () => filePending.promise);

    await page.goto(`/session/${SESSION_ID}`);
    await page.keyboard.press('Control+Shift+E');
    await expect(page.locator('[data-testid="files-sidebar-row"]').first()).toBeVisible();
    await page.locator('[data-testid="files-sidebar-row"]').first().dblclick();

    // EditorPane mounts in `editor-pane-loading` until the iframe posts
    // `{kind:'ready'}`. Synthesize a MessageEvent on the parent window with
    // `source` set to the iframe's contentWindow so EditorPane's
    // `event.source !== iframeRef.contentWindow` filter passes; synthetic
    // events have `origin === ''` which the EditorPane explicitly tolerates
    // (see EditorPane.tsx F-358 comment).
    const iframe = page.getByTestId('editor-pane-iframe');
    await expect(iframe).toBeAttached();
    await page.waitForFunction(() => {
      const el = document.querySelector(
        '[data-testid="editor-pane-iframe"]',
      ) as HTMLIFrameElement | null;
      return !!el && !!el.contentWindow;
    });
    await page.evaluate(() => {
      const el = document.querySelector(
        '[data-testid="editor-pane-iframe"]',
      ) as HTMLIFrameElement;
      const ev = new MessageEvent('message', {
        data: { kind: 'ready' },
        source: el.contentWindow,
      });
      window.dispatchEvent(ev);
    });

    const fileLoading = page.getByTestId('editor-pane-file-loading');
    await expect(fileLoading).toBeVisible();
    await expect(fileLoading).toHaveAttribute('role', 'status');

    filePending.resolve({ content: 'hello' });
  });

  test('Step 6: EditorPane error state — `read_file` rejects', async ({
    tauri,
    page,
  }) => {
    await bootstrapSession(tauri);
    await tauri.onInvoke('tree', async () => ({
      path: WORKSPACE,
      name: 'ws',
      kind: 'Dir',
      children: [
        { path: `${WORKSPACE}/notes.txt`, name: 'notes.txt', kind: 'File' },
      ],
    }));
    await tauri.onInvoke('read_file', async () => {
      throw new Error('read denied');
    });

    await page.goto(`/session/${SESSION_ID}`);
    await page.keyboard.press('Control+Shift+E');
    const rows = page.locator('[data-testid="files-sidebar-row"]');
    await expect(rows.first()).toBeVisible();
    await rows.first().dblclick();

    // Drive the iframe ready handshake so sendOpen() runs. Synthetic
    // MessageEvent dispatched on the parent window with source === the
    // iframe contentWindow — see Step 5 for the rationale.
    await page.waitForFunction(() => {
      const el = document.querySelector(
        '[data-testid="editor-pane-iframe"]',
      ) as HTMLIFrameElement | null;
      return !!el && !!el.contentWindow;
    });
    await page.evaluate(() => {
      const el = document.querySelector(
        '[data-testid="editor-pane-iframe"]',
      ) as HTMLIFrameElement;
      const ev = new MessageEvent('message', {
        data: { kind: 'ready' },
        source: el.contentWindow,
      });
      window.dispatchEvent(ev);
    });

    const errorBanner = page.getByTestId('editor-pane-error');
    await expect(errorBanner).toBeVisible();
    await expect(errorBanner).toHaveAttribute('role', 'alert');
    await expect(errorBanner).toContainText('read denied');

    // Note: error -> recovery via "open the next file in the same EditorPane"
    // is intentionally NOT exercised here. EditorPane's path-change effect
    // (see EditorPane.tsx createEffect on `props.path`) skips sendOpen()
    // while `currentValue === null`, which is the state after a failed
    // initial read. Production recovery is via close+reopen of the pane;
    // the LSP-error Reload affordance documented in UAT-02 step 8 is a
    // separate code path. The loading -> error transition (the F-400
    // surface area) is fully verified above.
  });

  // Steps 7-9 (TerminalPane loading / error / recovery) are not exercised
  // here. SessionWindow renders a `terminal-pane-stub` placeholder for
  // `pane_type === 'terminal'` (see SessionWindow.tsx) — TerminalPane is
  // not yet grid-wired. These transitions are covered by the F-400
  // component-unit tests (`panes/TerminalPane.test.tsx`) and will be
  // exercised end-to-end by UAT-03 once `tauri-driver` lands.
  test.skip('Steps 7-9: TerminalPane loading / error / recovery', () => {
    // When TerminalPane is grid-wired, the assertions are:
    //   * `[data-testid="terminal-pane-loading"][role="status"]` visible
    //     while `terminal_spawn` is pending.
    //   * `.terminal-pane__error[role="alert"]` visible when `terminal_spawn`
    //     rejects (instrumentation gap: no `data-testid="terminal-pane-error"`
    //     yet — see plan §UAT-09 callout).
    //   * After a successful spawn following a failure, loading clears and
    //     `[data-testid="terminal-pane-host"]` is visible.
  });
});
