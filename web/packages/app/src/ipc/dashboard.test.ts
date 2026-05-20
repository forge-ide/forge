import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setInvokeForTesting } from '../lib/tauri';
import {
  sessionList,
  openSession,
  listProviders,
  getActiveProvider,
  setActiveProvider,
  gitBranch,
} from './dashboard';

describe('dashboard ipc wrappers (F-365)', () => {
  let invokeMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    invokeMock = vi.fn();
    setInvokeForTesting(invokeMock as never);
  });

  afterEach(() => {
    setInvokeForTesting(null);
  });

  it('sessionList invokes `session_list` with no args', async () => {
    invokeMock.mockResolvedValue([]);

    const result = await sessionList();

    expect(invokeMock).toHaveBeenCalledWith('session_list', undefined);
    expect(result).toEqual([]);
  });

  it('openSession invokes `open_session` with id', async () => {
    invokeMock.mockResolvedValue(undefined);

    await openSession('sess-abc');

    expect(invokeMock).toHaveBeenCalledWith('open_session', { id: 'sess-abc' });
  });

  it('listProviders invokes `dashboard_list_providers` with no args', async () => {
    const entries = [
      { id: 'anthropic', display_name: 'Anthropic', credential_required: true, has_credential: true, model_available: true },
    ];
    invokeMock.mockResolvedValue(entries);

    const result = await listProviders();

    expect(invokeMock).toHaveBeenCalledWith('dashboard_list_providers', undefined);
    expect(result).toEqual(entries);
  });

  it('getActiveProvider invokes `get_active_provider` with no args', async () => {
    invokeMock.mockResolvedValue('anthropic');

    const result = await getActiveProvider();

    expect(invokeMock).toHaveBeenCalledWith('get_active_provider', undefined);
    expect(result).toBe('anthropic');
  });

  it('setActiveProvider forwards providerId only', async () => {
    invokeMock.mockResolvedValue(undefined);

    await setActiveProvider('anthropic');

    expect(invokeMock).toHaveBeenCalledWith('set_active_provider', {
      providerId: 'anthropic',
    });
  });

  it('gitBranch returns the branch name when the IPC resolves with one', async () => {
    invokeMock.mockResolvedValue({ branch: 'main' });
    expect(await gitBranch('/ws')).toBe('main');
    expect(invokeMock).toHaveBeenCalledWith('git_branch', {
      input: { workspace_root: '/ws' },
    });
  });

  it('gitBranch returns null when the response branch is null (detached HEAD)', async () => {
    invokeMock.mockResolvedValue({ branch: null });
    expect(await gitBranch('/ws')).toBeNull();
  });

  it('gitBranch returns null when the IPC resolves to undefined (defensive)', async () => {
    // Regression guard for the TypeError surfaced in jsdom: a test stub
    // (or a misshaped daemon response) that resolves to `undefined`
    // used to crash inside the wrapper's `.branch` destructure. The
    // wrapper now optional-chains the access and returns `null`.
    invokeMock.mockResolvedValue(undefined);
    expect(await gitBranch('/ws')).toBeNull();
  });
});
