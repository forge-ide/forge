import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router';
import { ChatPane } from './ChatPane';
import { setInvokeForTesting } from '../../lib/tauri';

const invoke = vi.fn();

function renderInRoute(path: string, workspaceRoot: string | null = '/ws') {
  const history = createMemoryHistory();
  history.set({ value: path });
  return render(() => (
    <MemoryRouter history={history}>
      <Route
        path="/session/:id"
        component={() => <ChatPane workspaceRoot={workspaceRoot} />}
      />
    </MemoryRouter>
  ));
}

function makeRow(
  id: string,
  workspaceRoot: string,
  overrides: Partial<{ subject: string; state: string }> = {},
) {
  return {
    id,
    subject: overrides.subject ?? id,
    state: overrides.state ?? 'active',
    persistence: 'persist',
    createdAt: '2026-01-01T00:00:00Z',
    lastEventAt: '2026-01-01T00:00:00Z',
    workspaceRoot,
    workspaceId: 'ws01',
  };
}

beforeEach(() => {
  invoke.mockReset();
  setInvokeForTesting(invoke as never);
});

afterEach(() => {
  setInvokeForTesting(null);
  cleanup();
});

describe('ChatPane (workspace sessions)', () => {
  it('renders the rows whose workspaceRoot matches', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_list') {
        return [
          makeRow('abc123', '/ws', { subject: 'Refactor auth' }),
          makeRow('def456', '/ws', { subject: 'Doc cleanup', state: 'stopped' }),
          makeRow('xyz789', '/other', { subject: 'Different workspace' }),
        ];
      }
      return undefined;
    });

    const { findByTestId, queryByTestId } = renderInRoute('/session/abc123');
    expect(await findByTestId('workspace-pane-chat-row-abc123')).toBeInTheDocument();
    expect(await findByTestId('workspace-pane-chat-row-def456')).toBeInTheDocument();
    expect(queryByTestId('workspace-pane-chat-row-xyz789')).toBeNull();
  });

  it('marks the row matching the URL :id as active', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_list') return [makeRow('abc123', '/ws')];
      return undefined;
    });
    const { findByTestId } = renderInRoute('/session/abc123');
    const row = await findByTestId('workspace-pane-chat-row-abc123');
    expect(row.getAttribute('aria-current')).toBe('true');
  });

  it('clicking another row navigates the router (no open_session call)', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_list') {
        return [makeRow('abc123', '/ws'), makeRow('def456', '/ws')];
      }
      return undefined;
    });
    const { findByTestId } = renderInRoute('/session/abc123');
    fireEvent.click(await findByTestId('workspace-pane-chat-row-def456'));
    // The IPC must NOT be touched — router navigation stays in-window.
    await waitFor(() =>
      expect(invoke).not.toHaveBeenCalledWith('open_session', expect.anything()),
    );
  });

  it('renders the empty state when no row matches workspaceRoot', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_list') return [makeRow('xyz789', '/other')];
      return undefined;
    });
    const { findByTestId } = renderInRoute('/session/abc123');
    expect(await findByTestId('workspace-pane-chat-empty')).toBeInTheDocument();
  });

  it('renders the no-workspace empty state when workspaceRoot is null', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'session_list') return [makeRow('abc123', '/ws')];
      return undefined;
    });
    const { findByTestId } = renderInRoute('/session/abc123', null);
    expect(await findByTestId('workspace-pane-chat-empty')).toBeInTheDocument();
  });
});
