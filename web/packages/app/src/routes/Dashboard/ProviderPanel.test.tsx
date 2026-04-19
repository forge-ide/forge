import { afterEach, describe, expect, it, vi } from 'vitest';
import { render, waitFor, fireEvent, cleanup } from '@solidjs/testing-library';
import { ProviderPanel } from './ProviderPanel';
import { setInvokeForTesting } from '../../lib/tauri';

interface ProviderStatus {
  reachable: boolean;
  base_url: string;
  models: string[];
  last_checked: string;
  error_kind?: string;
}

const reachable: ProviderStatus = {
  reachable: true,
  base_url: 'http://127.0.0.1:11434',
  models: ['llama3', 'mistral'],
  last_checked: '2026-04-18T00:00:00Z',
};

const unreachable: ProviderStatus = {
  reachable: false,
  base_url: 'http://127.0.0.1:11434',
  models: [],
  last_checked: '2026-04-18T00:00:00Z',
  error_kind: 'connection refused',
};

afterEach(() => {
  setInvokeForTesting(null);
  cleanup();
});

describe('ProviderPanel', () => {
  it('calls provider_status on mount and renders base_url + model count', async () => {
    const invoke = vi.fn().mockResolvedValue(reachable);
    setInvokeForTesting(invoke as never);

    const { findByText } = render(() => <ProviderPanel />);

    expect(await findByText('http://127.0.0.1:11434')).toBeInTheDocument();
    expect(await findByText(/2\s+models/i)).toBeInTheDocument();
    expect(invoke).toHaveBeenCalledWith('provider_status', undefined);
  });

  it('reveals the model list when the user expands the card', async () => {
    setInvokeForTesting((vi.fn().mockResolvedValue(reachable)) as never);

    const { findByRole, getByText } = render(() => <ProviderPanel />);

    const toggle = await findByRole('button', { name: /show models/i });
    fireEvent.click(toggle);

    expect(getByText('llama3')).toBeInTheDocument();
    expect(getByText('mistral')).toBeInTheDocument();
  });

  it('renders a health indicator labelled reachable when online', async () => {
    setInvokeForTesting((vi.fn().mockResolvedValue(reachable)) as never);

    const { findByLabelText } = render(() => <ProviderPanel />);
    expect(await findByLabelText(/reachable/i)).toBeInTheDocument();
  });

  it('shows a voice-compliant Start Ollama message when unreachable', async () => {
    setInvokeForTesting((vi.fn().mockResolvedValue(unreachable)) as never);

    const { findByRole, findByText } = render(() => <ProviderPanel />);

    const cta = await findByRole('button', { name: /start ollama/i });
    expect(cta).toBeInTheDocument();
    // Voice rule: technical identifiers must be shown verbatim.
    expect(await findByText(/ECONNREFUSED 127\.0\.0\.1:11434/)).toBeInTheDocument();
  });

  it('refresh button re-invokes provider_status', async () => {
    const invoke = vi
      .fn()
      .mockResolvedValueOnce(reachable)
      .mockResolvedValueOnce({ ...reachable, models: ['llama3', 'mistral', 'codellama'] });
    setInvokeForTesting(invoke as never);

    const { findByRole, findByText } = render(() => <ProviderPanel />);

    const refresh = await findByRole('button', { name: /refresh/i });
    fireEvent.click(refresh);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledTimes(2);
    });
    expect(await findByText(/3\s+models/i)).toBeInTheDocument();
  });

  it('renders the error state when provider_status rejects, showing the detail verbatim', async () => {
    const invoke = vi.fn().mockRejectedValue(new Error('authz denied: dashboard'));
    setInvokeForTesting(invoke as never);

    const { findByText } = render(() => <ProviderPanel />);

    // Noun-state heading per voice-terminology.md
    expect(await findByText('PROVIDER UNAVAILABLE')).toBeInTheDocument();
    // Verbatim technical detail — String(new Error('x')) is "Error: x"
    expect(await findByText(/Error: authz denied: dashboard/)).toBeInTheDocument();
  });

  it('retry button on the error state re-invokes provider_status and recovers', async () => {
    const invoke = vi
      .fn()
      .mockRejectedValueOnce(new Error('authz denied: dashboard'))
      .mockResolvedValueOnce(reachable);
    setInvokeForTesting(invoke as never);

    const { findByRole, findByText } = render(() => <ProviderPanel />);

    const retry = await findByRole('button', { name: /^retry$/i });
    fireEvent.click(retry);

    await waitFor(() => {
      expect(invoke).toHaveBeenCalledTimes(2);
    });
    // After recovery, the success render replaces the error state.
    expect(await findByText('http://127.0.0.1:11434')).toBeInTheDocument();
  });
});
