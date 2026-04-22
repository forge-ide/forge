import { describe, expect, it, vi, beforeEach, afterEach } from 'vitest';
import { cleanup, render, fireEvent } from '@solidjs/testing-library';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
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
  // Default any unstubbed invoke to a resolved promise so fire-and-fix call
  // sites (e.g., open_session in SessionsPanel) don't throw on `.catch(...)`
  // when a test only stubs the initial session_list call.
  invokeMock.mockResolvedValue(undefined);
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

  // F-401: `session_list` rejection must surface a visible error block
  // (noun + state heading + verbatim detail + RETRY), distinct from the
  // comment-syntax empty placeholder. Previous behavior swallowed the error
  // and returned `[]`, making "backend failed" visually identical to "zero
  // sessions". `dashboard.md D.5` is now the source of truth on this — see
  // the `SESSIONS UNAVAILABLE` block spec.
  it('renders the error block with verbatim detail when session_list rejects', async () => {
    invokeMock.mockRejectedValueOnce(new Error('disk exploded'));
    const { findByText, queryByText } = render(() => <SessionsPanel />);
    await waitForFetch();
    // Noun-state heading, matching PROVIDER UNAVAILABLE exemplar.
    expect(await findByText('SESSIONS UNAVAILABLE')).toBeTruthy();
    // Verbatim technical detail — String(new Error('x')) is "Error: x".
    expect(await findByText(/Error: disk exploded/)).toBeTruthy();
    // Error is distinct from empty — the comment-syntax placeholder must NOT
    // render when the fetch rejected.
    expect(queryByText('// no active sessions')).toBeNull();
  });

  it('retry button on the error state re-invokes session_list and recovers', async () => {
    invokeMock
      .mockRejectedValueOnce(new Error('transient'))
      .mockResolvedValueOnce([
        sample({ id: 'r1', subject: 'recovered', state: 'active' }),
      ]);
    const { findByRole, findByText } = render(() => <SessionsPanel />);
    await waitForFetch();
    const retry = await findByRole('button', { name: /^retry$/i });
    fireEvent.click(retry);
    await waitForFetch();
    expect(await findByText('recovered')).toBeTruthy();
  });

  // F-092: the stopped-pip pulse animation must be gated behind
  // `@media (prefers-reduced-motion: no-preference)` so users with
  // vestibular sensitivities (OS-level reduced-motion preference) get a
  // static, dimmed pip instead of an infinite pulse. JSDOM cannot evaluate
  // `@media (prefers-reduced-motion)`, so we assert the rule on disk.
  describe('reduced-motion gating for the stopped pip', () => {
    const css = readFileSync(resolve(__dirname, 'SessionsPanel.css'), 'utf-8');

    // Strip the `@media (prefers-reduced-motion: no-preference) { ... }`
    // block so we can inspect the *default* (no-preference, motion-on)
    // baseline without the gated overrides.
    function stripReducedMotionMediaBlock(source: string): string {
      const opener = /@media\s*\(\s*prefers-reduced-motion\s*:\s*no-preference\s*\)\s*\{/;
      const match = source.match(opener);
      if (!match || match.index === undefined) return source;
      const start = match.index;
      let i = start + match[0].length; // first byte inside the block
      let depth = 1;
      while (i < source.length && depth > 0) {
        const ch = source[i];
        if (ch === '{') depth += 1;
        else if (ch === '}') depth -= 1;
        i += 1;
      }
      return source.slice(0, start) + source.slice(i);
    }

    function ruleBody(source: string, selector: string): string | null {
      const escaped = selector.replace(/[.\\-]/g, (c) => `\\${c}`);
      const re = new RegExp(`(^|\\s)${escaped}\\s*\\{([^}]*)\\}`, 'm');
      const m = source.match(re);
      return m && m[2] !== undefined ? m[2] : null;
    }

    it('declares the pulse animation only inside a prefers-reduced-motion: no-preference media query', () => {
      // Default (reduced-motion or unspecified) baseline: no animation
      // should reach the stopped pip.
      const baseline = stripReducedMotionMediaBlock(css);
      expect(baseline).toContain('.session-card__pip--stopped');
      expect(baseline).not.toMatch(/\.session-card__pip--stopped\s*\{[^}]*animation\s*:/);

      // The animation lives behind the `no-preference` opt-in.
      expect(css).toMatch(
        /@media\s*\(\s*prefers-reduced-motion\s*:\s*no-preference\s*\)\s*\{[\s\S]*\.session-card__pip--stopped[\s\S]*animation\s*:\s*sessions-pip-pulse/,
      );
    });

    it('keeps the static stopped pip visually differentiated (dimmed opacity) for reduced-motion users', () => {
      // Strip the motion-gated overrides; the remaining baseline must still
      // give `.session-card__pip--stopped` a static differentiator (dimmed
      // opacity) so the "stopped" state stays readable without animation.
      const baseline = stripReducedMotionMediaBlock(css);
      const body = ruleBody(baseline, '.session-card__pip--stopped');
      expect(body, 'expected a default .session-card__pip--stopped rule').not.toBeNull();
      expect(body!).toMatch(/opacity\s*:\s*0?\.[0-9]+/);
    });
  });

  // F-079: open_session was previously a fire-and-forget `void invoke(...)`. A
  // rejection (IPC auth failure, validation error, etc.) must surface
  // user-visible feedback rather than silently dropping the click.
  it('surfaces an inline error when open_session rejects', async () => {
    invokeMock
      .mockResolvedValueOnce([sample({ id: 'xyz', subject: 'click me', state: 'active' })])
      .mockRejectedValueOnce(new Error('open denied'));

    const { findByLabelText, findByRole } = render(() => <SessionsPanel />);
    await waitForFetch();

    const card = await findByLabelText(/open session click me/i);
    fireEvent.click(card);

    // After microtasks drain, an inline error region must be visible to the user.
    await waitForFetch();
    const alert = await findByRole('alert');
    expect(alert.textContent ?? '').toMatch(/open denied/);
  });

  // F-416: tabs ↔ tabpanel association. Each role="tab" must reference its
  // panel via aria-controls; the matching role="tabpanel" must
  // reciprocate via aria-labelledby. This is the WAI-ARIA APG tabs pattern
  // and is the specific association axe-core flags when missing.
  describe('F-416 — tabs ↔ tabpanel association', () => {
    it('each tab carries an aria-controls pointing at an existing tabpanel', async () => {
      invokeMock.mockResolvedValueOnce([
        sample({ id: '1', state: 'active' }),
        sample({ id: '2', state: 'archived' }),
      ]);
      const { container, findByRole } = render(() => <SessionsPanel />);
      await waitForFetch();
      await findByRole('tab', { name: /active/i });

      const tabs = container.querySelectorAll<HTMLElement>('[role="tab"]');
      expect(tabs.length).toBeGreaterThanOrEqual(2);
      for (const tab of Array.from(tabs)) {
        const panelId = tab.getAttribute('aria-controls');
        expect(panelId, `tab "${tab.textContent}" missing aria-controls`).toBeTruthy();
        // The panel referenced by aria-controls only needs to exist when the
        // tab is selected; inactive tabs may reference a panel id that is
        // mounted only on selection. Selected tab's panel must be in-DOM now.
        if (tab.getAttribute('aria-selected') === 'true') {
          const panel = document.getElementById(panelId!);
          expect(panel, `panel ${panelId} not found for selected tab`).not.toBeNull();
          expect(panel!.getAttribute('role')).toBe('tabpanel');
          expect(panel!.getAttribute('aria-labelledby')).toBe(tab.id);
          expect(tab.id).toBeTruthy();
        }
      }
    });

    it('clicking a different tab swaps which tabpanel is labelled by which tab', async () => {
      invokeMock.mockResolvedValueOnce([
        sample({ id: '1', state: 'active' }),
        sample({ id: '2', state: 'archived' }),
      ]);
      const { container, findByRole } = render(() => <SessionsPanel />);
      await waitForFetch();

      const archivedTab = await findByRole('tab', { name: /archived/i });
      fireEvent.click(archivedTab);

      const panels = container.querySelectorAll<HTMLElement>('[role="tabpanel"]');
      expect(panels.length).toBeGreaterThanOrEqual(1);
      const panel = panels[0]!;
      expect(panel.getAttribute('aria-labelledby')).toBe(archivedTab.id);
      expect(archivedTab.getAttribute('aria-controls')).toBe(panel.id);
    });
  });

  // F-416: roving tabindex on the session grid. Tab enters the grid at
  // whichever card is the current tab stop; arrows, Home, and End move
  // focus within the grid without leaving it.
  describe('F-416 — session grid roving tabindex', () => {
    it('renders exactly one card with tabindex=0; the rest are tabindex=-1', async () => {
      invokeMock.mockResolvedValueOnce([
        sample({ id: '1', subject: 'alpha', state: 'active' }),
        sample({ id: '2', subject: 'beta', state: 'active' }),
        sample({ id: '3', subject: 'gamma', state: 'active' }),
      ]);
      const { container } = render(() => <SessionsPanel />);
      await waitForFetch();

      const cards = container.querySelectorAll<HTMLElement>('.session-card');
      expect(cards.length).toBe(3);
      const tabStops = Array.from(cards).filter(
        (c) => c.getAttribute('tabindex') === '0',
      );
      expect(tabStops.length).toBe(1);
      const inactive = Array.from(cards).filter(
        (c) => c.getAttribute('tabindex') === '-1',
      );
      expect(inactive.length).toBe(2);
    });

    it('ArrowRight moves focus to the next card', async () => {
      invokeMock.mockResolvedValueOnce([
        sample({ id: '1', subject: 'alpha', state: 'active' }),
        sample({ id: '2', subject: 'beta', state: 'active' }),
      ]);
      const { container } = render(() => <SessionsPanel />);
      await waitForFetch();

      const cards = container.querySelectorAll<HTMLElement>('.session-card');
      const first = cards[0]!;
      const second = cards[1]!;
      first.focus();
      first.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'ArrowRight',
          bubbles: true,
          cancelable: true,
        }),
      );
      expect(document.activeElement).toBe(second);
      expect(second.getAttribute('tabindex')).toBe('0');
      expect(first.getAttribute('tabindex')).toBe('-1');
    });

    it('End moves focus to the last card; Home moves it back to the first', async () => {
      invokeMock.mockResolvedValueOnce([
        sample({ id: '1', subject: 'alpha', state: 'active' }),
        sample({ id: '2', subject: 'beta', state: 'active' }),
        sample({ id: '3', subject: 'gamma', state: 'active' }),
      ]);
      const { container } = render(() => <SessionsPanel />);
      await waitForFetch();

      const cards = container.querySelectorAll<HTMLElement>('.session-card');
      const first = cards[0]!;
      const last = cards[2]!;
      first.focus();
      first.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'End',
          bubbles: true,
          cancelable: true,
        }),
      );
      expect(document.activeElement).toBe(last);
      last.dispatchEvent(
        new KeyboardEvent('keydown', {
          key: 'Home',
          bubbles: true,
          cancelable: true,
        }),
      );
      expect(document.activeElement).toBe(first);
    });
  });
});
