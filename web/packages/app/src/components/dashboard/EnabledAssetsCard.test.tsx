// F-724 tests — EnabledAssetsCard
//
// Covers the four interaction states (loading / empty / error / ready),
// section grouping (Skills / MCP servers / Agents), toggle behaviour
// (writes `catalog.enabled.<kind>.<id>` via `set_setting`, default true,
// store mirror reflects new value), workspace identifier rendering, and
// the no-workspace fallback.

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import type { ScopedRosterEntry } from '@forge/ipc';

// Mock the dialog plugin before importing the SUT — the real plugin throws
// outside a Tauri runtime. The mock is reset per-test via `mockOpenDialog`.
const mockOpenDialog = vi.fn();
vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (...args: unknown[]) => mockOpenDialog(...args),
}));

import {
  EnabledAssetsCard,
  workspaceShortName,
} from './EnabledAssetsCard';
import { setInvokeForTesting } from '../../lib/tauri';
import {
  activeWorkspaceRoot,
  setActiveWorkspaceRoot,
} from '../../stores/session';
import { resetSettingsStore } from '../../stores/settings';

const invokeMock = vi.fn();

const skill = (id: string): ScopedRosterEntry => ({
  entry: { type: 'Skill', id },
  scope: { type: 'SessionWide' },
});

const mcp = (id: string): ScopedRosterEntry => ({
  entry: { type: 'Mcp', id },
  scope: { type: 'SessionWide' },
});

const agent = (id: string, background = false): ScopedRosterEntry => ({
  entry: { type: 'Agent', id, background },
  scope: { type: 'SessionWide' },
});

interface SetupOpts {
  skills?: ScopedRosterEntry[];
  mcp?: ScopedRosterEntry[];
  agents?: ScopedRosterEntry[];
  listSkillsError?: string;
  setSettingError?: string;
  /** Pend a single command — its promise never resolves until the test ends. */
  pending?: boolean;
  /** Override `register_workspace`'s returned canonical path. Default echoes the input. */
  registerWorkspaceCanonical?: (picked: string) => string;
  /** Force `register_workspace` to reject with this message. */
  registerWorkspaceError?: string;
}

function setupInvoke(opts: SetupOpts = {}) {
  invokeMock.mockImplementation((cmd: string, payload?: Record<string, unknown>) => {
    if (opts.pending) return new Promise(() => undefined);
    switch (cmd) {
      case 'list_skills':
        if (opts.listSkillsError) return Promise.reject(new Error(opts.listSkillsError));
        return Promise.resolve(opts.skills ?? []);
      case 'list_mcp_servers':
        return Promise.resolve(opts.mcp ?? []);
      case 'list_agents':
        return Promise.resolve(opts.agents ?? []);
      case 'set_setting':
        if (opts.setSettingError) return Promise.reject(new Error(opts.setSettingError));
        return Promise.resolve(undefined);
      case 'register_workspace': {
        if (opts.registerWorkspaceError) {
          return Promise.reject(new Error(opts.registerWorkspaceError));
        }
        const picked = (payload?.workspaceRoot as string | undefined) ?? '';
        const canonical = opts.registerWorkspaceCanonical
          ? opts.registerWorkspaceCanonical(picked)
          : picked;
        return Promise.resolve(canonical);
      }
      default:
        return Promise.resolve(undefined);
    }
  });
}

async function flush(): Promise<void> {
  for (let i = 0; i < 6; i += 1) {
    await Promise.resolve();
  }
}

beforeEach(() => {
  invokeMock.mockReset();
  mockOpenDialog.mockReset();
  setInvokeForTesting(invokeMock as never);
  setActiveWorkspaceRoot('/home/user/acme-api');
  resetSettingsStore();
});

afterEach(() => {
  setInvokeForTesting(null);
  setActiveWorkspaceRoot(null);
  cleanup();
});

describe('<EnabledAssetsCard> (F-724)', () => {
  it('renders the card with the workspace short name in the header', async () => {
    setupInvoke();
    const { findByTestId } = render(() => <EnabledAssetsCard />);
    await flush();

    const card = await findByTestId('enabled-assets-card');
    expect(card).toBeTruthy();
    const workspace = await findByTestId('enabled-assets-workspace');
    expect(workspace.textContent).toBe('acme-api');
  });

  it('renders three section groups (Skills / MCP servers / Agents) when each has rows', async () => {
    setupInvoke({
      skills: [skill('typescript-review')],
      mcp: [mcp('github')],
      agents: [agent('refactor-bot')],
    });
    const { findByTestId, findByText } = render(() => <EnabledAssetsCard />);
    await flush();

    expect(await findByTestId('enabled-assets-group-skills')).toBeTruthy();
    expect(await findByTestId('enabled-assets-group-mcp')).toBeTruthy();
    expect(await findByTestId('enabled-assets-group-agents')).toBeTruthy();
    expect(await findByText('Skills')).toBeTruthy();
    expect(await findByText('MCP servers')).toBeTruthy();
    expect(await findByText('Agents')).toBeTruthy();
  });

  it('each row renders the entry label and a toggle switch', async () => {
    setupInvoke({ skills: [skill('typescript-review'), skill('postgres-schemata')] });
    const { findByText, findAllByRole } = render(() => <EnabledAssetsCard />);
    await flush();

    expect(await findByText('typescript-review')).toBeTruthy();
    expect(await findByText('postgres-schemata')).toBeTruthy();
    const switches = await findAllByRole('switch');
    expect(switches.length).toBe(2);
    expect(switches[0]!.getAttribute('aria-checked')).toBe('true');
  });

  it('queries each list_* command with the active workspace root + SessionWide scope', async () => {
    setupInvoke();
    render(() => <EnabledAssetsCard />);
    await flush();

    expect(invokeMock).toHaveBeenCalledWith('list_skills', {
      workspaceRoot: '/home/user/acme-api',
      scope: { type: 'SessionWide' },
    });
    expect(invokeMock).toHaveBeenCalledWith('list_mcp_servers', {
      workspaceRoot: '/home/user/acme-api',
      scope: { type: 'SessionWide' },
    });
    expect(invokeMock).toHaveBeenCalledWith('list_agents', {
      workspaceRoot: '/home/user/acme-api',
      scope: { type: 'SessionWide' },
    });
  });

  it('toggle click writes `catalog.enabled.<kind>.<id>` via set_setting', async () => {
    setupInvoke({ skills: [skill('typescript-review')] });
    const { findAllByRole } = render(() => <EnabledAssetsCard />);
    await flush();

    const switches = await findAllByRole('switch');
    const toggle = switches[0]!;
    expect((toggle as HTMLInputElement).checked).toBe(true);
    fireEvent.click(toggle);
    await flush();

    expect(invokeMock).toHaveBeenCalledWith('set_setting', {
      key: 'catalog.enabled.skills.typescript-review',
      value: false,
      level: 'user',
      workspaceRoot: '/home/user/acme-api',
    });
  });

  it('toggle round-trip: store mirror reflects the new value', async () => {
    setupInvoke({ skills: [skill('typescript-review')] });
    const { findAllByRole } = render(() => <EnabledAssetsCard />);
    await flush();

    const toggle = (await findAllByRole('switch'))[0]! as HTMLInputElement;
    expect(toggle.checked).toBe(true);

    fireEvent.click(toggle);
    await flush();

    const refreshed = (await findAllByRole('switch'))[0]! as HTMLInputElement;
    expect(refreshed.checked).toBe(false);
    expect(refreshed.getAttribute('aria-checked')).toBe('false');
  });

  it('writes value=true when re-enabling a previously disabled row', async () => {
    setupInvoke({ skills: [skill('typescript-review')] });
    const { findAllByRole } = render(() => <EnabledAssetsCard />);
    await flush();

    // First click: enabled → disabled.
    fireEvent.click((await findAllByRole('switch'))[0]!);
    await flush();
    // Second click: disabled → enabled.
    fireEvent.click((await findAllByRole('switch'))[0]!);
    await flush();

    expect(invokeMock).toHaveBeenCalledWith('set_setting', {
      key: 'catalog.enabled.skills.typescript-review',
      value: true,
      level: 'user',
      workspaceRoot: '/home/user/acme-api',
    });
  });

  it('surfaces set_setting rejection inline and leaves the row visible', async () => {
    setupInvoke({
      skills: [skill('typescript-review')],
      setSettingError: 'invalid value',
    });
    const { findAllByRole, findByText } = render(() => <EnabledAssetsCard />);
    await flush();

    fireEvent.click((await findAllByRole('switch'))[0]!);
    await flush();

    expect(await findByText(/set_setting failed: invalid value/)).toBeTruthy();
  });

  // ---- four states ----

  it('state: loading — renders the row-shaped skeleton', async () => {
    setupInvoke({ pending: true });
    const { findByTestId } = render(() => <EnabledAssetsCard />);
    const loading = await findByTestId('enabled-assets-loading');
    expect(loading.getAttribute('role')).toBe('status');
    expect(loading.getAttribute('aria-busy')).toBe('true');
  });

  it('state: empty — renders the verbatim "Nothing enabled" copy when every list is empty', async () => {
    setupInvoke({ skills: [], mcp: [], agents: [] });
    const { findByTestId } = render(() => <EnabledAssetsCard />);
    await flush();

    const empty = await findByTestId('enabled-assets-empty');
    expect(empty.textContent).toBe('Nothing enabled in this workspace yet.');
  });

  it('state: error — renders the error block with RETRY when a list_* rejects', async () => {
    setupInvoke({ listSkillsError: 'workspace_root not in registry: /home/user/acme-api' });
    const { findByText } = render(() => <EnabledAssetsCard />);
    // Resource rejection transitions through `loading=true` → errored; needs
    // an extra macrotask beyond the microtask flush.
    await new Promise((r) => setTimeout(r, 0));
    await flush();

    expect(await findByText('ENABLED EXTENSIONS UNAVAILABLE')).toBeTruthy();
    expect(await findByText(/workspace_root not in registry/)).toBeTruthy();
    expect(await findByText('RETRY')).toBeTruthy();
  });

  it('state: ready — populated card with rows under each non-empty section', async () => {
    setupInvoke({
      skills: [skill('typescript-review')],
      mcp: [mcp('github')],
      agents: [agent('refactor-bot')],
    });
    const { findByTestId } = render(() => <EnabledAssetsCard />);
    await flush();

    expect(await findByTestId('enabled-row-skills-typescript-review')).toBeTruthy();
    expect(await findByTestId('enabled-row-mcp-github')).toBeTruthy();
    expect(await findByTestId('enabled-row-agents-refactor-bot')).toBeTruthy();
  });

  // ---- no-workspace fallback ----

  it('renders the no-workspace empty state with an Open workspace CTA when activeWorkspaceRoot is null', async () => {
    setActiveWorkspaceRoot(null);
    setupInvoke();
    const { findByTestId, queryByTestId } = render(() => <EnabledAssetsCard />);
    await flush();

    const empty = await findByTestId('enabled-assets-no-workspace');
    // Copy mentions "create" so the user knows the picker supports both
    // existing folders and creating new ones — single CTA covers both intents.
    expect(empty.textContent).toContain('No workspace open');
    expect(empty.textContent).toContain('create a new one');
    expect(queryByTestId('enabled-assets-open-workspace')).not.toBeNull();
    expect(queryByTestId('enabled-assets-workspace')).toBeNull();
    // No IPCs fire when there's no workspace.
    expect(invokeMock).not.toHaveBeenCalledWith('list_skills', expect.anything());
  });

  // ---- picker → register_workspace seam ----
  //
  // Regression coverage for the bug where the dashboard's "Open workspace"
  // CTA set activeWorkspaceRoot without seeding workspaces.toml, so every
  // downstream list_* call tripped resolve_workspace_root_for_command's
  // registry gate ("workspace_root not in registry").

  it('Open workspace CTA registers the picked path before publishing it as active', async () => {
    setActiveWorkspaceRoot(null);
    mockOpenDialog.mockResolvedValue('/home/user/picked-ws');
    setupInvoke();

    const { findByTestId } = render(() => <EnabledAssetsCard />);
    const cta = await findByTestId('enabled-assets-open-workspace');
    fireEvent.click(cta);
    await flush();
    // Native dialog → register_workspace → setActiveWorkspaceRoot is a
    // promise chain; one extra macrotask lets it settle before assertions.
    await new Promise((r) => setTimeout(r, 0));
    await flush();

    expect(invokeMock).toHaveBeenCalledWith('register_workspace', {
      workspaceRoot: '/home/user/picked-ws',
    });
    expect(activeWorkspaceRoot()).toBe('/home/user/picked-ws');
  });

  it('persists the canonical path returned by register_workspace, not the raw picker output', async () => {
    setActiveWorkspaceRoot(null);
    mockOpenDialog.mockResolvedValue('/home/user/symlink-ws');
    setupInvoke({
      registerWorkspaceCanonical: () => '/home/user/canonical-ws',
    });

    const { findByTestId } = render(() => <EnabledAssetsCard />);
    fireEvent.click(await findByTestId('enabled-assets-open-workspace'));
    await flush();
    await new Promise((r) => setTimeout(r, 0));
    await flush();

    expect(activeWorkspaceRoot()).toBe('/home/user/canonical-ws');
  });

  it('surfaces register_workspace rejection and leaves activeWorkspaceRoot unset', async () => {
    setActiveWorkspaceRoot(null);
    mockOpenDialog.mockResolvedValue('/home/user/picked-ws');
    setupInvoke({
      registerWorkspaceError: 'workspace_root not found on disk: nope',
    });

    const { findByTestId, findByText } = render(() => <EnabledAssetsCard />);
    fireEvent.click(await findByTestId('enabled-assets-open-workspace'));
    await flush();
    await new Promise((r) => setTimeout(r, 0));
    await flush();

    expect(await findByText(/register_workspace failed/)).toBeTruthy();
    expect(activeWorkspaceRoot()).toBeNull();
    // No list_* call should fire either — the active root never flipped.
    expect(invokeMock).not.toHaveBeenCalledWith('list_skills', expect.anything());
  });

  it('does not call register_workspace when the picker is cancelled', async () => {
    setActiveWorkspaceRoot(null);
    mockOpenDialog.mockResolvedValue(null);
    setupInvoke();

    const { findByTestId } = render(() => <EnabledAssetsCard />);
    fireEvent.click(await findByTestId('enabled-assets-open-workspace'));
    await flush();
    await new Promise((r) => setTimeout(r, 0));
    await flush();

    expect(invokeMock).not.toHaveBeenCalledWith(
      'register_workspace',
      expect.anything(),
    );
    expect(activeWorkspaceRoot()).toBeNull();
  });
});

describe('workspaceShortName', () => {
  it('returns the last path segment', () => {
    expect(workspaceShortName('/home/user/acme-api')).toBe('acme-api');
  });
  it('trims trailing slashes', () => {
    expect(workspaceShortName('/home/user/acme-api/')).toBe('acme-api');
  });
  it('handles Windows separators', () => {
    expect(workspaceShortName('C:\\code\\acme-api')).toBe('acme-api');
  });
  it('returns the empty string for null', () => {
    expect(workspaceShortName(null)).toBe('');
  });
});
