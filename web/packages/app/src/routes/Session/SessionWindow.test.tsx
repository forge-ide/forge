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
import { SessionWindow } from './SessionWindow';
import { resetSessionEventStore } from '../../stores/session';
import { resetMessagesStore } from '../../stores/messages';
import { setInvokeForTesting } from '../../lib/tauri';

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
      return undefined;
    });
    setInvokeForTesting(invokeMock as never);
    listenMock.mockResolvedValue(unlistenMock);
  });

  afterEach(() => {
    setInvokeForTesting(null);
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
});
