// F-716: AppShell route-mount tests. AppShell wraps every top-level route
// and supplies the StatusBar on all routes plus a session-scoped
// ActivityBar + FilesSidebar pair. These tests pin the contract: StatusBar
// is route-agnostic; ActivityBar + FilesSidebar mount only on the session
// route (FilesSidebar additionally requires activeWorkspaceRoot + an
// activeOpenFile callback).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router';
import { AppShell } from './AppShell';
import {
  setActiveOpenFile,
  setActiveSessionId,
  setActiveWorkspaceRoot,
} from '../stores/session';
import { setInvokeForTesting } from '../lib/tauri';

// StatusBar mounts on every route; its bg-agents subscriber attaches a
// `session:event` listener via Tauri. Stub the event channel to a no-op
// unlisten so jsdom never reaches the missing Tauri bridge.
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => undefined),
}));

// CommandPalette mounts a global keydown listener; nothing else needed.

const DASHBOARD_TESTID = 'route-content-dashboard';
const SESSION_TESTID = 'route-content-session';
const USAGE_TESTID = 'route-content-usage';

function Stub(testid: string) {
  return () => <div data-testid={testid}>{testid}</div>;
}

function renderAt(path: string) {
  const history = createMemoryHistory();
  history.set({ value: path });
  return render(() => (
    <MemoryRouter history={history} root={AppShell}>
      <Route path="/" component={Stub(DASHBOARD_TESTID)} />
      <Route path="/session/:id" component={Stub(SESSION_TESTID)} />
      <Route path="/usage" component={Stub(USAGE_TESTID)} />
    </MemoryRouter>
  ));
}

describe('AppShell', () => {
  beforeEach(() => {
    // StatusBar's bg-agents snapshot call goes through `invoke`. Hermetic
    // stub keeps mount deterministic. `get_settings` covers the settings
    // store seeding triggered by surrounding initialization.
    setInvokeForTesting(
      (async () => undefined) as never,
    );
  });

  afterEach(() => {
    setInvokeForTesting(null);
    setActiveSessionId(null);
    setActiveWorkspaceRoot(null);
    setActiveOpenFile(null);
    cleanup();
  });

  it('hides the ActivityBar on the Dashboard route', async () => {
    const { findByTestId, queryByTestId } = renderAt('/');
    expect(await findByTestId(DASHBOARD_TESTID)).toBeInTheDocument();
    expect(queryByTestId('activity-bar')).toBeNull();
  });

  it('renders the StatusBar on the Dashboard route', async () => {
    const { findByTestId } = renderAt('/');
    expect(await findByTestId('status-bar')).toBeInTheDocument();
  });

  it('renders the ActivityBar and StatusBar on the Session route', async () => {
    const { findByTestId } = renderAt('/session/abc123');
    expect(await findByTestId('activity-bar')).toBeInTheDocument();
    expect(await findByTestId('status-bar')).toBeInTheDocument();
    expect(await findByTestId(SESSION_TESTID)).toBeInTheDocument();
  });

  it('mounts the primary Sidebar on the Dashboard route', async () => {
    const { findByTestId } = renderAt('/');
    expect(await findByTestId(DASHBOARD_TESTID)).toBeInTheDocument();
    expect(await findByTestId('sidebar')).toBeInTheDocument();
  });

  it('hides the primary Sidebar on the Session route', async () => {
    // The Sidebar's nav entries all point at dashboard routes
    // (/providers, /catalog, /usage) whose IPCs reject a session-*
    // webview label. Mounting it in the session window leaves the user
    // with "links" that all error on click; explicit gate keeps it out.
    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    expect(await findByTestId(SESSION_TESTID)).toBeInTheDocument();
    expect(queryByTestId('sidebar')).toBeNull();
  });

  it('hides the ActivityBar on the Usage route', async () => {
    const { findByTestId, queryByTestId } = renderAt('/usage');
    expect(await findByTestId('status-bar')).toBeInTheDocument();
    expect(await findByTestId(USAGE_TESTID)).toBeInTheDocument();
    expect(queryByTestId('activity-bar')).toBeNull();
  });

  it('renders the matched route content inside the shell', async () => {
    const { findByTestId } = renderAt('/usage');
    const shell = await findByTestId('app-shell');
    const content = await findByTestId(USAGE_TESTID);
    expect(shell.contains(content)).toBe(true);
  });

  it('does not mount FilesSidebar on non-session routes', async () => {
    setActiveWorkspaceRoot('/ws');
    const { findByTestId, queryByTestId } = renderAt('/');
    await findByTestId('app-shell');
    // FilesSidebar is workspace chrome — it must not appear on Dashboard
    // even when the workspace bridge happens to be populated from a
    // previous session.
    expect(queryByTestId('files-sidebar')).toBeNull();
  });

  it('keeps FilesSidebar hidden on the session route until the Files activity is selected', async () => {
    setActiveWorkspaceRoot('/ws');
    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    await findByTestId('activity-bar');
    expect(queryByTestId('files-sidebar')).toBeNull();
  });

  it('mounts FilesSidebar on the session route when Files is selected and workspace is known', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    setActiveOpenFile(() => () => undefined);
    // FilesSidebar reads the tree via Tauri on mount; stub the response so
    // it renders cleanly.
    setInvokeForTesting(
      (async (cmd: string) => {
        if (cmd === 'tree') {
          return { name: 'ws', path: '/ws', kind: 'Dir', children: [] };
        }
        return undefined;
      }) as never,
    );

    const { findByTestId } = renderAt('/session/abc123');
    const filesBtn = await findByTestId('activity-bar-files');
    filesBtn.click();
    expect(await findByTestId('files-sidebar')).toBeInTheDocument();
  });

  it('toggles FilesSidebar via the Cmd/Ctrl+Shift+E shortcut on the session route', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    setActiveOpenFile(() => () => undefined);
    setInvokeForTesting(
      (async (cmd: string) => {
        if (cmd === 'tree') {
          return { name: 'ws', path: '/ws', kind: 'Dir', children: [] };
        }
        return undefined;
      }) as never,
    );

    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    await findByTestId('activity-bar');

    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'E', metaKey: true, shiftKey: true }),
    );
    await findByTestId('files-sidebar');

    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'E', metaKey: true, shiftKey: true }),
    );
    await waitFor(() => expect(queryByTestId('files-sidebar')).toBeNull());
  });

  it('does not surface FilesSidebar on the dashboard since the ActivityBar is not mounted there', async () => {
    setActiveWorkspaceRoot('/ws');
    const { findByTestId, queryByTestId } = renderAt('/');
    await findByTestId('app-shell');
    // ActivityBar is gated behind `isSessionRoute()` — there is no Files
    // toggle on Dashboard, so the workspace tree cannot mount.
    expect(queryByTestId('activity-bar-files')).toBeNull();
    expect(queryByTestId('files-sidebar')).toBeNull();
  });

  // Activity-bar pane swap — each entry opens the matching sidebar pane,
  // clicking the active entry closes it.

  it('opens the Chat pane when the Chat activity is clicked', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    const { findByTestId } = renderAt('/session/abc123');
    const btn = await findByTestId('activity-bar-chat');
    btn.click();
    expect(await findByTestId('workspace-pane-chat')).toBeInTheDocument();
  });

  it('opens the Agents pane when the Agents activity is clicked', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    const { findByTestId } = renderAt('/session/abc123');
    const btn = await findByTestId('activity-bar-agents');
    btn.click();
    expect(await findByTestId('workspace-pane-agents')).toBeInTheDocument();
  });

  it('opens the Skills pane when the Skills activity is clicked', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    const { findByTestId } = renderAt('/session/abc123');
    const btn = await findByTestId('activity-bar-skills');
    btn.click();
    expect(await findByTestId('workspace-pane-skills')).toBeInTheDocument();
  });

  it('opens the MCP pane when the MCP activity is clicked', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    const { findByTestId } = renderAt('/session/abc123');
    const btn = await findByTestId('activity-bar-mcp');
    btn.click();
    expect(await findByTestId('workspace-pane-mcp')).toBeInTheDocument();
  });

  it('clicking the active activity closes its pane (toggle semantics)', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    const btn = await findByTestId('activity-bar-chat');
    btn.click();
    await findByTestId('workspace-pane-chat');
    btn.click();
    await waitFor(() =>
      expect(queryByTestId('workspace-pane-chat')).toBeNull(),
    );
  });

  it('switching activities swaps the pane (one pane visible at a time)', async () => {
    setActiveSessionId('abc123' as never);
    setActiveWorkspaceRoot('/ws');
    const { findByTestId, queryByTestId } = renderAt('/session/abc123');
    (await findByTestId('activity-bar-chat')).click();
    await findByTestId('workspace-pane-chat');

    (await findByTestId('activity-bar-skills')).click();
    await findByTestId('workspace-pane-skills');
    // Old pane is gone.
    expect(queryByTestId('workspace-pane-chat')).toBeNull();
  });
});
