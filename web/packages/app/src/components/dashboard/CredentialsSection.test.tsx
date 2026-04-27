import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { setInvokeForTesting } from '../../lib/tauri';
import {
  CREDENTIAL_PROVIDERS,
  CredentialBanner,
  CredentialsSection,
} from './CredentialsSection';

type InvokeMock = ReturnType<typeof vi.fn>;

/**
 * Build an invoke mock that routes credential commands through a stateful map
 * so login/logout/has act on the same store. Anything unrecognised returns
 * undefined so latent calls fail visibly.
 */
function buildCredentialInvoke(seed: Record<string, boolean> = {}): {
  fn: InvokeMock;
  store: Record<string, boolean>;
} {
  const store: Record<string, boolean> = { ...seed };
  const fn = vi.fn(async (cmd: string, args?: Record<string, unknown>) => {
    if (cmd === 'has_credential') {
      const id = args?.['providerId'] as string;
      return store[id] === true;
    }
    if (cmd === 'login_provider') {
      const id = args?.['providerId'] as string;
      store[id] = true;
      return undefined;
    }
    if (cmd === 'logout_provider') {
      const id = args?.['providerId'] as string;
      delete store[id];
      return undefined;
    }
    return undefined;
  });
  return { fn, store };
}

describe('CredentialsSection (F-588)', () => {
  beforeEach(() => {
    const { fn } = buildCredentialInvoke();
    setInvokeForTesting(fn as never);
  });

  afterEach(() => {
    setInvokeForTesting(null);
    cleanup();
  });

  it('renders one row per credential-supporting provider', async () => {
    const { findByTestId } = render(() => <CredentialsSection />);
    for (const p of CREDENTIAL_PROVIDERS) {
      const row = await findByTestId(`credential-row-${p.id}`);
      expect(row).toBeTruthy();
    }
  });

  it('shows the missing indicator when has_credential returns false', async () => {
    const { findByTestId } = render(() => <CredentialsSection />);
    const row = await findByTestId('credential-row-anthropic');
    await waitFor(() => {
      const indicator = row.querySelector('.credentials-section__indicator--missing');
      expect(indicator).toBeTruthy();
    });
  });

  it('shows the stored indicator when has_credential returns true', async () => {
    const { fn } = buildCredentialInvoke({ anthropic: true });
    setInvokeForTesting(fn as never);

    const { findByTestId } = render(() => <CredentialsSection />);
    const row = await findByTestId('credential-row-anthropic');
    await waitFor(() => {
      const indicator = row.querySelector('.credentials-section__indicator--stored');
      expect(indicator).toBeTruthy();
    });
  });

  it('uses a password input — never echoes the typed value', async () => {
    const { findByTestId } = render(() => <CredentialsSection />);
    const input = (await findByTestId('credential-input-anthropic')) as HTMLInputElement;
    expect(input.type).toBe('password');
  });

  it('typing + submitting on a fresh provider stores the key and clears the input', async () => {
    const { fn, store } = buildCredentialInvoke();
    setInvokeForTesting(fn as never);

    const { findByTestId } = render(() => <CredentialsSection />);
    const input = (await findByTestId('credential-input-anthropic')) as HTMLInputElement;
    const submit = (await findByTestId('credential-submit-anthropic')) as HTMLButtonElement;

    fireEvent.input(input, { target: { value: 'sk-ant-1' } });
    fireEvent.click(submit);

    await waitFor(() => {
      expect(store['anthropic']).toBe(true);
    });
    // The password input has been cleared so the runtime heap no longer
    // holds a copy of the typed value.
    await waitFor(() => {
      expect(input.value).toBe('');
    });
    // IPC was called with the typed key.
    expect(fn).toHaveBeenCalledWith('login_provider', {
      providerId: 'anthropic',
      key: 'sk-ant-1',
    });
  });

  it('rotation flow opens a confirmation modal before overwriting an existing key', async () => {
    const { fn } = buildCredentialInvoke({ anthropic: true });
    setInvokeForTesting(fn as never);

    const { findByTestId, queryByTestId } = render(() => <CredentialsSection />);
    const input = (await findByTestId('credential-input-anthropic')) as HTMLInputElement;
    const submit = await findByTestId('credential-submit-anthropic');

    // Wait until the row has resolved the existing-credential state.
    await waitFor(() => {
      expect(queryByTestId('credential-logout-anthropic')).toBeTruthy();
    });

    fireEvent.input(input, { target: { value: 'sk-ant-NEW' } });
    fireEvent.click(submit);

    // Modal appears — IPC has NOT been called for login yet.
    const modal = await findByTestId('credential-rotation-modal');
    expect(modal).toBeTruthy();
    expect(
      fn.mock.calls.find(
        (c) => c[0] === 'login_provider',
      ),
    ).toBeUndefined();
  });

  it('rotation confirm calls login_provider with the new key', async () => {
    const { fn, store } = buildCredentialInvoke({ anthropic: true });
    setInvokeForTesting(fn as never);

    const { findByTestId, queryByTestId } = render(() => <CredentialsSection />);
    const input = (await findByTestId('credential-input-anthropic')) as HTMLInputElement;
    const submit = await findByTestId('credential-submit-anthropic');

    await waitFor(() => {
      expect(queryByTestId('credential-logout-anthropic')).toBeTruthy();
    });

    fireEvent.input(input, { target: { value: 'sk-ant-NEW' } });
    fireEvent.click(submit);

    const confirm = await findByTestId('credential-rotation-confirm');
    fireEvent.click(confirm);

    await waitFor(() => {
      expect(
        fn.mock.calls.some(
          (c) => c[0] === 'login_provider' && (c[1] as { key: string }).key === 'sk-ant-NEW',
        ),
      ).toBe(true);
    });
    expect(store['anthropic']).toBe(true);
  });

  it('rotation cancel discards the pending value — no IPC call', async () => {
    const { fn } = buildCredentialInvoke({ anthropic: true });
    setInvokeForTesting(fn as never);

    const { findByTestId, queryByTestId } = render(() => <CredentialsSection />);
    const input = (await findByTestId('credential-input-anthropic')) as HTMLInputElement;
    const submit = await findByTestId('credential-submit-anthropic');

    await waitFor(() => {
      expect(queryByTestId('credential-logout-anthropic')).toBeTruthy();
    });

    fireEvent.input(input, { target: { value: 'sk-ant-NEW' } });
    fireEvent.click(submit);

    const cancel = await findByTestId('credential-rotation-cancel');
    fireEvent.click(cancel);

    await waitFor(() => {
      expect(queryByTestId('credential-rotation-modal')).toBeNull();
    });
    expect(
      fn.mock.calls.some((c) => c[0] === 'login_provider'),
    ).toBe(false);
  });

  it('logout button calls logout_provider and flips the indicator', async () => {
    const { fn, store } = buildCredentialInvoke({ anthropic: true });
    setInvokeForTesting(fn as never);

    const { findByTestId } = render(() => <CredentialsSection />);
    const logout = await findByTestId('credential-logout-anthropic');

    fireEvent.click(logout);

    await waitFor(() => {
      expect(store['anthropic']).toBeUndefined();
    });
    expect(fn).toHaveBeenCalledWith('logout_provider', { providerId: 'anthropic' });
  });

  it('surfaces an IPC failure in an alert without crashing', async () => {
    const failingInvoke = vi.fn(async (cmd: string) => {
      if (cmd === 'has_credential') return false;
      if (cmd === 'login_provider') throw new Error('keyring unavailable');
      return undefined;
    });
    setInvokeForTesting(failingInvoke as never);

    const { findByTestId } = render(() => <CredentialsSection />);
    const input = (await findByTestId('credential-input-anthropic')) as HTMLInputElement;
    const submit = await findByTestId('credential-submit-anthropic');

    fireEvent.input(input, { target: { value: 'sk-ant-bad' } });
    fireEvent.click(submit);

    const err = await findByTestId('credential-error-anthropic');
    expect(err.textContent).toContain('keyring unavailable');
  });

  it('async submit button carries aria-busy for accessibility', async () => {
    let resolveLogin: (() => void) | undefined;
    const slowInvoke = vi.fn(async (cmd: string) => {
      if (cmd === 'has_credential') return false;
      if (cmd === 'login_provider') {
        return new Promise<void>((r) => {
          resolveLogin = r;
        });
      }
      return undefined;
    });
    setInvokeForTesting(slowInvoke as never);

    const { findByTestId } = render(() => <CredentialsSection />);
    const input = (await findByTestId('credential-input-anthropic')) as HTMLInputElement;
    const submit = (await findByTestId('credential-submit-anthropic')) as HTMLButtonElement;

    fireEvent.input(input, { target: { value: 'sk-ant-1' } });
    fireEvent.click(submit);

    await waitFor(() => {
      expect(submit.getAttribute('aria-busy')).toBe('true');
    });

    resolveLogin?.();
  });
});

describe('CredentialBanner (F-588)', () => {
  it('renders the missing-credential prompt for the named provider', () => {
    const { getByTestId } = render(() => <CredentialBanner providerLabel="Anthropic" />);
    const banner = getByTestId('credential-banner');
    expect(banner.textContent).toContain('Anthropic');
    expect(banner.getAttribute('role')).toBe('status');
  });
});
