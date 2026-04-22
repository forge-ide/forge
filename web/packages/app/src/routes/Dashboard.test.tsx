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

  // Dashboard is wrapped in a router-capable context so any descendant
  // router primitives resolve cleanly, matching the shell's runtime.
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

  // F-409: spec dashboard.md §D.1 mandates a single flat surface — no tab
  // bar, no sidebar, no pane splits. A <nav> element violates the flatness
  // rule; AgentMonitor access is already provided by the StatusBar badge
  // and session-roster entry.
  it('renders no <nav> element (spec D.1 flat surface)', () => {
    const { container } = renderDashboard();
    expect(container.querySelector('nav')).toBeNull();
  });
});
