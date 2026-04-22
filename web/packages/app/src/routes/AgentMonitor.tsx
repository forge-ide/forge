// Agent Monitor route (F-140). Three-column layout per
// `docs/ui-specs/agent-monitor.md` §9: list (280px) | trace (flex) | inspector
// (340px). Renders the union of background agents (fetched via
// `list_background_agents`) and sub-agents surfaced through `session:event`
// (F-134 `SubAgentSpawned`, F-139 Step*), plus a session-root row so the
// active turn's trace has a home when no sub-agent exists.
//
// Event plumbing kept intentionally thin: the view subscribes to the existing
// `sessionEvents` store and re-derives timelines from its most-recent-event
// signal. A richer aggregator can land alongside resource sampling (cpu/rss/
// fds) — see §9.3 "Resource usage" — without reshaping this file.

import {
  createEffect,
  createMemo,
  createResource,
  createSignal,
  For,
  on,
  onCleanup,
  onMount,
  Show,
  type Component,
  type JSX,
} from 'solid-js';
import { useParams, useSearchParams } from '@solidjs/router';
import { invoke } from '../lib/tauri';
import { useFocusTrap } from '../lib/useFocusTrap';
import type { BgAgentSummary } from '@forge/ipc';
import { onSessionEvent, type SessionEventPayload } from '../ipc/session';
import './AgentMonitor.css';

// ---------------------------------------------------------------------------
// Wire shapes + domain types
// ---------------------------------------------------------------------------

/** Filter tabs per spec §9.1. */
export type AgentFilter = 'all' | 'running' | 'background' | 'session' | 'failed';

/** Row state drives progress-bar color + pulsing ring. */
export type AgentRowState = 'running' | 'queued' | 'done' | 'error';

export type StepKind = 'plan' | 'tool' | 'model' | 'wait' | 'spawn';

/** Status for a timeline step — `running` renders the pulsing ring. */
export type StepStatus = 'running' | 'done' | 'error';

/** Classification used for sorting + filter assignment. */
export type AgentCategory = 'session' | 'background' | 'sub-agent';

export interface AgentRow {
  id: string;
  name: string;
  category: AgentCategory;
  state: AgentRowState;
  /** Parent row id when `category === 'sub-agent'`. */
  parentId?: string;
  /** 0..1 — drives left-column progress bar width. */
  progress: number;
  /** Relative start ISO timestamp; `undefined` collapses the meta row. */
  startedAt?: string;
  /** Free-form model label; shown on the meta row. */
  model?: string;
}

export interface AgentStep {
  id: string;
  kind: StepKind;
  title: string;
  status: StepStatus;
  startedAt: string;
  /** Optional preview shown in the drawer; mono 11px per spec §9.2. */
  preview?: string;
}

/** Inspector pane datum — fields absent from the backend render as `-`. */
export interface AgentInspectorData {
  source?: string;
  provider?: string;
  model?: string;
  isolation?: string;
  maxTokens?: number;
  allowedTools: string[];
  allowedPaths: string[];
  /** `undefined` renders as `-`; no placeholder zeros so missing data is visible. */
  resources: {
    cpu?: number;
    rss?: number;
    fds?: number;
  };
}

/** Per-instance resource-sample snapshot folded from F-152
 * `Event::ResourceSample`. Stored in the live state map so the Inspector's
 * cpu / rss / fds pills read the most recent value. `null` fields in the
 * wire payload survive as `undefined` here so the pill renders the `—`
 * placeholder the design calls out. */
export interface ResourceSnapshot {
  cpu?: number;
  rss?: number;
  fds?: number;
}

// ---------------------------------------------------------------------------
// Filter + sort
// ---------------------------------------------------------------------------

/** Public so the tests can exercise the sort invariant directly. */
export function filterAgents(rows: AgentRow[], filter: AgentFilter): AgentRow[] {
  switch (filter) {
    case 'all':
      return rows;
    case 'running':
      return rows.filter((r) => r.state === 'running');
    case 'background':
      return rows.filter((r) => r.category === 'background');
    case 'session':
      return rows.filter((r) => r.category === 'session' || r.category === 'sub-agent');
    case 'failed':
      return rows.filter((r) => r.state === 'error');
  }
}

// Spec §9.1 sort: running → queued → error → done, each group most-recent-first.
const STATE_ORDER: Record<AgentRowState, number> = {
  running: 0,
  queued: 1,
  error: 2,
  done: 3,
};

export function sortAgents(rows: AgentRow[]): AgentRow[] {
  return [...rows].sort((a, b) => {
    const byState = STATE_ORDER[a.state] - STATE_ORDER[b.state];
    if (byState !== 0) return byState;
    // Recent-first — missing startedAt sinks.
    const aStarted = a.startedAt ? Date.parse(a.startedAt) : -Infinity;
    const bStarted = b.startedAt ? Date.parse(b.startedAt) : -Infinity;
    return bStarted - aStarted;
  });
}

// ---------------------------------------------------------------------------
// Live agent state assembled from bg-agents + session events
// ---------------------------------------------------------------------------

interface InternalAgentState {
  sessionRoot: AgentRow;
  rows: AgentRow[];
  stepsByAgent: Record<string, AgentStep[]>;
}

/** Snapshot of the live event-driven state. Exported for the unit test
 * that pins the session-root upsert path without mounting a full route.
 */
export interface LiveAgentState {
  subAgents: AgentRow[];
  stepsByAgent: Record<string, AgentStep[]>;
  /** F-152: most recent resource sample per instance id. Undefined keys
   * render as `—` pills in the Inspector. A completion event clears the
   * entry so the pills reset automatically. */
  resourcesByAgent: Record<string, ResourceSnapshot>;
}

/**
 * Pure folder for a `session:event` payload. Adding sub-agent rows, step
 * timelines, and session-root rows lands here so the logic can be pinned
 * without mounting `<AgentMonitor>`. Callers pass the prior state and get
 * back the post-event snapshot; a no-op event returns the input reference
 * unchanged so a SolidJS setter short-circuits.
 */
export function applyEventToState(
  prev: LiveAgentState,
  payload: { event: unknown },
): LiveAgentState {
  const ev = payload.event as Record<string, unknown> | null;
  if (!ev || typeof ev !== 'object') return prev;
  const type = ev['type'];

  if (type === 'sub_agent_spawned') {
    const parent = ev['parent'];
    const child = ev['child'];
    if (typeof parent !== 'string' || typeof child !== 'string') return prev;
    const next: AgentRow = {
      id: child,
      name: `sub-agent ${child.slice(0, 8)}`,
      category: 'sub-agent',
      state: 'running',
      parentId: parent,
      progress: 0.3,
      startedAt: new Date().toISOString(),
    };
    return {
      ...prev,
      subAgents: [...prev.subAgents.filter((r) => r.id !== child), next],
    };
  }

  if (type === 'step_started') {
    const stepId = ev['step_id'];
    const kind = ev['kind'];
    const instanceId = ev['instance_id'];
    if (typeof stepId !== 'string' || typeof kind !== 'string') return prev;
    const agentId =
      typeof instanceId === 'string' && instanceId.length > 0
        ? instanceId
        : 'session-root';
    // Upsert a row so session-root steps (live `StepStarted.instance_id`)
    // attach to a selectable row in the left column. The legacy
    // `'session-root'` fallback covers a pre-F-140 daemon where
    // `instance_id` is `None` — the UI still needs a row to hang steps off.
    let subAgents = prev.subAgents;
    const hasRow = prev.subAgents.some((r) => r.id === agentId);
    if (!hasRow) {
      const label =
        agentId === 'session-root'
          ? 'session'
          : `session ${agentId.slice(0, 8)}`;
      subAgents = [
        ...prev.subAgents,
        {
          id: agentId,
          name: label,
          category: 'session',
          state: 'running',
          progress: 0.3,
          startedAt: new Date().toISOString(),
        },
      ];
    }
    const startedAt =
      typeof ev['started_at'] === 'string'
        ? (ev['started_at'] as string)
        : new Date().toISOString();
    const newStep: AgentStep = {
      id: stepId,
      kind: normaliseKind(kind),
      title: `${kind} step`,
      status: 'running',
      startedAt,
    };
    const existing = prev.stepsByAgent[agentId] ?? [];
    return {
      ...prev,
      subAgents,
      stepsByAgent: { ...prev.stepsByAgent, [agentId]: [...existing, newStep] },
    };
  }

  if (type === 'step_finished') {
    const stepId = ev['step_id'];
    if (typeof stepId !== 'string') return prev;
    const out: Record<string, AgentStep[]> = {};
    for (const k of Object.keys(prev.stepsByAgent)) {
      const steps = prev.stepsByAgent[k];
      if (!steps) continue;
      out[k] = steps.map((s) =>
        s.id === stepId
          ? {
              ...s,
              status: outcomeOf(ev['outcome']) === 'error' ? 'error' : 'done',
            }
          : s,
      );
    }
    return { ...prev, stepsByAgent: out };
  }

  if (type === 'background_agent_completed') {
    // F-140: when the Stop button (or a natural terminal transition) fires
    // a `BackgroundAgentCompleted` event on `session:event`, flip the
    // matching sub-agent row to its terminal variant in-place instead of
    // dropping it silently. The row stays in the left column so the user
    // can still inspect the trace + definition — the progress bar + pulse
    // turn off, the state chip reads "done".
    //
    // Background agents (category `'background'`) are refetched from
    // `list_background_agents` on this event elsewhere in the route; those
    // rows re-render from backend state. This branch handles the sub-agent
    // + session-root rows that only live in the in-memory `subAgents`
    // signal.
    const id = ev['id'];
    if (typeof id !== 'string') return prev;
    let changed = false;
    const nextSubAgents = prev.subAgents.map((r) => {
      if (r.id !== id || r.state === 'done' || r.state === 'error') return r;
      changed = true;
      return { ...r, state: 'done' as AgentRowState, progress: 1 };
    });
    // F-152: clearing the resources map entry on terminal transition is
    // the mechanism behind "pills clear back to '—' when the instance
    // terminates" — the inspector reads from this map and an absent key
    // resolves to undefined → dash.
    const hadResources = id in prev.resourcesByAgent;
    if (!changed && !hadResources) return prev;
    let nextResources = prev.resourcesByAgent;
    if (hadResources) {
      const { [id]: _cleared, ...rest } = prev.resourcesByAgent;
      nextResources = rest;
    }
    return {
      ...prev,
      subAgents: changed ? nextSubAgents : prev.subAgents,
      resourcesByAgent: nextResources,
    };
  }

  if (type === 'resource_sample') {
    // F-152: fold the sampler's per-instance emission into the live
    // resources map. Missing fields (Option<None> on the Rust side) arrive
    // as `null` on the wire; they're preserved as `undefined` here so the
    // pill renders the `—` placeholder without falsely reading as zero.
    const instanceId = ev['instance_id'];
    if (typeof instanceId !== 'string') return prev;
    const snapshot: ResourceSnapshot = {};
    const cpu = ev['cpu_pct'];
    if (typeof cpu === 'number') snapshot.cpu = cpu;
    const rss = ev['rss_bytes'];
    if (typeof rss === 'number') snapshot.rss = rss;
    const fds = ev['fd_count'];
    if (typeof fds === 'number') snapshot.fds = fds;
    return {
      ...prev,
      resourcesByAgent: { ...prev.resourcesByAgent, [instanceId]: snapshot },
    };
  }

  return prev;
}

function bgState(s: BgAgentSummary['state']): AgentRowState {
  switch (s) {
    case 'Running':
      return 'running';
    case 'Completed':
      return 'done';
    case 'Failed':
      return 'error';
    default:
      return 'queued';
  }
}

function toBgRow(s: BgAgentSummary): AgentRow {
  return {
    id: s.id,
    name: s.agent_name,
    category: 'background',
    state: bgState(s.state),
    progress: s.state === 'Completed' ? 1 : s.state === 'Failed' ? 1 : 0.5,
  };
}

// ---------------------------------------------------------------------------
// Left column — agent list
// ---------------------------------------------------------------------------

export const AgentList: Component<{
  rows: AgentRow[];
  filter: AgentFilter;
  onFilter: (f: AgentFilter) => void;
  selectedId: string | null;
  onSelect: (id: string) => void;
}> = (props) => {
  const visible = createMemo(() => sortAgents(filterAgents(props.rows, props.filter)));

  // F-416: tabs ↔ tabpanel association (WAI-ARIA APG tabs pattern). A
  // single tabpanel hosts the row list; its aria-labelledby always points
  // at the currently-selected filter tab so assistive tech can announce
  // the panel's context when filter selection changes.
  const filterTabId = (f: AgentFilter) => `agent-filter-tab-${f}`;
  const rowsPanelId = 'agent-filter-rows-panel';

  return (
    <aside class="agent-monitor__list" aria-label="Agents">
      <div role="tablist" class="agent-monitor__filters">
        <For each={FILTERS}>
          {(f) => (
            <button
              type="button"
              role="tab"
              id={filterTabId(f)}
              aria-controls={rowsPanelId}
              aria-selected={props.filter === f}
              class="agent-monitor__filter"
              classList={{ 'agent-monitor__filter--active': props.filter === f }}
              onClick={() => props.onFilter(f)}
            >
              {f}
            </button>
          )}
        </For>
      </div>
      <Show
        when={visible().length > 0}
        fallback={
          <p
            class="agent-monitor__empty"
            id={rowsPanelId}
            role="tabpanel"
            aria-labelledby={filterTabId(props.filter)}
          >
            // no agents
          </p>
        }
      >
        <ul
          class="agent-monitor__rows"
          id={rowsPanelId}
          role="tabpanel"
          aria-labelledby={filterTabId(props.filter)}
        >
          <For each={visible()}>
            {(row) => (
              <AgentListRow
                row={row}
                active={props.selectedId === row.id}
                onSelect={props.onSelect}
              />
            )}
          </For>
        </ul>
      </Show>
    </aside>
  );
};

const FILTERS: AgentFilter[] = ['all', 'running', 'background', 'session', 'failed'];

const AgentListRow: Component<{
  row: AgentRow;
  active: boolean;
  onSelect: (id: string) => void;
}> = (props) => {
  const widthStyle = (): JSX.CSSProperties => ({
    width: `${Math.max(0, Math.min(1, props.row.progress)) * 100}%`,
  });
  return (
    <li>
      <button
        type="button"
        class="agent-monitor__row"
        classList={{
          'agent-monitor__row--active': props.active,
          'agent-monitor__row--running': props.row.state === 'running',
        }}
        aria-label={`Select agent ${props.row.name}`}
        aria-current={props.active ? 'true' : undefined}
        onClick={() => props.onSelect(props.row.id)}
      >
        <span class="agent-monitor__row-name">
          <Show when={props.row.state === 'running'}>
            <span class="agent-monitor__row-pulse" aria-hidden="true" />
          </Show>
          {props.row.name}
        </span>
        <span class="agent-monitor__row-meta">
          {props.row.category === 'background' ? 'BG' : props.row.category}
          {props.row.model ? ` · ${props.row.model}` : ''}
        </span>
        <span
          class="agent-monitor__progress"
          data-state={props.row.state}
          aria-hidden="true"
        >
          <span class="agent-monitor__progress-fill" style={widthStyle()} />
        </span>
      </button>
    </li>
  );
};

// ---------------------------------------------------------------------------
// Middle column — trace timeline
// ---------------------------------------------------------------------------

export const AgentTrace: Component<{
  agent: AgentRow | null;
  steps: AgentStep[];
  onStepClick: (step: AgentStep) => void;
}> = (props) => {
  return (
    <section class="agent-monitor__trace" aria-label="Trace">
      <Show
        when={props.agent}
        fallback={<p class="agent-monitor__empty">// select an agent</p>}
      >
        {(agent) => (
          <>
            <header class="agent-monitor__trace-head">
              <h2 class="agent-monitor__trace-name">{agent().name}</h2>
              <span class="agent-monitor__trace-id">{agent().id}</span>
            </header>
            <Show
              when={props.steps.length > 0}
              fallback={<p class="agent-monitor__empty">// no steps yet</p>}
            >
              <ol class="agent-monitor__steps">
                <For each={props.steps}>
                  {(step) => <AgentTraceStep step={step} onClick={props.onStepClick} />}
                </For>
              </ol>
            </Show>
          </>
        )}
      </Show>
    </section>
  );
};

const AgentTraceStep: Component<{
  step: AgentStep;
  onClick: (step: AgentStep) => void;
}> = (props) => {
  return (
    <li class="agent-monitor__step">
      <button
        type="button"
        class="agent-monitor__step-btn"
        aria-label={`Open step ${props.step.title}`}
        onClick={() => props.onClick(props.step)}
      >
        <span
          class="agent-monitor__step-dot"
          classList={{
            'agent-monitor__step-dot--running': props.step.status === 'running',
            'agent-monitor__step-dot--error': props.step.status === 'error',
          }}
          aria-hidden="true"
        />
        <span class="agent-monitor__step-body">
          <span
            class="agent-monitor__step-chip"
            data-kind={props.step.kind}
            aria-label={`step kind ${props.step.kind}`}
          >
            {props.step.kind}
          </span>
          <span class="agent-monitor__step-title">{props.step.title}</span>
        </span>
      </button>
    </li>
  );
};

// ---------------------------------------------------------------------------
// Right column — inspector
// ---------------------------------------------------------------------------

export const AgentInspector: Component<{
  agent: AgentRow | null;
  data: AgentInspectorData | null;
  onStop: (id: string) => void;
}> = (props) => {
  return (
    <aside class="agent-monitor__inspector" aria-label="Inspector">
      <Show
        when={props.agent && props.data}
        fallback={<p class="agent-monitor__empty">// select an agent</p>}
      >
        <section>
          <h3 class="agent-monitor__inspector-heading">Definition</h3>
          <dl class="agent-monitor__def">
            <dt>name</dt>
            <dd>{props.agent!.name}</dd>
            <dt>source</dt>
            <dd>{props.data!.source ?? '—'}</dd>
            <dt>provider</dt>
            <dd>{props.data!.provider ?? '—'}</dd>
            <dt>model</dt>
            <dd>{props.data!.model ?? '—'}</dd>
            <dt>isolation</dt>
            <dd>{props.data!.isolation ?? '—'}</dd>
          </dl>
        </section>
        <section>
          <h3 class="agent-monitor__inspector-heading">Allowed tools</h3>
          <Show
            when={props.data!.allowedTools.length > 0}
            fallback={<p class="agent-monitor__empty">// none</p>}
          >
            <ul class="agent-monitor__pills">
              <For each={props.data!.allowedTools}>
                {(tool) => <li class="agent-monitor__pill">{tool}</li>}
              </For>
            </ul>
          </Show>
        </section>
        <section>
          <h3 class="agent-monitor__inspector-heading">Allowed paths</h3>
          <Show
            when={props.data!.allowedPaths.length > 0}
            fallback={<p class="agent-monitor__empty">// none</p>}
          >
            <ul class="agent-monitor__pills">
              <For each={props.data!.allowedPaths}>
                {(p) => (
                  <li class="agent-monitor__pill agent-monitor__pill--mono">{p}</li>
                )}
              </For>
            </ul>
          </Show>
        </section>
        <section>
          <h3 class="agent-monitor__inspector-heading">Resource usage</h3>
          <ul class="agent-monitor__pills">
            <li class="agent-monitor__pill">
              cpu {props.data!.resources.cpu != null ? `${props.data!.resources.cpu.toFixed(1)}%` : '—'}
            </li>
            <li class="agent-monitor__pill">
              rss {props.data!.resources.rss != null ? `${props.data!.resources.rss}MB` : '—'}
            </li>
            <li class="agent-monitor__pill">
              fds {props.data!.resources.fds != null ? props.data!.resources.fds : '—'}
            </li>
          </ul>
        </section>
        <section>
          <button
            type="button"
            class="agent-monitor__stop"
            onClick={() => props.onStop(props.agent!.id)}
          >
            Stop agent
          </button>
        </section>
      </Show>
    </aside>
  );
};

// ---------------------------------------------------------------------------
// Step-detail drawer
// ---------------------------------------------------------------------------

export const StepDrawer: Component<{
  step: AgentStep | null;
  onClose: () => void;
}> = (props) => {
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') props.onClose();
  };

  createEffect(() => {
    if (props.step) {
      document.addEventListener('keydown', onKey);
      onCleanup(() => document.removeEventListener('keydown', onKey));
    }
  });

  // F-402: extract the dialog body so its mount/unmount drives useFocusTrap —
  // focus lands inside the drawer on open, Tab traps within, focus restores
  // to the previously-focused element on close.
  const Body: Component<{ step: AgentStep }> = (p) => {
    let dialogRef: HTMLDivElement | undefined;
    useFocusTrap(() => dialogRef);
    return (
      <div
        ref={dialogRef}
        class="agent-monitor__drawer"
        role="dialog"
        aria-label="Step detail"
        aria-modal="true"
      >
        <header class="agent-monitor__drawer-head">
          <h3>{p.step.title}</h3>
          <button
            type="button"
            class="agent-monitor__drawer-close"
            aria-label="Close step detail"
            onClick={props.onClose}
          >
            ×
          </button>
        </header>
        <dl class="agent-monitor__def">
          <dt>kind</dt>
          <dd>{p.step.kind}</dd>
          <dt>status</dt>
          <dd>{p.step.status}</dd>
          <dt>started</dt>
          <dd>{p.step.startedAt}</dd>
        </dl>
        <Show when={p.step.preview}>
          <pre class="agent-monitor__drawer-preview">{p.step.preview}</pre>
        </Show>
      </div>
    );
  };

  return (
    <Show when={props.step}>
      {(step) => <Body step={step()} />}
    </Show>
  );
};

// ---------------------------------------------------------------------------
// Route shell — assembles columns + data sources
// ---------------------------------------------------------------------------

async function fetchBgAgents(sessionId: string | null): Promise<BgAgentSummary[]> {
  if (!sessionId) return [];
  try {
    const result = await invoke<BgAgentSummary[]>('list_background_agents', {
      sessionId,
    });
    return Array.isArray(result) ? result : [];
  } catch {
    return [];
  }
}

/**
 * Stop a running agent instance by calling the `stop_background_agent`
 * Tauri command (F-138). Exported so the component test can exercise the
 * wiring without mounting the full `AgentMonitor` route shell (which needs
 * a router, live session event bus, and a resource loader).
 *
 * Returns a discriminated result instead of throwing so the Inspector's
 * click handler can stay idempotent — a stale id, a missing session id, or
 * an invoke rejection all collapse to `skipped` / `failed` without
 * bubbling.
 *
 * The Tauri boundary sends `{ sessionId, instanceId }`; the backend's
 * `stop_background_agent` command authorizes via `require_window_label`
 * and delegates to `Orchestrator::stop`, which is a silent no-op on
 * stale ids — so the "click Stop on an already-terminal row" race is
 * provably safe even end-to-end.
 */
export async function stopAgentInstance(
  deps: { invoke: typeof invoke },
  sessionId: string | null,
  instanceId: string,
): Promise<'ok' | 'skipped' | 'failed'> {
  if (!sessionId) return 'skipped';
  try {
    await deps.invoke<void>('stop_background_agent', {
      sessionId,
      instanceId,
    });
    return 'ok';
  } catch {
    return 'failed';
  }
}

/** Inspector stub — real data lands when the backend exposes def metadata.
 *
 * F-152: the `resources` field accepts the live snapshot from
 * `resourcesByAgent` so the pills show real sampler output instead of
 * placeholder dashes. Passing `undefined` preserves pre-F-152 behavior
 * (pills show `—`), which is exactly what happens when an instance is
 * untracked or terminated.
 *
 * The snapshot carries `rss` in bytes (Rust wire shape `rss_bytes`); the
 * pill renders the label "MB", so we divide by 1024*1024 here to keep the
 * display units honest. `cpu` is already a percent (0..=100); `fds` is a
 * raw count.
 */
export function inspectorStub(
  row: AgentRow,
  resources?: ResourceSnapshot,
): AgentInspectorData {
  const data: AgentInspectorData = {
    allowedTools: [],
    allowedPaths: [],
    resources: {},
  };
  if (resources) {
    if (resources.cpu !== undefined) data.resources.cpu = resources.cpu;
    if (resources.rss !== undefined) {
      data.resources.rss = Math.round(resources.rss / (1024 * 1024));
    }
    if (resources.fds !== undefined) data.resources.fds = resources.fds;
  }
  if (row.model) data.model = row.model;
  return data;
}

export const AgentMonitor: Component = () => {
  const params = useParams<{ id?: string }>();
  const sessionId = () => params.id ?? null;
  // F-153: honor `?instance=<id>` from the status-bar badge's nav so the
  // monitor pre-selects the clicked row instead of the default first row.
  // Param absence falls through to the auto-select-first effect below.
  const [searchParams] = useSearchParams<{ instance?: string }>();

  const [filter, setFilter] = createSignal<AgentFilter>('all');
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [openStep, setOpenStep] = createSignal<AgentStep | null>(null);

  // Background agents — refetched when the session id changes.
  const [bgAgents, { refetch }] = createResource(sessionId, fetchBgAgents, {
    initialValue: [],
  });

  // Live rows observed via session:event — session-root and sub-agents. The
  // helper upserts a session row the first time `StepStarted.instance_id`
  // arrives, so the empty state transitions to a populated trace without a
  // hardcoded placeholder competing with the live id.
  const [subAgents, setSubAgents] = createSignal<AgentRow[]>([]);
  const [stepsByAgent, setStepsByAgent] = createSignal<Record<string, AgentStep[]>>({});
  // F-152: per-instance cpu / rss / fds folded from `resource_sample`
  // events. A completion event clears the entry so the pills reset.
  const [resourcesByAgent, setResourcesByAgent] = createSignal<
    Record<string, ResourceSnapshot>
  >({});

  const rows = createMemo<AgentRow[]>(() => [
    ...bgAgents().map(toBgRow),
    ...subAgents(),
  ]);

  onMount(async () => {
    // Subscribe to session events to update sub-agents + step timelines.
    const unlisten = await onSessionEvent((payload) => applyEvent(payload));
    onCleanup(() => unlisten());
  });

  function applyEvent(payload: SessionEventPayload) {
    const ev = payload.event as Record<string, unknown> | null;
    if (ev && typeof ev === 'object' && ev['type'] === 'background_agent_completed') {
      void refetch();
    }
    // All other variants fold through the pure helper — keeps the logic
    // testable without mounting the route.
    const snapshot: LiveAgentState = {
      subAgents: subAgents(),
      stepsByAgent: stepsByAgent(),
      resourcesByAgent: resourcesByAgent(),
    };
    const next = applyEventToState(snapshot, payload);
    if (next === snapshot) return;
    if (next.subAgents !== snapshot.subAgents) setSubAgents(next.subAgents);
    if (next.stepsByAgent !== snapshot.stepsByAgent) setStepsByAgent(next.stepsByAgent);
    if (next.resourcesByAgent !== snapshot.resourcesByAgent) {
      setResourcesByAgent(next.resourcesByAgent);
    }
  }

  // Auto-select the session root so the trace column isn't empty on mount.
  // F-153: when `?instance=<id>` is present, prefer selecting that row.
  // Falls back to the first row when the requested instance isn't in the
  // current list (stale id, navigation from a different session, etc.).
  createEffect(
    on(rows, (list) => {
      if (selectedId() || list.length === 0) return;
      const wanted = searchParams.instance;
      if (wanted) {
        const match = list.find((r) => r.id === wanted);
        if (match) {
          setSelectedId(match.id);
          return;
        }
      }
      const first = list[0];
      if (first) setSelectedId(first.id);
    }),
  );

  const selected = createMemo(() =>
    rows().find((r) => r.id === selectedId()) ?? null,
  );
  const selectedSteps = createMemo<AgentStep[]>(() => {
    const id = selectedId();
    if (!id) return [];
    return stepsByAgent()[id] ?? [];
  });
  const inspector = createMemo<AgentInspectorData | null>(() => {
    const row = selected();
    if (!row) return null;
    const resources = resourcesByAgent()[row.id];
    return inspectorStub(row, resources);
  });

  const onStop = async (id: string) => {
    // Wire the inspector's Stop button to the `stop_background_agent` Tauri
    // command (F-140). F-138 already ships this command end-to-end — here
    // we route the Agent Monitor's Stop click through it so clicking the
    // button drives `forge_agents::Orchestrator::stop(id)`. The
    // orchestrator's resulting `AgentEvent::Completed` is forwarded by the
    // per-session background registry as `Event::BackgroundAgentCompleted`
    // on `session:event`; `applyEventToState` folds that into the row's
    // terminal variant so the UI doesn't need any extra reconciliation.
    //
    // Failures (stale id, network) are swallowed: clicking Stop on a row
    // that has already transitioned to terminal must not surface an error
    // — the orchestrator itself is idempotent on unknown ids, but a
    // concurrent terminal transition between render and click is the most
    // likely "failure" path and should be invisible to the user.
    await stopAgentInstance({ invoke }, sessionId(), id);
  };

  return (
    <main class="agent-monitor">
      <AgentList
        rows={rows()}
        filter={filter()}
        onFilter={setFilter}
        selectedId={selectedId()}
        onSelect={setSelectedId}
      />
      <AgentTrace
        agent={selected()}
        steps={selectedSteps()}
        onStepClick={setOpenStep}
      />
      <AgentInspector agent={selected()} data={inspector()} onStop={onStop} />
      <StepDrawer step={openStep()} onClose={() => setOpenStep(null)} />
    </main>
  );
};

function normaliseKind(raw: string): StepKind {
  const k = raw.toLowerCase();
  if (k === 'plan' || k === 'tool' || k === 'model' || k === 'wait' || k === 'spawn') {
    return k;
  }
  return 'model';
}

function outcomeOf(raw: unknown): 'ok' | 'error' {
  if (raw && typeof raw === 'object') {
    const status = (raw as { status?: unknown }).status;
    if (status === 'error') return 'error';
  }
  return 'ok';
}
