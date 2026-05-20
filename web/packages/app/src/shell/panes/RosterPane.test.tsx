// RosterPane chrome tests via the AgentsPane wrapper (any of Agents /
// Skills / MCP exercises the same code path — using one is enough for
// coverage; pane-specific wiring lives in their own thin components and
// is asserted via the AppShell pane-swap tests).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router';
import { AgentsPane } from './AgentsPane';
import { setInvokeForTesting } from '../../lib/tauri';

const invoke = vi.fn();

function renderPane(workspaceRoot: string | null) {
  const history = createMemoryHistory();
  history.set({ value: '/session/abc' });
  return render(() => (
    <MemoryRouter history={history}>
      <Route
        path="/session/:id"
        component={() => <AgentsPane workspaceRoot={workspaceRoot} />}
      />
      <Route path="/catalog/:kind" component={() => <div data-testid="catalog-stub" />} />
    </MemoryRouter>
  ));
}

beforeEach(() => {
  invoke.mockReset();
  setInvokeForTesting(invoke as never);
});

afterEach(() => {
  setInvokeForTesting(null);
  cleanup();
});

describe('RosterPane (via AgentsPane)', () => {
  it('renders the no-workspace empty state when workspaceRoot is null', async () => {
    const { findByTestId } = renderPane(null);
    expect(await findByTestId('workspace-pane-agents-no-workspace')).toBeInTheDocument();
    expect(invoke).not.toHaveBeenCalledWith('list_agents', expect.anything());
  });

  it('renders the list when list_agents resolves', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_agents') {
        return [
          {
            entry: { type: 'Agent', id: 'planner', background: false },
            scope: { type: 'SessionWide' },
          },
          {
            entry: { type: 'Agent', id: 'reviewer', background: false },
            scope: { type: 'SessionWide' },
          },
        ];
      }
      return undefined;
    });

    const { findByTestId } = renderPane('/ws');
    expect(await findByTestId('workspace-pane-agents-row-planner')).toBeInTheDocument();
    expect(await findByTestId('workspace-pane-agents-row-reviewer')).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('list_agents', {
      workspaceRoot: '/ws',
      scope: { type: 'SessionWide' },
    });
  });

  it('renders the empty state when the roster is empty', async () => {
    invoke.mockImplementation(async () => []);
    const { findByTestId } = renderPane('/ws');
    expect(await findByTestId('workspace-pane-agents-empty')).toBeInTheDocument();
  });

  it('surfaces an error block when list_agents rejects', async () => {
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === 'list_agents') throw new Error('forbidden');
      return undefined;
    });
    const { findByText } = renderPane('/ws');
    expect(await findByText(/forbidden/)).toBeInTheDocument();
  });

  it('Manage button navigates to the Catalog route for the kind', async () => {
    invoke.mockImplementation(async () => []);
    const { findByTestId } = renderPane('/ws');
    const manage = await findByTestId('workspace-pane-agents-manage');
    fireEvent.click(manage);
    await waitFor(() =>
      expect((window.document.querySelector('[data-testid="catalog-stub"]'))).not.toBeNull(),
    );
  });
});
