import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setInvokeForTesting } from '../lib/tauri';
import {
  sessionList,
  openSession,
  providerStatus,
  listProviders,
  getActiveProvider,
  setActiveProvider,
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

  it('providerStatus invokes `provider_status` with no args', async () => {
    const status = {
      reachable: true,
      base_url: 'http://127.0.0.1:11434',
      models: ['llama3'],
      last_checked: '2026-04-22T00:00:00Z',
    };
    invokeMock.mockResolvedValue(status);

    const result = await providerStatus();

    expect(invokeMock).toHaveBeenCalledWith('provider_status', undefined);
    expect(result).toEqual(status);
  });

  it('listProviders invokes `dashboard_list_providers` with no args', async () => {
    const entries = [
      { id: 'ollama', display_name: 'Ollama', credential_required: false, has_credential: false, model_available: true },
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
});
