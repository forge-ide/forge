import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import { MemoryRouter, Route } from '@solidjs/router';
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

  // F-140: Dashboard now renders a router-aware `<A>` link to the Agent
  // Monitor, so it must mount under a `<MemoryRouter>` for the link to
  // resolve its route context without erroring.
  function renderDashboard() {
    return render(() => (
      <MemoryRouter>
        <Route path="/" component={Dashboard} />
      </MemoryRouter>
    ));
  }

  it('renders the placeholder heading', () => {
    const { getByRole } = renderDashboard();
    const heading = getByRole('heading', { level: 1 });
    expect(heading.textContent).toBe('Forge — Dashboard');
  });

  it('exposes an Agent Monitor link in the app navigation', () => {
    const { getByRole } = renderDashboard();
    const link = getByRole('link', { name: /agent monitor/i });
    expect(link.getAttribute('href')).toBe('/agents');
  });
});
