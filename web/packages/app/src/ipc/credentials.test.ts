import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProviderId } from '@forge/ipc';
import { setInvokeForTesting } from '../lib/tauri';
import { hasCredential, loginProvider, logoutProvider } from './credentials';

describe('credentials ipc wrappers (F-588)', () => {
  let invokeMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    invokeMock = vi.fn();
    setInvokeForTesting(invokeMock as never);
  });

  afterEach(() => {
    setInvokeForTesting(null);
  });

  it('loginProvider invokes `login_provider` with providerId + key', async () => {
    invokeMock.mockResolvedValue(undefined);

    await loginProvider('anthropic' as ProviderId, 'sk-ant-1');

    expect(invokeMock).toHaveBeenCalledWith('login_provider', {
      providerId: 'anthropic',
      key: 'sk-ant-1',
    });
  });

  it('logoutProvider invokes `logout_provider` with providerId only', async () => {
    invokeMock.mockResolvedValue(undefined);

    await logoutProvider('openai' as ProviderId);

    expect(invokeMock).toHaveBeenCalledWith('logout_provider', {
      providerId: 'openai',
    });
  });

  it('hasCredential invokes `has_credential` and returns the boolean', async () => {
    invokeMock.mockResolvedValue(true);

    const result = await hasCredential('anthropic' as ProviderId);

    expect(invokeMock).toHaveBeenCalledWith('has_credential', {
      providerId: 'anthropic',
    });
    expect(result).toBe(true);
  });

  it('hasCredential returns false when the backend reports no entry', async () => {
    invokeMock.mockResolvedValue(false);

    const result = await hasCredential('openai' as ProviderId);

    expect(result).toBe(false);
  });

  it('loginProvider propagates IPC rejections so the UI can surface errors', async () => {
    invokeMock.mockRejectedValue(new Error('forbidden: window label mismatch'));

    await expect(loginProvider('anthropic' as ProviderId, 'sk-ant-x')).rejects.toThrow(
      /forbidden/,
    );
  });
});
