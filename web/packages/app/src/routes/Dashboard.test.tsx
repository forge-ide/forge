import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { MemoryRouter, Route } from '@solidjs/router';
import { Dashboard } from './Dashboard';
import { setInvokeForTesting } from '../lib/tauri';

describe('Dashboard', () => {
  beforeEach(() => {
    // Dashboard mounts ProviderPanel + SessionsPanel + CredentialsSection,
    // each of which invokes Tauri commands on mount. Route every command
    // to a hermetic stub so tests don't attempt a real bridge call.
    setInvokeForTesting(
      (async (cmd: string) => {
        if (cmd === 'provider_status') {
          return {
            reachable: true,
            base_url: 'http://127.0.0.1:11434',
            models: [],
            last_checked: '2026-04-18T00:00:00Z',
          };
        }
        if (cmd === 'session_list') return [];
        if (cmd === 'has_credential') return true;
        return undefined;
      }) as never,
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

  // F-588: when every credential-bearing provider has a key stored, the
  // first-run banner stays hidden.
  it('does not render the credential banner when all providers have keys', async () => {
    const { queryByTestId } = renderDashboard();
    // Resource resolves on next microtask; flush.
    await waitFor(() => {
      expect(queryByTestId('credential-banner')).toBeNull();
    });
  });

  // F-588: when at least one credential-bearing provider has no key, the
  // banner names the first such provider.
  it('renders the credential banner when a provider has no stored key', async () => {
    setInvokeForTesting(
      (async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === 'provider_status') {
          return {
            reachable: true,
            base_url: 'http://127.0.0.1:11434',
            models: [],
            last_checked: '2026-04-18T00:00:00Z',
          };
        }
        if (cmd === 'session_list') return [];
        if (cmd === 'has_credential') {
          // Anthropic missing, OpenAI present — banner should name Anthropic.
          return args?.['providerId'] === 'openai';
        }
        return undefined;
      }) as never,
    );

    const { findByTestId } = renderDashboard();
    const banner = await findByTestId('credential-banner');
    expect(banner.textContent).toContain('Anthropic');
  });
});
