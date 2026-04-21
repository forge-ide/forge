// F-138: status-bar host that surfaces background-agent state for the active
// session. Follows `docs/ui-specs/shell.md` §2 for placement (bottom strip of
// the Session window) and `docs/product/ai-ux.md` §4.5 + §10.6 for the
// `notifications.bg_agents` semantics (`toast | os | both | silent`).
//
// Wiring:
//  - On mount: `listBackgroundAgents(sessionId)` seeds the running set.
//  - On mount: `subscribe(handler)` hooks into the same `session:event`
//    channel F-137 emits on. Started → insert; Completed → remove + fire the
//    configured notification.
//  - Badge hidden when count is zero. Click the badge → popover with one row
//    per running agent and Promote/Stop buttons.
//
// Testability seams. Every external dependency — the IPC list/promote/stop
// calls, the Tauri `session:event` listener, the OS-notification plugin, and
// the in-app toast queue — is injectable as a prop. This keeps the component
// testable in jsdom without pulling `@tauri-apps/plugin-notification` into
// the render. Production call sites use the defaults.

import {
  type Component,
  For,
  Show,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js';
import type { BgAgentSummary, SessionId } from '@forge/ipc';
import {
  isPermissionGranted as pluginIsPermissionGranted,
  requestPermission as pluginRequestPermission,
  sendNotification as pluginSendNotification,
} from '@tauri-apps/plugin-notification';
import {
  listBackgroundAgents as defaultListBackgroundAgents,
  onSessionEvent,
  promoteBackgroundAgent as defaultPromoteBackgroundAgent,
  stopBackgroundAgent as defaultStopBackgroundAgent,
  type SessionEventPayload,
} from '../ipc/session';
import { settings } from '../stores/settings';
import { pushToast } from '../components/toast';
import './StatusBar.css';

// ---------------------------------------------------------------------------
// Event bus + notification adapter surfaces (injectable).
// ---------------------------------------------------------------------------

/** Subscribe to `session:event` payloads. Returns an unlisten handle. */
export type BgAgentsSubscribe = (
  handler: (payload: SessionEventPayload) => void,
) => Promise<() => void>;

/**
 * Notification side-effect surface. `toast` pushes into the in-app queue;
 * `os` dispatches an OS-level notification; the permission helpers power the
 * lazy request path so we don't prompt at mount.
 */
export interface NotificationAdapter {
  toast: (message: string) => void;
  os: (message: string) => void | Promise<void>;
  isPermissionGranted: () => Promise<boolean>;
  requestPermission: () => Promise<'granted' | 'denied' | 'default'>;
}

export const defaultNotificationAdapter: NotificationAdapter = {
  toast: (message) => {
    pushToast('info', message);
  },
  os: (message) => {
    pluginSendNotification({ title: 'Forge', body: message });
  },
  isPermissionGranted: () => pluginIsPermissionGranted(),
  requestPermission: async () => {
    const res = await pluginRequestPermission();
    if (res === 'granted' || res === 'denied' || res === 'default') return res;
    return 'default';
  },
};

/** Production default: forward every `onSessionEvent` payload verbatim. */
const defaultSubscribe: BgAgentsSubscribe = (handler) =>
  onSessionEvent(handler);

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export interface StatusBarProps {
  sessionId: SessionId;
  /** Initial snapshot provider. Defaults to the real Tauri command. */
  listBackgroundAgents?: typeof defaultListBackgroundAgents;
  /** Promote IPC wrapper. */
  promoteBackgroundAgent?: typeof defaultPromoteBackgroundAgent;
  /** Stop IPC wrapper. */
  stopBackgroundAgent?: typeof defaultStopBackgroundAgent;
  /** Event-bus subscription. Test harnesses inject a controllable fake. */
  subscribe?: BgAgentsSubscribe;
  /** Notification delivery adapter. */
  notificationAdapter?: NotificationAdapter;
}

interface BgEventStarted {
  type: 'background_agent_started';
  id: string;
  agent?: string;
}

interface BgEventCompleted {
  type: 'background_agent_completed';
  id: string;
}

/** Narrowing helper — guards against malformed payloads on the event bus. */
function classifyEvent(ev: unknown): BgEventStarted | BgEventCompleted | null {
  if (typeof ev !== 'object' || ev === null) return null;
  const obj = ev as Record<string, unknown>;
  const type = obj['type'];
  const id = obj['id'];
  if (typeof id !== 'string') return null;
  if (type === 'background_agent_started') {
    const agent =
      typeof obj['agent'] === 'string' ? (obj['agent'] as string) : undefined;
    return agent === undefined
      ? { type, id }
      : { type, id, agent };
  }
  if (type === 'background_agent_completed') {
    return { type, id };
  }
  return null;
}

export const StatusBar: Component<StatusBarProps> = (props) => {
  const listBg = (): typeof defaultListBackgroundAgents =>
    props.listBackgroundAgents ?? defaultListBackgroundAgents;
  const promoteBg = (): typeof defaultPromoteBackgroundAgent =>
    props.promoteBackgroundAgent ?? defaultPromoteBackgroundAgent;
  const stopBg = (): typeof defaultStopBackgroundAgent =>
    props.stopBackgroundAgent ?? defaultStopBackgroundAgent;
  const subscribe = (): BgAgentsSubscribe =>
    props.subscribe ?? defaultSubscribe;
  const notifier = (): NotificationAdapter =>
    props.notificationAdapter ?? defaultNotificationAdapter;

  // Running set keyed by instance id. The same id from multiple sources (the
  // initial list + the subscribe stream) is idempotently deduped.
  const [running, setRunning] = createSignal<BgAgentSummary[]>([]);
  const [popoverOpen, setPopoverOpen] = createSignal(false);

  let unlisten: (() => void) | null = null;
  let mounted = true;

  const badgeCount = () =>
    running().filter((r) => r.state === 'Running').length;

  const notifyOnCompletion = async (instanceId: string): Promise<void> => {
    const mode = settings.notifications.bg_agents;
    const message = `Background agent ${instanceId.slice(0, 8)} completed`;
    if (mode === 'silent') return;
    const n = notifier();
    if (mode === 'toast' || mode === 'both') {
      try {
        n.toast(message);
      } catch (err) {
        console.error('bg_agents toast failed', err);
      }
    }
    if (mode === 'os' || mode === 'both') {
      try {
        let granted = await n.isPermissionGranted();
        if (!granted) {
          const res = await n.requestPermission();
          granted = res === 'granted';
        }
        if (granted) {
          await n.os(message);
        }
      } catch (err) {
        console.error('bg_agents OS notification failed', err);
      }
    }
  };

  const handlePayload = (payload: SessionEventPayload): void => {
    if (payload.session_id !== props.sessionId) return;
    const ev = classifyEvent(payload.event);
    if (ev === null) return;
    if (ev.type === 'background_agent_started') {
      setRunning((prev) => {
        if (prev.some((r) => r.id === ev.id)) return prev;
        return [
          ...prev,
          {
            id: ev.id,
            agent_name: ev.agent ?? 'agent',
            state: 'Running',
          },
        ];
      });
      return;
    }
    // background_agent_completed
    setRunning((prev) => prev.filter((r) => r.id !== ev.id));
    void notifyOnCompletion(ev.id);
  };

  onMount(() => {
    // Install the subscription BEFORE the initial list so a fast completion
    // event between the two awaits isn't lost to a listener-registration race
    // — mirrors the same discipline `BackgroundAgentRegistry::start` uses for
    // its orchestrator-stream subscription.
    void (async () => {
      try {
        const off = await subscribe()(handlePayload);
        if (mounted) {
          unlisten = off;
        } else {
          off();
        }
      } catch (err) {
        console.error('bg-agents subscribe failed', err);
      }
    })();
    void (async () => {
      try {
        const snapshot = await listBg()(props.sessionId);
        if (mounted && Array.isArray(snapshot)) {
          const runningRows = snapshot.filter((r) => r.state === 'Running');
          setRunning(runningRows);
        }
      } catch (err) {
        console.error('list_background_agents failed', err);
      }
    })();
  });

  onCleanup(() => {
    mounted = false;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  });

  const togglePopover = (): void => {
    setPopoverOpen((v) => !v);
  };

  const onPromote = async (id: string): Promise<void> => {
    try {
      await promoteBg()(props.sessionId, id);
      setRunning((prev) => prev.filter((r) => r.id !== id));
    } catch (err) {
      console.error('promote_background_agent failed', err);
    }
  };

  const onStop = async (id: string): Promise<void> => {
    try {
      await stopBg()(props.sessionId, id);
      // Don't pre-remove: the completion event will clear the row and fire
      // the notification on the same path as any other terminal transition.
    } catch (err) {
      console.error('stop_background_agent failed', err);
    }
  };

  return (
    <footer class="status-bar" aria-label="Status bar" data-testid="status-bar">
      <Show when={badgeCount() > 0}>
        <button
          type="button"
          class="status-bar__bg-badge"
          data-testid="bg-agents-badge"
          aria-label={`${badgeCount()} background agents`}
          aria-haspopup="menu"
          aria-expanded={popoverOpen()}
          onClick={togglePopover}
        >
          <span class="status-bar__bg-badge-count">{badgeCount()}</span>
          <span class="status-bar__bg-badge-label">{' bg'}</span>
        </button>
      </Show>
      <Show when={popoverOpen() && badgeCount() > 0}>
        <div
          class="status-bar__bg-popover"
          data-testid="bg-agents-popover"
          role="menu"
        >
          <For each={running().filter((r) => r.state === 'Running')}>
            {(row) => (
              <div class="status-bar__bg-row" role="menuitem">
                <span
                  class="status-bar__bg-row-name"
                  data-testid={`bg-agents-name-${row.id}`}
                >
                  {row.agent_name}
                </span>
                <span class="status-bar__bg-row-id">{row.id.slice(0, 8)}</span>
                <button
                  type="button"
                  class="status-bar__bg-row-action"
                  data-testid={`bg-agents-promote-${row.id}`}
                  onClick={() => {
                    void onPromote(row.id);
                  }}
                >
                  Promote
                </button>
                <button
                  type="button"
                  class="status-bar__bg-row-action status-bar__bg-row-action--stop"
                  data-testid={`bg-agents-stop-${row.id}`}
                  onClick={() => {
                    void onStop(row.id);
                  }}
                >
                  Stop
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>
    </footer>
  );
};
