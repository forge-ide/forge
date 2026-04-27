import { type Component, createSignal, Show } from 'solid-js';
import { Button } from '@forge/design';
import type { SessionId } from '@forge/ipc';
import { compactTranscript } from '../../ipc/session';
import './CompactButton.css';

/**
 * F-598 transcript-toolbar trigger for manual context compaction.
 *
 * Click handler dispatches `compactTranscript(sessionId)` over IPC; while
 * the call is in flight the button surfaces a "compacting…" label and is
 * disabled so a user double-click cannot fan out two privileged summary
 * calls. On success we surface a transient toast — the actual marker
 * arrives through the `ContextCompacted` event stream and is rendered by
 * the message store, so the toast is purely a "you asked, it ran"
 * acknowledgment.
 *
 * Failure paths (network drop, daemon error) drop into the toast as well
 * so the user is not left wondering whether the click registered. The
 * component is purposely stateless beyond `pending` / `toast` — it does
 * not track the in-flight summary's progress, because the daemon never
 * publishes intermediate compaction events. Either the marker lands or
 * the IPC call surfaces an error.
 */
export interface CompactButtonProps {
  sessionId: SessionId;
  /**
   * Optional dispatcher seam for tests — defaults to the real IPC call.
   * Tests pass a stub that resolves / rejects deterministically.
   */
  onCompact?: (sessionId: SessionId) => Promise<void>;
}

const TOAST_TIMEOUT_MS = 3_000;

export const CompactButton: Component<CompactButtonProps> = (props) => {
  const [pending, setPending] = createSignal(false);
  const [toast, setToast] = createSignal<{
    kind: 'success' | 'error';
    text: string;
  } | null>(null);

  const dispatch = async (): Promise<void> => {
    if (pending()) return;
    setPending(true);
    setToast(null);
    try {
      const fn = props.onCompact ?? compactTranscript;
      await fn(props.sessionId);
      setToast({ kind: 'success', text: 'Compaction started' });
    } catch (err) {
      const detail = err instanceof Error ? err.message : String(err);
      setToast({ kind: 'error', text: `Compact failed: ${detail}` });
    } finally {
      setPending(false);
      // The toast self-dismisses after a short window so the toolbar
      // returns to its idle shape without forcing the user to interact.
      window.setTimeout(() => setToast(null), TOAST_TIMEOUT_MS);
    }
  };

  return (
    <div class="compact-button" data-testid="compact-button-root">
      <Button
        variant="ghost"
        size="sm"
        class="compact-button__btn"
        aria-label="Compact transcript"
        // F-598: announce the in-flight state to assistive tech. `disabled`
        // alone removes the control from the focus order but does not
        // signal "operation in progress" — `aria-busy` is what screen
        // readers narrate for the work-pending phase.
        aria-busy={pending()}
        disabled={pending()}
        onClick={() => {
          void dispatch();
        }}
        data-testid="compact-button"
      >
        {pending() ? 'COMPACTING…' : 'COMPACT'}
      </Button>
      <Show when={toast()}>
        {(t) => (
          <span
            class="compact-button__toast"
            data-toast-kind={t().kind}
            data-testid="compact-button-toast"
            role="status"
          >
            {t().text}
          </span>
        )}
      </Show>
    </div>
  );
};
