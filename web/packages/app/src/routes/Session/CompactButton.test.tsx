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

  it('disables the button while a compaction call is in flight', () => {
    let resolve!: () => void;
    const blocking = new Promise<void>((r) => {
      resolve = r;
    });
    const onCompact = vi.fn().mockReturnValue(blocking);
    const { getByTestId } = render(() => (
      <CompactButton sessionId={'sess-1' as SessionId} onCompact={onCompact} />
    ));
    fireEvent.click(getByTestId('compact-button'));
    // Mid-flight: button is disabled and the label flips to the pending state.
    const btn = getByTestId('compact-button') as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.textContent).toContain('COMPACTING');
    resolve();
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
