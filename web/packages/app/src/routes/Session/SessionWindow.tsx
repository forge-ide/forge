import { type Component, onCleanup, onMount } from 'solid-js';
import { useParams } from '@solidjs/router';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { SessionId } from '@forge/ipc';
import {
  onSessionEvent,
  sessionHello,
  sessionSubscribe,
} from '../../ipc/session';
import {
  setActiveSessionId,
  setSessionEvents,
  setSessions,
} from '../../stores/session';
import { pushEvent, type SessionEvent } from '../../stores/messages';
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
        await sessionHello(id);
        await sessionSubscribe(id);
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
        pushEvent(payload.session_id, payload.event as SessionEvent);
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
  });

  const handleClose = () => {
    try {
      void getCurrentWindow().close();
    } catch (err) {
      console.error('window close failed', err);
    }
  };

  const subject = () => `Session ${sessionId()}`;
  const providerLabel = () => 'ollama \u00b7 pending';
  const costLabel = () => 'in 0 \u00b7 out 0 \u00b7 $0.00';

  return (
    <main class="session-window">
      <section class="session-window__pane" aria-label="Session pane">
        <PaneHeader
          subject={subject()}
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
