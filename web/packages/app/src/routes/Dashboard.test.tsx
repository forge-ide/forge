import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import { Dashboard } from './Dashboard';
import { setInvokeForTesting } from '../lib/tauri';

describe('Dashboard', () => {
  beforeEach(() => {
    // Dashboard mounts ProviderPanel which invokes `provider_status` on mount.
    // Stub so tests don't attempt a real Tauri bridge call.
    setInvokeForTesting(
      (async () => ({
        reachable: true,
        base_url: 'http://127.0.0.1:11434',
        models: [],
        last_checked: '2026-04-18T00:00:00Z',
      })) as never,
    );
  });

  afterEach(() => {
    setInvokeForTesting(null);
    cleanup();
  });

  it('renders the placeholder heading', () => {
    const { getByRole } = render(() => <Dashboard />);
    const heading = getByRole('heading', { level: 1 });
    expect(heading.textContent).toBe('Forge — Dashboard');
  });
});
