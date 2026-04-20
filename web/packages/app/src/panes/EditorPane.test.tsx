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
  it('renders EDITOR type label, breadcrumb leaf, and close button', () => {
    const { getByTestId, getByText, getByRole } = render(() => (
      <EditorPane
        sessionId={SID}
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
    expect(getByText('EDITOR')).toBeInTheDocument();
    expect(getByText('main.ts')).toBeInTheDocument();
    expect(getByTestId('editor-pane-iframe')).toBeInTheDocument();
    expect(getByRole('button', { name: /close editor pane/i })).toBeInTheDocument();
  });

  it('does not render the dirty dot initially', () => {
    const { queryByTestId } = render(() => (
      <EditorPane
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
        sessionId={SID}
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
});

afterEach(() => {
  // Nothing global to reset — the pane uses injected helpers and attaches
  // its own window listener, which is cleaned up onCleanup.
});

beforeEach(() => {
  // Reset has no cross-test state today; future-proof.
});
