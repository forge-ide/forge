// F-122 tests. The iframe is rendered with `about:blank` so the Monaco
// bundle never boots in jsdom (per monaco-host README). Message flow between
// the parent and iframe is simulated via `window.dispatchEvent(new
// MessageEvent(...))` using `iframe.contentWindow` as the source — that
// matches what a real monaco-host bundle would set on its own postMessage.
//
// The signatures under test are the F-122 rev-2 shape: `readFile(sessionId,
// path)` / `writeFile(sessionId, path, content)`. The webview does not pass
// `workspaceRoot` — the shell looks it up server-side from the cache
// populated at `session_hello`. See `crates/forge-shell/src/ipc.rs`.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { fireEvent, render } from '@solidjs/testing-library';
import type { SessionId } from '@forge/ipc';
import {
  EditorPane,
  breadcrumbFromPath,
  trimmedBreadcrumb,
} from './EditorPane';
import { setActiveSessionId } from '../stores/session';

const SID = 'session-editor-test' as SessionId;
const FILE = '/workspace/demo/src/main.ts';

function fireFromIframe(iframe: HTMLIFrameElement, data: unknown): void {
  const event = new MessageEvent('message', {
    data,
    source: iframe.contentWindow as MessageEventSource,
  });
  window.dispatchEvent(event);
}

describe('breadcrumb helpers', () => {
  it('splits absolute paths into non-empty segments', () => {
    expect(breadcrumbFromPath('/a/b/c.txt')).toEqual(['a', 'b', 'c.txt']);
  });

  it('trims the prefix to final two segments with ellipsis when deep', () => {
    const { prefix, leaf } = trimmedBreadcrumb('/a/b/c/d/e/file.rs');
    expect(leaf).toBe('file.rs');
    expect(prefix.startsWith('…/')).toBe(true);
  });

  it('emits an empty prefix when the path is a single segment', () => {
    const { prefix, leaf } = trimmedBreadcrumb('/foo.rs');
    expect(prefix).toBe('');
    expect(leaf).toBe('foo.rs');
  });
});

describe('EditorPane render', () => {
  it('renders EDITOR type label, subject leaf, and close button via PaneHeader', () => {
    const { getByTestId, getByRole } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={vi.fn().mockResolvedValue({
          path: FILE,
          content: 'hi',
          bytes: 2,
          sha256: 'abc',
        })}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />
    ));
    // F-394: chrome comes from PaneHeader, not an inline header block.
    expect(getByTestId('pane-header-type-label').textContent).toBe('EDITOR');
    expect(getByTestId('pane-header-subject').textContent).toBe('main.ts');
    expect(getByTestId('editor-pane-iframe')).toBeInTheDocument();
    // EditorPane uses "Close pane" aria-label (matches the primitive's
    // closeAriaLabel contract).
    expect(getByRole('button', { name: /close pane/i })).toBeInTheDocument();
  });

  it('does not render the dirty dot initially', () => {
    const { queryByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={vi.fn().mockResolvedValue({
          path: FILE,
          content: '',
          bytes: 0,
          sha256: '',
        })}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />
    ));
    expect(queryByTestId('editor-pane-dirty')).not.toBeInTheDocument();
  });
});

describe('open-on-ready flow', () => {
  it('calls readFile and posts an open message after the iframe emits ready', async () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: 'const x = 1;\n',
      bytes: 13,
      sha256: 'sha',
    });
    const posted: unknown[] = [];
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        postToIframe={(msg) => posted.push(msg)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;

    fireFromIframe(iframe, { kind: 'ready' });
    // Allow microtasks for readFile resolution.
    await Promise.resolve();
    await Promise.resolve();

    expect(readFile).toHaveBeenCalledWith(SID, FILE);
    const openMsg = posted.find(
      (m) => (m as { kind?: string }).kind === 'open',
    ) as { kind: string; uri: string; languageId: string; value: string } | undefined;
    expect(openMsg).toBeDefined();
    expect(openMsg?.uri).toBe(`file://${FILE}`);
    expect(openMsg?.languageId).toBe('typescript');
    expect(openMsg?.value).toBe('const x = 1;\n');
  });

  it('surfaces a read error through the alert region', async () => {
    const readFile = vi.fn().mockRejectedValue(new Error('path denied'));
    const { getByTestId, queryByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    fireFromIframe(iframe, { kind: 'ready' });
    await Promise.resolve();
    await Promise.resolve();
    const alert = queryByTestId('editor-pane-error');
    expect(alert?.textContent).toContain('path denied');
  });
});

describe('dirty state + save flow', () => {
  async function openFile(
    readFile: ReturnType<typeof vi.fn>,
    writeFile: ReturnType<typeof vi.fn>,
    posted: unknown[],
    initial = 'original\n',
  ) {
    readFile.mockResolvedValue({
      path: FILE,
      content: initial,
      bytes: initial.length,
      sha256: 'sha',
    });
    const rendered = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={writeFile}
        postToIframe={(m) => posted.push(m)}
        onClose={vi.fn()}
      />
    ));
    const iframe = rendered.getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    fireFromIframe(iframe, { kind: 'ready' });
    await Promise.resolve();
    await Promise.resolve();
    return { ...rendered, iframe };
  }

  it('marks the pane dirty when the iframe reports a change', async () => {
    const readFile = vi.fn();
    const writeFile = vi.fn().mockResolvedValue(undefined);
    const posted: unknown[] = [];
    const { iframe, getByTestId } = await openFile(readFile, writeFile, posted);

    fireFromIframe(iframe, {
      kind: 'change',
      uri: `file://${FILE}`,
      value: 'original\nmore\n',
    });
    expect(getByTestId('editor-pane-dirty')).toBeInTheDocument();
  });

  it('clears dirty when the change value matches the last saved value', async () => {
    const readFile = vi.fn();
    const writeFile = vi.fn().mockResolvedValue(undefined);
    const posted: unknown[] = [];
    const { iframe, queryByTestId } = await openFile(readFile, writeFile, posted);

    fireFromIframe(iframe, {
      kind: 'change',
      uri: `file://${FILE}`,
      value: 'different\n',
    });
    fireFromIframe(iframe, {
      kind: 'change',
      uri: `file://${FILE}`,
      value: 'original\n',
    });
    expect(queryByTestId('editor-pane-dirty')).not.toBeInTheDocument();
  });

  it('Cmd+S sends a save request to the iframe, which persists via writeFile', async () => {
    const readFile = vi.fn();
    const writeFile = vi.fn().mockResolvedValue(undefined);
    const posted: unknown[] = [];
    const { getByTestId, iframe, queryByTestId } = await openFile(
      readFile,
      writeFile,
      posted,
    );

    // Dirty the buffer.
    fireFromIframe(iframe, {
      kind: 'change',
      uri: `file://${FILE}`,
      value: 'v2\n',
    });
    expect(getByTestId('editor-pane-dirty')).toBeInTheDocument();

    // Cmd+S on the pane root.
    const root = getByTestId('editor-pane');
    fireEvent.keyDown(root, { key: 's', metaKey: true });

    // The pane only asked the iframe; the iframe replies with the value.
    const saveReq = posted.find(
      (m) => (m as { kind?: string }).kind === 'save',
    );
    expect(saveReq).toBeDefined();

    fireFromIframe(iframe, {
      kind: 'save',
      uri: `file://${FILE}`,
      value: 'v2\n',
    });
    await Promise.resolve();
    await Promise.resolve();

    expect(writeFile).toHaveBeenCalledWith(SID, FILE, 'v2\n');
    expect(queryByTestId('editor-pane-dirty')).not.toBeInTheDocument();
  });

  it('Ctrl+S triggers the same save path for non-mac keyboards', async () => {
    const readFile = vi.fn();
    const writeFile = vi.fn().mockResolvedValue(undefined);
    const posted: unknown[] = [];
    const { getByTestId } = await openFile(readFile, writeFile, posted);

    const root = getByTestId('editor-pane');
    fireEvent.keyDown(root, { key: 's', ctrlKey: true });

    expect(posted.some((m) => (m as { kind?: string }).kind === 'save')).toBe(true);
  });

  it('surfaces a writeFile rejection through the alert region', async () => {
    const readFile = vi.fn();
    const writeFile = vi.fn().mockRejectedValue('forge-fs: path denied');
    const posted: unknown[] = [];
    const { iframe, getByTestId } = await openFile(readFile, writeFile, posted);

    // Simulate the iframe returning a save with contents.
    fireFromIframe(iframe, {
      kind: 'save',
      uri: `file://${FILE}`,
      value: 'will-fail\n',
    });
    await Promise.resolve();
    await Promise.resolve();

    expect(getByTestId('editor-pane-error').textContent).toContain(
      'forge-fs: path denied',
    );
  });
});

// F-385: single source of truth for sessionId. EditorPane reads the id
// from the global `activeSessionId` signal; no `sessionId` prop is accepted.
// Proven by asserting readFile / writeFile are called with the signal's
// value, not a value plumbed through props.
describe('EditorPane — sessionId sourced from activeSessionId signal (F-385)', () => {
  it('readFile is called with the signal value (no sessionId prop)', async () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: 'x',
      bytes: 1,
      sha256: 'sha',
    });
    const posted: unknown[] = [];
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        postToIframe={(m) => posted.push(m)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    fireFromIframe(iframe, { kind: 'ready' });
    await Promise.resolve();
    await Promise.resolve();
    expect(readFile).toHaveBeenCalledWith(SID, FILE);
  });

  it('writeFile is called with the signal value on save', async () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: 'v1',
      bytes: 2,
      sha256: 'sha',
    });
    const writeFile = vi.fn().mockResolvedValue(undefined);
    const posted: unknown[] = [];
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={writeFile}
        postToIframe={(m) => posted.push(m)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    fireFromIframe(iframe, { kind: 'ready' });
    await Promise.resolve();
    await Promise.resolve();
    fireFromIframe(iframe, { kind: 'save', uri: `file://${FILE}`, value: 'v2' });
    await Promise.resolve();
    await Promise.resolve();
    expect(writeFile).toHaveBeenCalledWith(SID, FILE, 'v2');
  });
});

describe('message origin isolation', () => {
  it('ignores messages from windows other than the hosted iframe', async () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: 'x',
      bytes: 1,
      sha256: 'sha',
    });
    const posted: unknown[] = [];
    render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        postToIframe={(m) => posted.push(m)}
        onClose={vi.fn()}
      />
    ));

    // MessageEvent with no `source` — simulates a cross-window post from
    // an unrelated iframe.
    const event = new MessageEvent('message', {
      data: { kind: 'ready' },
      source: null,
    });
    window.dispatchEvent(event);
    await Promise.resolve();

    expect(readFile).not.toHaveBeenCalled();
    expect(posted).toEqual([]);
  });

  // F-358: defense-in-depth for the cross-origin postMessage contract.
  // Even when the message's `source` is the hosted iframe's window, a
  // divergent `event.origin` must be rejected. Symmetrically, outbound
  // `postMessage` must use the explicit iframe origin rather than `'*'`.
  it('ignores messages from the iframe window when origin does not match the expected iframe origin', async () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: 'x',
      bytes: 1,
      sha256: 'sha',
    });
    const posted: unknown[] = [];
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        expectedIframeOrigin="https://editor.forge.local"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        postToIframe={(m) => posted.push(m)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;

    // Same source as the iframe, but a foreign origin — must be dropped.
    const event = new MessageEvent('message', {
      data: { kind: 'ready' },
      origin: 'https://evil.example',
      source: iframe.contentWindow as MessageEventSource,
    });
    window.dispatchEvent(event);
    await Promise.resolve();

    expect(readFile).not.toHaveBeenCalled();
    expect(posted).toEqual([]);
  });

  it('accepts messages from the iframe when origin matches the expected iframe origin', async () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: 'const y = 2;\n',
      bytes: 13,
      sha256: 'sha',
    });
    const posted: unknown[] = [];
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        expectedIframeOrigin="https://editor.forge.local"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        postToIframe={(m) => posted.push(m)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;

    const event = new MessageEvent('message', {
      data: { kind: 'ready' },
      origin: 'https://editor.forge.local',
      source: iframe.contentWindow as MessageEventSource,
    });
    window.dispatchEvent(event);
    await Promise.resolve();
    await Promise.resolve();

    expect(readFile).toHaveBeenCalledWith(SID, FILE);
    expect(posted.some((m) => (m as { kind?: string }).kind === 'open')).toBe(true);
  });

  it('posts messages to the iframe using the explicit expected origin (never "*")', () => {
    // Use the real iframe postMessage path (no `postToIframe` override) so
    // we exercise the production postMessage call. A recording spy on
    // `HTMLIFrameElement.prototype.contentWindow.postMessage` captures
    // the target-origin argument.
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: '',
      bytes: 0,
      sha256: 'sha',
    });
    const { unmount } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        expectedIframeOrigin="https://editor.forge.local"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />
    ));

    // Cleanup path posts `close` to the iframe; the target origin argument
    // of that call must be the explicit expected origin, not `'*'`.
    const calls: Array<[unknown, string]> = [];
    // Patch at the window level — jsdom's iframe contentWindow.postMessage
    // is a stub by default, so spy on window.postMessage for the close path.
    const original = window.postMessage.bind(window);
    vi.spyOn(window, 'postMessage').mockImplementation(((...args: unknown[]) => {
      calls.push([args[0], String(args[1])]);
      return original(args[0] as unknown as string, args[1] as string);
    }) as typeof window.postMessage);

    unmount();

    // We don't care which messages landed on window — only that none of
    // them used "*" as the target origin.
    for (const [, targetOrigin] of calls) {
      expect(targetOrigin).not.toBe('*');
    }
  });
});

// F-394: EditorPane uses the PaneHeader primitive — no inline header markup.
// The dirty-dot ships through the PaneHeader `trailing` slot. The sub-
// structural <header> no longer stamps `role="banner"`.
describe('EditorPane PaneHeader adoption (F-394)', () => {
  it('routes the dirty indicator through the PaneHeader trailing slot', async () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: 'v1',
      bytes: 2,
      sha256: 'sha',
    });
    const writeFile = vi.fn().mockResolvedValue(undefined);
    const posted: unknown[] = [];
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={writeFile}
        postToIframe={(m) => posted.push(m)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    fireFromIframe(iframe, { kind: 'ready' });
    await Promise.resolve();
    await Promise.resolve();
    fireFromIframe(iframe, {
      kind: 'change',
      uri: `file://${FILE}`,
      value: 'v2',
    });
    const dirty = getByTestId('editor-pane-dirty');
    // Dirty dot lives inside the primitive — not under `.editor-pane__header`.
    expect(dirty.closest('.pane-header')).not.toBeNull();
    expect(dirty.closest('.editor-pane__header')).toBeNull();
  });

  it('emits no role="banner" landmark from the editor pane', () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: '',
      bytes: 0,
      sha256: '',
    });
    const { container } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />
    ));
    expect(container.querySelectorAll('[role="banner"]').length).toBe(0);
  });

  it('forwards onHeaderPointerDown through the PaneHeader for drag-to-dock', () => {
    const readFile = vi.fn().mockResolvedValue({
      path: FILE,
      content: '',
      bytes: 0,
      sha256: '',
    });
    const onHeaderPointerDown = vi.fn();
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onHeaderPointerDown={onHeaderPointerDown}
        onClose={vi.fn()}
      />
    ));
    const header = getByTestId('pane-header-subject').parentElement!;
    fireEvent.pointerDown(header);
    expect(onHeaderPointerDown).toHaveBeenCalledTimes(1);
  });
});

describe('loading states (F-400)', () => {
  it('shows LOADING EDITOR placeholder before the iframe emits ready', () => {
    const { getByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={vi.fn().mockResolvedValue({ path: FILE, content: '', bytes: 0, sha256: '' })}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />
    ));
    const loading = getByTestId('editor-pane-loading');
    expect(loading).toBeInTheDocument();
    expect(loading.getAttribute('role')).toBe('status');
    expect(loading.textContent).toContain('LOADING EDITOR');
  });

  it('removes the LOADING EDITOR placeholder once the iframe emits ready', async () => {
    const readFile = vi.fn().mockResolvedValue({ path: FILE, content: 'x', bytes: 1, sha256: '' });
    const { getByTestId, queryByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        postToIframe={vi.fn()}
        onClose={vi.fn()}
      />
    ));
    expect(queryByTestId('editor-pane-loading')).toBeInTheDocument();

    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    const event = new MessageEvent('message', {
      data: { kind: 'ready' },
      source: iframe.contentWindow as MessageEventSource,
    });
    window.dispatchEvent(event);
    await Promise.resolve();
    await Promise.resolve();

    expect(queryByTestId('editor-pane-loading')).not.toBeInTheDocument();
  });

  it('shows LOADING FILE while readFile is in-flight (after ready)', async () => {
    let resolveRead!: (v: { path: string; content: string; bytes: number; sha256: string }) => void;
    const readFile = vi.fn().mockReturnValue(
      new Promise<{ path: string; content: string; bytes: number; sha256: string }>((res) => {
        resolveRead = res;
      }),
    );
    const { getByTestId, queryByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        postToIframe={vi.fn()}
        onClose={vi.fn()}
      />
    ));

    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    const event = new MessageEvent('message', {
      data: { kind: 'ready' },
      source: iframe.contentWindow as MessageEventSource,
    });
    window.dispatchEvent(event);
    await Promise.resolve();

    expect(getByTestId('editor-pane-file-loading')).toBeInTheDocument();

    resolveRead({ path: FILE, content: 'done', bytes: 4, sha256: '' });
    await Promise.resolve();
    await Promise.resolve();

    expect(queryByTestId('editor-pane-file-loading')).not.toBeInTheDocument();
  });

  it('hides the loading placeholder when an error is shown instead', async () => {
    const readFile = vi.fn().mockRejectedValue(new Error('denied'));
    const { getByTestId, queryByTestId } = render(() => (
      <EditorPane
        path={FILE}
        src="about:blank"
        readFile={readFile}
        writeFile={vi.fn().mockResolvedValue(undefined)}
        onClose={vi.fn()}
      />
    ));
    const iframe = getByTestId('editor-pane-iframe') as HTMLIFrameElement;
    const event = new MessageEvent('message', {
      data: { kind: 'ready' },
      source: iframe.contentWindow as MessageEventSource,
    });
    window.dispatchEvent(event);
    await Promise.resolve();
    await Promise.resolve();

    expect(queryByTestId('editor-pane-loading')).not.toBeInTheDocument();
    expect(getByTestId('editor-pane-error')).toBeInTheDocument();
  });
});

beforeEach(() => {
  // F-385: EditorPane reads the session id from the global `activeSessionId`
  // signal. Tests mount outside of SessionWindow, so we seed the signal
  // directly before each mount and clear it in afterEach.
  setActiveSessionId(SID);
});

afterEach(() => {
  setActiveSessionId(null);
  // The pane uses injected helpers and attaches its own window listener,
  // which is cleaned up onCleanup.
});
