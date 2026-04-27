import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent, cleanup, waitFor } from '@solidjs/testing-library';
import type { SessionId } from '@forge/ipc';
import { CompactButton } from './CompactButton';

describe('CompactButton (F-598)', () => {
  it('renders with the canonical accessible name', () => {
    const { getByRole } = render(() => (
      <CompactButton sessionId={'sess-1' as SessionId} onCompact={async () => {}} />
    ));
    expect(getByRole('button').getAttribute('aria-label')).toBe(
      'Compact transcript',
    );
    cleanup();
  });

  it('dispatches onCompact with the session id when clicked', () => {
    const onCompact = vi.fn().mockResolvedValue(undefined);
    const { getByTestId } = render(() => (
      <CompactButton sessionId={'sess-7' as SessionId} onCompact={onCompact} />
    ));
    fireEvent.click(getByTestId('compact-button'));
    expect(onCompact).toHaveBeenCalledWith('sess-7');
    cleanup();
  });

  it('disables the button and sets aria-busy while a compaction call is in flight', () => {
    let resolve!: () => void;
    const blocking = new Promise<void>((r) => {
      resolve = r;
    });
    const onCompact = vi.fn().mockReturnValue(blocking);
    const { getByTestId } = render(() => (
      <CompactButton sessionId={'sess-1' as SessionId} onCompact={onCompact} />
    ));
    fireEvent.click(getByTestId('compact-button'));
    // Mid-flight: button is disabled, the label flips to the pending state,
    // and `aria-busy` flips to "true" so screen readers narrate the
    // operation-in-progress phase. `disabled` alone is not enough — it
    // takes the control out of the focus order without announcing why.
    const btn = getByTestId('compact-button') as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.getAttribute('aria-busy')).toBe('true');
    expect(btn.textContent).toContain('COMPACTING');
    resolve();
    cleanup();
  });

  it('clears aria-busy at idle (no in-flight call)', () => {
    const { getByTestId } = render(() => (
      <CompactButton sessionId={'sess-1' as SessionId} onCompact={async () => {}} />
    ));
    const btn = getByTestId('compact-button') as HTMLButtonElement;
    // Solid renders `aria-busy={false}` either as the literal attribute
    // "false" or omits it entirely; both satisfy the WAI-ARIA contract
    // that assistive tech treats anything other than "true" as not busy.
    const busy = btn.getAttribute('aria-busy');
    expect(busy === null || busy === 'false').toBe(true);
    cleanup();
  });

  it('surfaces a success toast after the IPC promise resolves', async () => {
    const onCompact = vi.fn().mockResolvedValue(undefined);
    const { getByTestId, queryByTestId } = render(() => (
      <CompactButton sessionId={'sess-1' as SessionId} onCompact={onCompact} />
    ));
    fireEvent.click(getByTestId('compact-button'));
    await waitFor(() => {
      const toast = queryByTestId('compact-button-toast');
      expect(toast).not.toBeNull();
      expect(toast!.getAttribute('data-toast-kind')).toBe('success');
    });
    cleanup();
  });

  it('surfaces an error toast when the IPC promise rejects', async () => {
    const onCompact = vi.fn().mockRejectedValue(new Error('daemon offline'));
    const { getByTestId, queryByTestId } = render(() => (
      <CompactButton sessionId={'sess-1' as SessionId} onCompact={onCompact} />
    ));
    fireEvent.click(getByTestId('compact-button'));
    await waitFor(() => {
      const toast = queryByTestId('compact-button-toast');
      expect(toast).not.toBeNull();
      expect(toast!.getAttribute('data-toast-kind')).toBe('error');
      expect(toast!.textContent).toContain('daemon offline');
    });
    cleanup();
  });
});
