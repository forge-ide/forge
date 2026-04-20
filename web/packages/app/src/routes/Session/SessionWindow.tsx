import { type Component, onCleanup, onMount } from 'solid-js';
import { useParams } from '@solidjs/router';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { ProviderId, SessionId } from '@forge/ipc';
import {
  getPersistentApprovals,
  onSessionEvent,
  sessionHello,
  sessionSubscribe,
} from '../../ipc/session';
import {
  setActiveSessionId,
  setActiveWorkspaceRoot,
  setSessionEvents,
  setSessions,
} from '../../stores/session';
import { pushEvent } from '../../stores/messages';
import { fromRustEvent } from '../../ipc/events';
import { seedPersistentApprovals } from '../../stores/approvals';
import { PaneHeader } from './PaneHeader';
import { ChatPane } from './ChatPane';
import './SessionWindow.css';

/**
 * Session window shell — default single-pane layout from
 * docs/ui-specs/layout-panes.md §3.4. Splitters and drag-to-dock are
 * deferred to Phase 2. Subscribes to the session event stream on mount
 * (via the F-020 IPC wrappers) and cleanly detaches on unmount.
 */
export const SessionWindow: Component = () => {
  const params = useParams<{ id: string }>();
  const sessionId = () => params.id as SessionId;

  let unlisten: (() => void) | null = null;
  let mounted = true;

  onMount(() => {
    const id = sessionId();
    setActiveSessionId(id);
    setSessions(id, { id, state: 'Active' });

    void (async () => {
      try {
        const ack = await sessionHello(id);
        // F-036: remember the workspace root from the HelloAck so the
        // ApprovalPrompt / WhitelistedPill code paths can pass it to the
        // persistent-approval commands without re-querying.
        setActiveWorkspaceRoot(ack.workspace);
        await sessionSubscribe(id);
        // F-036: seed the per-session whitelist with any persisted
        // workspace/user approvals for the active workspace. Failure here is
        // non-fatal — the session still runs, users just won't see auto-
        // approvals until they reload. Log and continue.
        try {
          const persisted = await getPersistentApprovals(ack.workspace);
          seedPersistentApprovals(id, persisted);
        } catch (seedErr) {
          console.error('get_persistent_approvals failed', seedErr);
        }
      } catch (err) {
        console.error('session_hello/subscribe failed', err);
      }
      const listener = await onSessionEvent((payload) => {
        if (payload.session_id !== id) return;
        setSessionEvents(payload.session_id, {
          lastSeq: payload.seq,
          lastEvent: payload.event,
        });
        // Route typed events to the messages store for ChatPane rendering.
        // `payload.event` is the Rust-serialized `forge_core::Event` shape
        // (`{"type":"user_message",…}`); fromRustEvent adapts it to the
        // store's discriminated union. Non-renderable variants return null.
        const storeEvent = fromRustEvent(payload.event);
        if (storeEvent) pushEvent(payload.session_id, storeEvent);
      });
      if (mounted) {
        unlisten = listener;
      } else {
        // Component unmounted before the async setup completed — detach immediately.
        listener();
      }
    })();
  });

  onCleanup(() => {
    mounted = false;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    setActiveSessionId(null);
    setActiveWorkspaceRoot(null);
  });

  const handleClose = () => {
    try {
      void getCurrentWindow().close();
    } catch (err) {
      console.error('window close failed', err);
    }
  };

  const subject = () => `Session ${sessionId()}`;
  // Phase 1 ships only the Ollama provider; once the active session carries
  // its provider id over IPC (Phase 2), wire it through here so the pill
  // accent follows the live provider per ai-patterns.md §7 (F-091).
  const providerId = (): ProviderId => 'ollama' as ProviderId;
  const providerLabel = () => 'ollama \u00b7 pending';
  const costLabel = () => 'in 0 \u00b7 out 0 \u00b7 $0.00';

  return (
    <main class="session-window">
      <section class="session-window__pane" aria-label="Session pane">
        <PaneHeader
          subject={subject()}
          providerId={providerId()}
          providerLabel={providerLabel()}
          costLabel={costLabel()}
          onClose={handleClose}
        />
        <div class="session-window__pane-body">
          <ChatPane />
        </div>
      </section>
    </main>
  );
};
