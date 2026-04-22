import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { setInvokeForTesting } from '../lib/tauri';
import { sessionList, openSession, providerStatus } from './dashboard';

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
});
