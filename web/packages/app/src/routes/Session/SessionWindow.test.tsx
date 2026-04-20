import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, waitFor } from '@solidjs/testing-library';

const { invokeMock, listenMock, unlistenMock, closeMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
  unlistenMock: vi.fn(),
  closeMock: vi.fn(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: listenMock,
}));

vi.mock('@tauri-apps/api/window', () => ({
  getCurrentWindow: () => ({ close: closeMock }),
}));

import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router';
import {
  SessionWindow,
  __setInjectedLayoutStoreForTesting,
} from './SessionWindow';
import { resetSessionEventStore } from '../../stores/session';
import { resetMessagesStore } from '../../stores/messages';
import { setInvokeForTesting } from '../../lib/tauri';
import type { Layouts, PaneState } from '@forge/ipc';
import { createStore, produce } from 'solid-js/store';
import { defaultLayouts, type LayoutStore } from '../../layout/layoutStore';

const helloAck = {
  session_id: 'abc123',
  workspace: '/ws',
  started_at: '2026-04-18T00:00:00Z',
  event_seq: 0,
  schema_version: 1,
};

function renderAt(path: string) {
  const history = createMemoryHistory();
  history.set({ value: path });
  return render(() => (
    <MemoryRouter history={history}>
      <Route path="/session/:id" component={SessionWindow} />
    </MemoryRouter>
  ));
}

// F-126: test-only fake layout store that skips the `read_layouts` /
// `write_layouts` IPC roundtrip. Mirrors the production `LayoutStore`
// interface closely enough that SessionWindow can drive the Open ->
// EditorPane flow through it deterministically.
function makeFakeLayoutStore(): LayoutStore & { __openFileCalls: string[] } {
  // Use `createStore` so mutations notify Solid reactivity — SessionWindow's
  // `<Show when={activeEditorFile()}>` only retracks when the underlying
  // signal is reactive. A plain object would mutate silently and the
  // EditorPane would never mount.
  const [state, setState] = createStore<Layouts>(defaultLayouts());
  const calls: string[] = [];
  return {
    get layouts() {
      return state;
    },
    async load() {
      /* already seeded */
    },
    setLayouts() {
      /* no-op */
    },
    setLayout() {
      /* no-op */
    },
    setPaneState(layoutName, leafId, paneState) {
      setState('named', layoutName, 'pane_state', leafId, paneState);
    },
    setActive(name) {
      setState('active', name);
    },
    openFile(path) {
      calls.push(path ?? '<null>');
      setState(
        produce((s) => {
          const layout = s.named[s.active];
          if (!layout) return;
          const current = layout.pane_state.editor ?? ({} as PaneState);
          layout.pane_state.editor = { ...current, active_file: path };
        }),
      );
    },
    activeEditorFile() {
      const layout = state.named[state.active];
      return layout?.pane_state.editor?.active_file ?? null;
    },
    cancelPendingWrite() {
      /* no-op */
    },
    async flush() {
      /* no-op */
    },
    __openFileCalls: calls,
  };
}

function renderWithStore(path: string, store: LayoutStore) {
  __setInjectedLayoutStoreForTesting(store);
  const history = createMemoryHistory();
  history.set({ value: path });
  return render(() => (
    <MemoryRouter history={history}>
      <Route path="/session/:id" component={SessionWindow} />
    </MemoryRouter>
  ));
}

describe('SessionWindow', () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    unlistenMock.mockReset();
    closeMock.mockReset();
    resetSessionEventStore();
    resetMessagesStore();
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_hello') return helloAck;
      // F-126: layoutStore.load() calls read_layouts on mount when no store
      // is injected. Return the default layouts so SessionWindow proceeds
      // cleanly; writes are no-ops for tests that don't assert on them.
      if (cmd === 'read_layouts') return defaultLayouts();
      if (cmd === 'write_layouts') return undefined;
      // F-126: the FilesSidebar calls `tree` when the sidebar opens; return
      // a minimal empty-root shape so the component mounts cleanly.
      if (cmd === 'tree') {
        return {
          name: 'ws',
          path: '/ws',
          kind: 'Dir',
          children: [],
        };
      }
      return undefined;
    });
    setInvokeForTesting(invokeMock as never);
    listenMock.mockResolvedValue(unlistenMock);
  });

  afterEach(() => {
    setInvokeForTesting(null);
    __setInjectedLayoutStoreForTesting(null);
    cleanup();
  });

  it('renders the session id from the route', async () => {
    const { findByTestId } = renderAt('/session/abc123');
    const subject = await findByTestId('pane-header-subject');
    expect(subject.textContent).toContain('abc123');
  });

  it('calls session_hello on mount with the route-param id', async () => {
    renderAt('/session/abc123');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('session_hello', {
        sessionId: 'abc123',
      }),
    );
  });

  it('calls session_subscribe on mount after hello resolves', async () => {
    renderAt('/session/abc123');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('session_subscribe', {
        sessionId: 'abc123',
        since: 0,
      }),
    );
    // hello must run before subscribe
    const helloIdx = invokeMock.mock.calls.findIndex((c) => c[0] === 'session_hello');
    const subIdx = invokeMock.mock.calls.findIndex((c) => c[0] === 'session_subscribe');
    expect(helloIdx).toBeGreaterThanOrEqual(0);
    expect(subIdx).toBeGreaterThan(helloIdx);
  });

  it('attaches a session:event listener on mount', async () => {
    renderAt('/session/abc123');
    await waitFor(() =>
      expect(listenMock).toHaveBeenCalledWith('session:event', expect.any(Function)),
    );
  });

  it('detaches the session:event listener on unmount', async () => {
    const { unmount } = renderAt('/session/abc123');
    await waitFor(() => expect(listenMock).toHaveBeenCalled());
    unmount();
    await waitFor(() => expect(unlistenMock).toHaveBeenCalledTimes(1));
  });

  it('renders exactly one pane slot (no splitter or dock zones)', async () => {
    const { container, findByTestId } = renderAt('/session/abc123');
    await findByTestId('pane-header-subject');
    const panes = container.querySelectorAll('.session-window__pane');
    expect(panes.length).toBe(1);
    expect(container.querySelector('.session-window__splitter')).toBeNull();
    expect(container.querySelector('.session-window__dock-zone')).toBeNull();
  });

  it('pane header shows subject, ollama provider label, cost placeholder, close action', async () => {
    const { findByTestId, findByRole } = renderAt('/session/abc123');
    const subject = await findByTestId('pane-header-subject');
    expect(subject.textContent).toContain('abc123');
    const provider = await findByTestId('pane-header-provider');
    expect(provider.textContent?.toLowerCase()).toContain('ollama');
    const cost = await findByTestId('pane-header-cost');
    expect(cost.textContent).toMatch(/in\s+0.*out\s+0.*\$0/i);
    const close = await findByRole('button', { name: /close/i });
    expect(close).toBeInTheDocument();
  });

  it('close button invokes the current window close()', async () => {
    const { findByRole } = renderAt('/session/abc123');
    const close = await findByRole('button', { name: /close/i });
    close.click();
    expect(closeMock).toHaveBeenCalledTimes(1);
  });

  it('renders a ChatPane placeholder with the CHAT type label', async () => {
    const { findByTestId } = renderAt('/session/abc123');
    const chatPane = await findByTestId('chat-pane');
    expect(chatPane.textContent).toContain('CHAT');
  });

  it('calls get_persistent_approvals with the HelloAck workspace after hello resolves (F-036)', async () => {
    renderAt('/session/abc123');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('get_persistent_approvals', {
        workspaceRoot: '/ws',
      }),
    );
    // Must fire after hello (we need its `workspace` field).
    const helloIdx = invokeMock.mock.calls.findIndex((c) => c[0] === 'session_hello');
    const getIdx = invokeMock.mock.calls.findIndex(
      (c) => c[0] === 'get_persistent_approvals',
    );
    expect(helloIdx).toBeGreaterThanOrEqual(0);
    expect(getIdx).toBeGreaterThan(helloIdx);
  });

  it('seeds the approvals store from get_persistent_approvals (F-036)', async () => {
    const seedMod = await import('../../stores/approvals');
    // Mock get_persistent_approvals to return two seed entries.
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_hello') return helloAck;
      if (cmd === 'get_persistent_approvals') {
        return [
          {
            scope_key: 'tool:fs.write',
            tool_name: 'fs.write',
            label: 'this tool',
            level: 'workspace',
          },
        ];
      }
      return undefined;
    });

    seedMod.resetApprovalsStore();
    renderAt('/session/abc123');
    await waitFor(() => {
      const wl = seedMod.getApprovalWhitelist('abc123' as never);
      expect('tool:fs.write' in wl.entries).toBe(true);
    });
    const wl = seedMod.getApprovalWhitelist('abc123' as never);
    expect(wl.entries['tool:fs.write']?.level).toBe('workspace');
  });

  it('routes Rust-shaped session:event payloads through the adapter into the chat pane', async () => {
    let captured: ((ev: { payload: unknown }) => void) | null = null;
    listenMock.mockImplementation(async (_name: string, handler: (ev: { payload: unknown }) => void) => {
      captured = handler;
      return unlistenMock;
    });

    const { findByTestId } = renderAt('/session/abc123');
    await findByTestId('chat-pane');
    await waitFor(() => expect(captured).not.toBeNull());

    // Fire a real Rust-shaped user_message event — the adapter must rename
    // id → message_id and discriminate on kind so the store renders it.
    captured!({
      payload: {
        session_id: 'abc123',
        seq: 1,
        event: {
          type: 'user_message',
          id: 'u-wire-1',
          at: '2026-04-18T10:00:00Z',
          text: 'hello from the wire',
          context: [],
          branch_parent: null,
        },
      },
    });

    const list = await findByTestId('message-list');
    await waitFor(() => expect(list.textContent).toContain('hello from the wire'));
  });

  // -----------------------------------------------------------------------
  // F-126: activity bar + files sidebar
  // -----------------------------------------------------------------------

  it('renders the activity bar alongside the pane', async () => {
    const { findByTestId } = renderAt('/session/abc123');
    const bar = await findByTestId('activity-bar');
    expect(bar).toBeInTheDocument();
    expect(await findByTestId('activity-bar-files')).toBeInTheDocument();
  });

  it('keeps the files sidebar hidden by default', async () => {
    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    await findByTestId('activity-bar');
    expect(queryByTestId('files-sidebar')).toBeNull();
  });

  it('toggles the files sidebar when Cmd+Shift+E fires after the workspace is known', async () => {
    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    // Wait for session_hello -> activeWorkspaceRoot populated, and the
    // activity bar rendered.
    await findByTestId('activity-bar');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('session_hello', {
        sessionId: 'abc123',
      }),
    );

    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'E', metaKey: true, shiftKey: true }),
    );
    await findByTestId('files-sidebar');

    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'E', metaKey: true, shiftKey: true }),
    );
    await waitFor(() => expect(queryByTestId('files-sidebar')).toBeNull());
  });

  it('toggles the files sidebar when the activity bar Files button is clicked', async () => {
    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    await findByTestId('activity-bar');
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith('session_hello', {
        sessionId: 'abc123',
      }),
    );
    const files = await findByTestId('activity-bar-files');
    files.click();
    await findByTestId('files-sidebar');
    files.click();
    await waitFor(() => expect(queryByTestId('files-sidebar')).toBeNull());
  });

  // -----------------------------------------------------------------------
  // F-126 mandatory fix: Open action -> layoutStore -> EditorPane end-to-end.
  // This test is the whole reason PR #291 was closed: the previous Open
  // handler was a placeholder. Here we assert the full flow:
  //   1. User opens the Files sidebar.
  //   2. User double-clicks a file row (or context-menu Open).
  //   3. SessionWindow.onFileOpen invokes layoutStore.openFile(path).
  //   4. SessionWindow reactively mounts an EditorPane for that path in the
  //      main pane body (replacing the ChatPane fallback).
  // -----------------------------------------------------------------------

  it('routes a files-sidebar Open click through the layout store and mounts an EditorPane', async () => {
    // Arrange the `tree` IPC mock to return a workspace with one file so
    // the sidebar has something to double-click.
    const treeNode = {
      name: 'ws',
      path: '/ws',
      kind: 'Dir',
      children: [
        {
          name: 'README.md',
          path: '/ws/README.md',
          kind: 'File',
          children: null,
        },
      ],
    };
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_hello') return helloAck;
      if (cmd === 'read_layouts') return defaultLayouts();
      if (cmd === 'write_layouts') return undefined;
      if (cmd === 'tree') return treeNode;
      if (cmd === 'read_file') {
        // EditorPane.sendOpen calls `read_file` when it mounts. Return a
        // stubbed content so the pane doesn't error out.
        return { content: '# hi', bytes: 4, sha256: 'abc' };
      }
      return undefined;
    });

    const store = makeFakeLayoutStore();
    const { findByTestId, findByText, queryByTestId } = renderWithStore(
      '/session/abc123',
      store,
    );

    // Initial state: ChatPane is mounted, no EditorPane.
    await findByTestId('chat-pane');
    expect(queryByTestId('editor-pane')).toBeNull();

    // Open the Files sidebar via the activity bar.
    const filesBtn = await findByTestId('activity-bar-files');
    filesBtn.click();
    await findByTestId('files-sidebar');

    // Double-click the README row. Sidebar emits onOpen(path);
    // SessionWindow calls store.openFile(path); reactive Show swaps the
    // pane body from ChatPane to EditorPane.
    const row = await findByText('README.md');
    row.dispatchEvent(
      new MouseEvent('dblclick', { bubbles: true, cancelable: true }),
    );

    const editor = await findByTestId('editor-pane');
    expect(editor).toBeInTheDocument();
    expect(store.__openFileCalls).toContain('/ws/README.md');
    // ChatPane gone, EditorPane breadcrumb displays the path leaf.
    expect(queryByTestId('chat-pane')).toBeNull();
    const breadcrumb = await findByTestId('editor-pane-breadcrumb');
    expect(breadcrumb.textContent).toContain('README.md');
  });

  it('closing the EditorPane clears the active file and falls back to ChatPane', async () => {
    invokeMock.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_hello') return helloAck;
      if (cmd === 'read_layouts') return defaultLayouts();
      if (cmd === 'write_layouts') return undefined;
      if (cmd === 'tree') {
        return { name: 'ws', path: '/ws', kind: 'Dir', children: [] };
      }
      if (cmd === 'read_file') {
        return { content: '', bytes: 0, sha256: '' };
      }
      return undefined;
    });

    const store = makeFakeLayoutStore();
    // Pre-seed an open file so the EditorPane mounts immediately.
    store.openFile('/ws/seed.ts');

    const { findByTestId, findByRole, queryByTestId } = renderWithStore(
      '/session/abc123',
      store,
    );

    await findByTestId('editor-pane');
    expect(queryByTestId('chat-pane')).toBeNull();

    const close = await findByRole('button', { name: /close editor pane/i });
    close.click();

    await findByTestId('chat-pane');
    expect(queryByTestId('editor-pane')).toBeNull();
    expect(store.activeEditorFile()).toBeNull();
  });
});
