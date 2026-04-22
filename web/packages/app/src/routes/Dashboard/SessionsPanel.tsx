import { createResource, createSignal, For, Show, type Component } from 'solid-js';
import { invoke } from '../../lib/tauri';
import { useRovingTabindex } from '../../lib/useRovingTabindex';
import './SessionsPanel.css';

export type SessionWireState = 'active' | 'archived' | 'stopped';

export interface SessionSummary {
  id: string;
  subject: string;
  state: SessionWireState;
  persistence: 'persist' | 'ephemeral';
  createdAt: string;
  lastEventAt: string;
  /** Optional; provider chip is shown when present. */
  provider?: string;
}

type Tab = 'active' | 'archived';

async function fetchSessions(): Promise<SessionSummary[]> {
  try {
    const result = await invoke<SessionSummary[]>('session_list');
    return Array.isArray(result) ? result : [];
  } catch {
    return [];
  }
}

function partition(sessions: SessionSummary[]): Record<Tab, SessionSummary[]> {
  return {
    active: sessions.filter((s) => s.state !== 'archived'),
    archived: sessions.filter((s) => s.state === 'archived'),
  };
}

function count(n: number): string {
  return n.toString().padStart(2, '0');
}

/**
 * Sessions panel for the Dashboard. Industrial-ledger specimen cards with
 * Active / Archived tabs. Data comes from the `session_list` Tauri command;
 * card clicks dispatch `open_session` to reopen the Session window.
 */
export const SessionsPanel: Component = () => {
  const [sessions] = createResource(fetchSessions, { initialValue: [] });
  const [tab, setTab] = createSignal<Tab>('active');
  // F-079: inline error surface when `open_session` rejects (IPC auth fail,
  // missing window, validation error, etc.). Previously a fire-and-forget
  // `void invoke(...)` swallowed all rejections silently.
  const [openError, setOpenError] = createSignal<string | null>(null);
  const groups = () => partition(sessions() ?? []);
  const current = () => groups()[tab()];

  const handleOpen = (id: string) => {
    setOpenError(null);
    invoke('open_session', { id }).catch((err) => {
      const detail = err instanceof Error ? err.message : String(err);
      setOpenError(`open_session failed: ${detail}`);
    });
  };

  // F-416: roving tabindex on the session grid. The hook keeps exactly one
  // card as the tab stop and handles ArrowRight/Left/Up/Down/Home/End so
  // the grid is a single Tab stop with internal arrow navigation per
  // WAI-ARIA APG grid pattern. The ref is a signal so the hook's effect
  // re-attaches whenever <Show> toggles between grid and fallback.
  const [gridRef, setGridRef] = createSignal<HTMLDivElement | undefined>();
  useRovingTabindex(gridRef, '.session-card');

  const panelId = () => `sessions-panel-${tab()}`;
  const tabId = (t: Tab) => `sessions-tab-${t}`;

  return (
    <section class="sessions" aria-label="Sessions">
      <div role="tablist" class="sessions__tabs">
        <TabButton tab="active" current={tab()} onSelect={setTab} count={groups().active.length} />
        <TabButton tab="archived" current={tab()} onSelect={setTab} count={groups().archived.length} />
      </div>
      <Show when={openError()}>
        {(msg) => (
          <p class="sessions__error" role="alert">
            {msg()}
          </p>
        )}
      </Show>
      <Show
        when={current().length > 0}
        fallback={
          <p
            class="sessions__empty"
            id={panelId()}
            role="tabpanel"
            aria-labelledby={tabId(tab())}
          >
            {tab() === 'active' ? '// no active sessions' : '// archive is empty'}
          </p>
        }
      >
        <div
          ref={setGridRef}
          class="sessions__grid"
          id={panelId()}
          role="tabpanel"
          aria-labelledby={tabId(tab())}
        >
          <For each={current()}>
            {(session) => <SessionCard session={session} onOpen={handleOpen} />}
          </For>
        </div>
      </Show>
    </section>
  );
};

interface TabButtonProps {
  tab: Tab;
  current: Tab;
  count: number;
  onSelect: (t: Tab) => void;
}

const TabButton: Component<TabButtonProps> = (props) => {
  const selected = () => props.tab === props.current;
  return (
    <button
      type="button"
      role="tab"
      id={`sessions-tab-${props.tab}`}
      aria-controls={`sessions-panel-${props.tab}`}
      class="sessions__tab"
      classList={{ 'sessions__tab--active': selected() }}
      aria-selected={selected()}
      onClick={() => props.onSelect(props.tab)}
    >
      <span class="sessions__tab-label">{props.tab}</span>
      <span class="sessions__tab-count">{count(props.count)}</span>
    </button>
  );
};

interface SessionCardProps {
  session: SessionSummary;
  onOpen: (id: string) => void;
}

const SessionCard: Component<SessionCardProps> = (props) => {
  const stateClass = () => `session-card__pip session-card__pip--${props.session.state}`;
  return (
    <button
      type="button"
      class="session-card"
      onClick={() => props.onOpen(props.session.id)}
      aria-label={`Open session ${props.session.subject}`}
    >
      <header class="session-card__header">
        <h3 class="session-card__subject">{props.session.subject}</h3>
        <span
          class="session-card__badge"
          classList={{
            'session-card__badge--persist': props.session.persistence === 'persist',
            'session-card__badge--ephemeral': props.session.persistence === 'ephemeral',
          }}
        >
          {props.session.persistence}
        </span>
      </header>
      <div class="session-card__state">
        <span class={stateClass()} aria-hidden="true" />
        <span class="session-card__state-label">{props.session.state}</span>
      </div>
      <footer class="session-card__footer">
        <span class="session-card__provider">{props.session.provider ?? '—'}</span>
        <span class="session-card__last">{formatRelative(props.session.lastEventAt)}</span>
      </footer>
    </button>
  );
};

/** Cheap relative-time formatter — no external date lib. */
export function formatRelative(iso: string, now: Date = new Date()): string {
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return '—';
  const seconds = Math.floor((now.getTime() - then.getTime()) / 1000);
  if (seconds < 60) return 'just now';
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  if (days < 7) return `${days}d ago`;
  return then.toISOString().slice(0, 10);
}
