import { describe, expect, it, vi } from 'vitest';
import { cleanup, render, fireEvent } from '@solidjs/testing-library';
import {
  AgentInspector,
  AgentList,
  AgentTrace,
  applyEventToState,
  filterAgents,
  sortAgents,
  StepDrawer,
  stopAgentInstance,
  type AgentInspectorData,
  type AgentRow,
  type AgentStep,
  type LiveAgentState,
} from './AgentMonitor';
import { afterEach } from 'vitest';

afterEach(() => cleanup());

const row = (over: Partial<AgentRow> = {}): AgentRow => ({
  id: 'a1',
  name: 'coder',
  category: 'sub-agent',
  state: 'running',
  progress: 0.4,
  ...over,
});

const step = (over: Partial<AgentStep> = {}): AgentStep => ({
  id: 's1',
  kind: 'tool',
  title: 'fs.read readme.txt',
  status: 'running',
  startedAt: '2026-04-20T12:00:00Z',
  ...over,
});

const inspector = (over: Partial<AgentInspectorData> = {}): AgentInspectorData => ({
  allowedTools: [],
  allowedPaths: [],
  resources: {},
  ...over,
});

// ---------------------------------------------------------------------------
// Filter + sort helpers — exercised directly so the invariant is pinned
// independent of rendering.
// ---------------------------------------------------------------------------

describe('AgentMonitor: filter + sort helpers', () => {
  const rows: AgentRow[] = [
    row({ id: 'done', state: 'done', progress: 1, startedAt: '2026-04-20T10:00Z' }),
    row({ id: 'run', state: 'running', progress: 0.5, startedAt: '2026-04-20T11:00Z' }),
    row({ id: 'err', state: 'error', progress: 1, startedAt: '2026-04-20T10:30Z' }),
    row({
      id: 'bg',
      category: 'background',
      state: 'queued',
      progress: 0,
      startedAt: '2026-04-20T09:00Z',
    }),
  ];

  it('filters "running" to only running rows', () => {
    const out = filterAgents(rows, 'running');
    expect(out.map((r) => r.id)).toEqual(['run']);
  });

  it('filters "background" to only the background category', () => {
    const out = filterAgents(rows, 'background');
    expect(out.map((r) => r.id)).toEqual(['bg']);
  });

  it('filters "failed" to only errored rows', () => {
    const out = filterAgents(rows, 'failed');
    expect(out.map((r) => r.id)).toEqual(['err']);
  });

  it('sorts by state (running → queued → error → done), most recent first within group', () => {
    const sorted = sortAgents(rows);
    // running first, then queued (bg), then error, then done
    expect(sorted.map((r) => r.id)).toEqual(['run', 'bg', 'err', 'done']);
  });
});

// ---------------------------------------------------------------------------
// Live event folding — the path from `session:event` payload to rendered state
// ---------------------------------------------------------------------------

describe('applyEventToState', () => {
  const empty: LiveAgentState = { subAgents: [], stepsByAgent: {} };

  it('upserts a session row the first time StepStarted arrives with an instance id', () => {
    const after = applyEventToState(empty, {
      event: {
        type: 'step_started',
        step_id: 'step-1',
        kind: 'model',
        instance_id: 'abc123def456',
        started_at: '2026-04-20T12:00:00Z',
      },
    });

    expect(after.subAgents).toHaveLength(1);
    expect(after.subAgents[0]?.id).toBe('abc123def456');
    expect(after.subAgents[0]?.category).toBe('session');
    expect(after.stepsByAgent['abc123def456']).toHaveLength(1);
    expect(after.stepsByAgent['abc123def456']?.[0]?.kind).toBe('model');
  });

  it('does not duplicate the row when further StepStarted events arrive for the same instance id', () => {
    const s1 = applyEventToState(empty, {
      event: {
        type: 'step_started',
        step_id: 'step-1',
        kind: 'model',
        instance_id: 'inst-1',
      },
    });
    const s2 = applyEventToState(s1, {
      event: {
        type: 'step_started',
        step_id: 'step-2',
        kind: 'tool',
        instance_id: 'inst-1',
      },
    });

    expect(s2.subAgents).toHaveLength(1);
    expect(s2.stepsByAgent['inst-1']).toHaveLength(2);
  });

  it('adds a sub-agent row on SubAgentSpawned', () => {
    const after = applyEventToState(empty, {
      event: { type: 'sub_agent_spawned', parent: 'p', child: 'c' },
    });

    expect(after.subAgents).toHaveLength(1);
    expect(after.subAgents[0]?.category).toBe('sub-agent');
    expect(after.subAgents[0]?.parentId).toBe('p');
  });

  it('closes a step to done on StepFinished with ok outcome', () => {
    const started = applyEventToState(empty, {
      event: {
        type: 'step_started',
        step_id: 's',
        kind: 'model',
        instance_id: 'inst',
      },
    });
    const finished = applyEventToState(started, {
      event: {
        type: 'step_finished',
        step_id: 's',
        outcome: { status: 'ok' },
      },
    });

    expect(finished.stepsByAgent['inst']?.[0]?.status).toBe('done');
  });

  it('closes a step to error on StepFinished with error outcome', () => {
    const started = applyEventToState(empty, {
      event: {
        type: 'step_started',
        step_id: 's',
        kind: 'tool',
        instance_id: 'inst',
      },
    });
    const finished = applyEventToState(started, {
      event: {
        type: 'step_finished',
        step_id: 's',
        outcome: { status: 'error', reason: 'nope' },
      },
    });

    expect(finished.stepsByAgent['inst']?.[0]?.status).toBe('error');
  });

  it('returns the same reference for an unrecognised event type', () => {
    const after = applyEventToState(empty, {
      event: { type: 'user_message', id: 'm', text: 'hi' },
    });
    expect(after).toBe(empty);
  });

  it('flips a running sub-agent row to done on background_agent_completed', () => {
    // F-140: Stop button → `stop_background_agent` → orchestrator.stop →
    // `BackgroundAgentCompleted` on `session:event`. The row must stay
    // visible but transition to a terminal variant so the user can still
    // inspect the trace.
    const spawned = applyEventToState(empty, {
      event: { type: 'sub_agent_spawned', parent: 'p', child: 'c' },
    });
    expect(spawned.subAgents[0]?.state).toBe('running');

    const completed = applyEventToState(spawned, {
      event: { type: 'background_agent_completed', id: 'c' },
    });
    const row = completed.subAgents.find((r) => r.id === 'c');
    expect(row?.state).toBe('done');
    expect(row?.progress).toBe(1);
  });

  it('is a no-op when the completed id has no tracked sub-agent row', () => {
    const after = applyEventToState(empty, {
      event: { type: 'background_agent_completed', id: 'unknown' },
    });
    expect(after).toBe(empty);
  });

  it('does not re-flip an already-terminal row', () => {
    const spawned = applyEventToState(empty, {
      event: { type: 'sub_agent_spawned', parent: 'p', child: 'c' },
    });
    const first = applyEventToState(spawned, {
      event: { type: 'background_agent_completed', id: 'c' },
    });
    const second = applyEventToState(first, {
      event: { type: 'background_agent_completed', id: 'c' },
    });
    // Stable reference proves we did not allocate a new state tree for a
    // redundant completion event.
    expect(second).toBe(first);
  });
});

// ---------------------------------------------------------------------------
// Left column: list
// ---------------------------------------------------------------------------

describe('<AgentList>', () => {
  it('renders one tab per filter category (5 total)', () => {
    const { getAllByRole } = render(() => (
      <AgentList
        rows={[row()]}
        filter="all"
        onFilter={() => {}}
        selectedId={null}
        onSelect={() => {}}
      />
    ));
    const tabs = getAllByRole('tab');
    expect(tabs).toHaveLength(5);
    expect(tabs.map((t) => t.textContent?.trim())).toEqual([
      'all',
      'running',
      'background',
      'session',
      'failed',
    ]);
  });

  it('invokes onFilter when a filter tab is clicked', () => {
    const onFilter = vi.fn();
    const { getAllByRole } = render(() => (
      <AgentList
        rows={[]}
        filter="all"
        onFilter={onFilter}
        selectedId={null}
        onSelect={() => {}}
      />
    ));
    const tabs = getAllByRole('tab');
    const bgTab = tabs[2];
    if (!bgTab) throw new Error('expected 5 filter tabs, got fewer');
    fireEvent.click(bgTab); // background
    expect(onFilter).toHaveBeenCalledWith('background');
  });

  it('renders a per-row progress bar with width proportional to progress', () => {
    const { container } = render(() => (
      <AgentList
        rows={[row({ progress: 0.75 })]}
        filter="all"
        onFilter={() => {}}
        selectedId={null}
        onSelect={() => {}}
      />
    ));
    const fill = container.querySelector('.agent-monitor__progress-fill') as HTMLElement;
    expect(fill).toBeTruthy();
    expect(fill.style.width).toBe('75%');
  });

  it('applies the running modifier so the pulsing-ring style attaches', () => {
    const { container } = render(() => (
      <AgentList
        rows={[row({ state: 'running' })]}
        filter="all"
        onFilter={() => {}}
        selectedId={null}
        onSelect={() => {}}
      />
    ));
    const btn = container.querySelector('.agent-monitor__row--running');
    expect(btn).toBeTruthy();
    // Pulse marker is rendered inside the row name span.
    expect(container.querySelector('.agent-monitor__row-pulse')).toBeTruthy();
  });

  it('calls onSelect when a row is clicked', () => {
    const onSelect = vi.fn();
    const { getByLabelText } = render(() => (
      <AgentList
        rows={[row({ id: 'x', name: 'reviewer' })]}
        filter="all"
        onFilter={() => {}}
        selectedId={null}
        onSelect={onSelect}
      />
    ));
    fireEvent.click(getByLabelText(/Select agent reviewer/i));
    expect(onSelect).toHaveBeenCalledWith('x');
  });
});

// ---------------------------------------------------------------------------
// Middle column: trace
// ---------------------------------------------------------------------------

describe('<AgentTrace>', () => {
  it('renders step chips tagged with their kind so styling can select on it', () => {
    const steps: AgentStep[] = [
      step({ id: 's1', kind: 'model', title: 'model pass' }),
      step({ id: 's2', kind: 'tool', title: 'tool call', status: 'done' }),
      step({ id: 's3', kind: 'spawn', title: 'spawn child', status: 'error' }),
    ];
    const { container } = render(() => (
      <AgentTrace agent={row()} steps={steps} onStepClick={() => {}} />
    ));
    const chips = container.querySelectorAll('.agent-monitor__step-chip');
    expect(chips).toHaveLength(3);
    const kinds = Array.from(chips).map((c) => c.getAttribute('data-kind'));
    expect(kinds).toEqual(['model', 'tool', 'spawn']);
  });

  it('renders a running-state dot so pulsing-ring CSS attaches', () => {
    const { container } = render(() => (
      <AgentTrace agent={row()} steps={[step({ status: 'running' })]} onStepClick={() => {}} />
    ));
    const dot = container.querySelector('.agent-monitor__step-dot--running');
    expect(dot).toBeTruthy();
  });

  it('invokes onStepClick with the clicked step', () => {
    const onStepClick = vi.fn();
    const s = step({ id: 'clickable', title: 'open me' });
    const { getByLabelText } = render(() => (
      <AgentTrace agent={row()} steps={[s]} onStepClick={onStepClick} />
    ));
    fireEvent.click(getByLabelText(/Open step open me/));
    expect(onStepClick).toHaveBeenCalledWith(s);
  });

  it('falls back to a placeholder when no agent is selected', () => {
    const { getByText } = render(() => (
      <AgentTrace agent={null} steps={[]} onStepClick={() => {}} />
    ));
    expect(getByText(/select an agent/i)).toBeTruthy();
  });
});

// ---------------------------------------------------------------------------
// Right column: inspector
// ---------------------------------------------------------------------------

describe('<AgentInspector>', () => {
  it('renders definition/tools/paths sections and a Stop button', () => {
    const data = inspector({
      source: 'a.agents/coder.md:12',
      provider: 'anthropic',
      model: 'sonnet-4.5',
      allowedTools: ['fs.read', 'shell.exec'],
      allowedPaths: ['/workspace/**'],
      resources: { cpu: 12.5, rss: 128, fds: 4 },
    });
    const { getByText, getAllByText } = render(() => (
      <AgentInspector agent={row()} data={data} onStop={() => {}} />
    ));
    expect(getByText('Definition')).toBeTruthy();
    expect(getByText('Allowed tools')).toBeTruthy();
    expect(getByText('Allowed paths')).toBeTruthy();
    expect(getByText('Resource usage')).toBeTruthy();
    expect(getByText('Stop agent')).toBeTruthy();

    // Pills rendered literally.
    expect(getByText('fs.read')).toBeTruthy();
    expect(getByText('/workspace/**')).toBeTruthy();

    // Resource pills show formatted numbers.
    expect(getAllByText(/cpu\s+12.5%/i).length).toBeGreaterThan(0);
    expect(getAllByText(/rss\s+128MB/i).length).toBeGreaterThan(0);
    expect(getAllByText(/fds\s+4/i).length).toBeGreaterThan(0);
  });

  it('renders em-dashes when resource samples are unknown', () => {
    const { getByText } = render(() => (
      <AgentInspector agent={row()} data={inspector()} onStop={() => {}} />
    ));
    expect(getByText(/cpu\s+—/)).toBeTruthy();
    expect(getByText(/rss\s+—/)).toBeTruthy();
    expect(getByText(/fds\s+—/)).toBeTruthy();
  });

  it('invokes onStop with the agent id', () => {
    const onStop = vi.fn();
    const { getByText } = render(() => (
      <AgentInspector agent={row({ id: 'kill-me' })} data={inspector()} onStop={onStop} />
    ));
    fireEvent.click(getByText('Stop agent'));
    expect(onStop).toHaveBeenCalledWith('kill-me');
  });
});

// ---------------------------------------------------------------------------
// Step-detail drawer
// ---------------------------------------------------------------------------

describe('<StepDrawer>', () => {
  it('renders the step detail when a step is open', () => {
    const { getByText } = render(() => (
      <StepDrawer step={step({ title: 'fs.read readme.txt' })} onClose={() => {}} />
    ));
    expect(getByText('fs.read readme.txt')).toBeTruthy();
  });

  it('renders nothing when step is null', () => {
    const { queryByRole } = render(() => <StepDrawer step={null} onClose={() => {}} />);
    expect(queryByRole('dialog')).toBeNull();
  });

  it('closes on Escape keypress', () => {
    const onClose = vi.fn();
    render(() => <StepDrawer step={step()} onClose={onClose} />);
    fireEvent.keyDown(document, { key: 'Escape' });
    expect(onClose).toHaveBeenCalled();
  });

  it('closes on the X button', () => {
    const onClose = vi.fn();
    const { getByLabelText } = render(() => (
      <StepDrawer step={step()} onClose={onClose} />
    ));
    fireEvent.click(getByLabelText(/Close step detail/i));
    expect(onClose).toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// stopAgentInstance wiring
//
// Pins F-140's mandatory fix: the Inspector's Stop button must invoke the
// `stop_background_agent` Tauri command (F-138's authz-gated command)
// rather than no-op. The component test exercises the helper directly so
// the wiring is provable without mounting the full route shell (which needs
// a router + live session event bus to boot).
// ---------------------------------------------------------------------------
describe('stopAgentInstance wiring', () => {
  it('invokes the stop_background_agent command with the session + instance ids', async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const result = await stopAgentInstance({ invoke }, 'sess-a', 'kill-me');
    expect(invoke).toHaveBeenCalledWith('stop_background_agent', {
      sessionId: 'sess-a',
      instanceId: 'kill-me',
    });
    expect(result).toBe('ok');
  });

  it('is a no-op when no session id is active (session-root pre-subscribe)', async () => {
    const invoke = vi.fn();
    const result = await stopAgentInstance({ invoke }, null, 'whatever');
    expect(invoke).not.toHaveBeenCalled();
    expect(result).toBe('skipped');
  });

  it('swallows invoke rejections so the Stop click stays idempotent', async () => {
    const invoke = vi.fn().mockRejectedValue(new Error('stale id'));
    const result = await stopAgentInstance({ invoke }, 'sess-a', 'kill-me');
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(result).toBe('failed');
  });
});
