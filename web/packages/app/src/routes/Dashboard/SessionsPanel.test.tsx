import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { cleanup, render, fireEvent } from '@solidjs/testing-library';
import { SessionsPanel, type SessionSummary } from './SessionsPanel';
import { setInvokeForTesting } from '../../lib/tauri';

const invokeMock = vi.fn();

const sample = (over: Partial<SessionSummary> = {}): SessionSummary => ({
  id: 'abc',
  subject: 'refactor payments',
  state: 'active',
  persistence: 'persist',
  createdAt: '2026-04-15T10:00:00Z',
  lastEventAt: '2026-04-15T11:00:00Z',
  provider: 'ollama',
  ...over,
});

async function waitForFetch() {
  // Let the microtask queue drain so the resource reads the mocked invoke.
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  invokeMock.mockReset();
  setInvokeForTesting(invokeMock as never);
});

afterEach(() => {
  setInvokeForTesting(null);
  cleanup();
});

describe('SessionsPanel', () => {
  it('renders Active and Archived tabs with counts from session_list', async () => {
    invokeMock.mockResolvedValueOnce([
      sample({ id: '1', state: 'active' }),
      sample({ id: '2', state: 'stopped' }),
      sample({ id: '3', state: 'archived' }),
    ]);

    const { findByRole } = render(() => <SessionsPanel />);
    await waitForFetch();

    const active = await findByRole('tab', { name: /active/i });
    const archived = await findByRole('tab', { name: /archived/i });
    expect(active.textContent).toMatch(/active/i);
    expect(active.textContent).toMatch(/02/);
    expect(archived.textContent).toMatch(/archived/i);
    expect(archived.textContent).toMatch(/01/);
  });

  it('shows only active cards on Active and only archived on Archived', async () => {
    invokeMock.mockResolvedValueOnce([
      sample({ id: '1', subject: 'alpha', state: 'active' }),
      sample({ id: '2', subject: 'beta-stale', state: 'stopped' }),
      sample({ id: '3', subject: 'gamma-old', state: 'archived' }),
    ]);
    const { findByRole, findByText, queryByText } = render(() => <SessionsPanel />);
    await waitForFetch();

    expect(await findByText('alpha')).toBeTruthy();
    expect(await findByText('beta-stale')).toBeTruthy();
    expect(queryByText('gamma-old')).toBeNull();

    fireEvent.click(await findByRole('tab', { name: /archived/i }));
    expect(await findByText('gamma-old')).toBeTruthy();
    expect(queryByText('alpha')).toBeNull();
    expect(queryByText('beta-stale')).toBeNull();
  });

  it('clicking a card invokes open_session with the session id', async () => {
    invokeMock.mockResolvedValueOnce([
      sample({ id: 'xyz', subject: 'click me', state: 'active' }),
    ]);
    const { findByLabelText } = render(() => <SessionsPanel />);
    await waitForFetch();

    const card = await findByLabelText(/open session click me/i);
    fireEvent.click(card);

    expect(invokeMock).toHaveBeenCalledWith('open_session', { id: 'xyz' });
  });

  it('renders the comment-syntax empty state on each tab', async () => {
    invokeMock.mockResolvedValueOnce([]);
    const { findByText, findByRole } = render(() => <SessionsPanel />);
    await waitForFetch();

    expect(await findByText('// no active sessions')).toBeTruthy();
    fireEvent.click(await findByRole('tab', { name: /archived/i }));
    expect(await findByText('// archive is empty')).toBeTruthy();
  });

  it('marks stopped sessions with the stopped pip class', async () => {
    invokeMock.mockResolvedValueOnce([
      sample({ id: 's', subject: 'stalled', state: 'stopped' }),
    ]);
    const { findByLabelText } = render(() => <SessionsPanel />);
    await waitForFetch();

    const card = await findByLabelText(/open session stalled/i);
    const pip = card.querySelector('.session-card__pip');
    expect(pip?.classList.contains('session-card__pip--stopped')).toBe(true);
  });

  it('treats session_list failure as an empty list', async () => {
    invokeMock.mockRejectedValueOnce(new Error('disk exploded'));
    const { findByText } = render(() => <SessionsPanel />);
    await waitForFetch();
    expect(await findByText('// no active sessions')).toBeTruthy();
  });
});
