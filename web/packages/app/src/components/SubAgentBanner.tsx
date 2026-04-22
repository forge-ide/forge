import { type Component, createSignal, For, Show } from 'solid-js';
import type { ChatTurn, SubAgentStatus } from '../stores/messages';
import './SubAgentBanner.css';

// ---------------------------------------------------------------------------
// SubAgentBanner — inline representation of a spawned sub-agent in the
// parent's ChatPane (F-136). Matches `docs/ui-specs/sub-agent-banner.md` §6
// Phase-2 anatomy: 2px ember left-border accent, header row
// (`↳ spawned · <name>  delegated at HH:MM  [state]  [chevron]`), click
// header to toggle collapse. Double-click navigates to the Agent Monitor
// (F-140) with the child instance pre-selected — the route 404s until F-140
// lands.
//
// Phase-3 deferred (per spec §6): model + tool-count chips require wire
// fields (`model`, `tool_count`) the orchestrator does not forward today;
// the state chip becomes an interactive popover trigger at the same time.
//
// Backend gap (flagged in the PR body): today no step events ride the
// child's `instance_id` so live step streaming is wired through props but
// stays empty in production. The banner still flips `running → done` when
// `BackgroundAgentCompleted` arrives for the child (F-137 already forwards
// that event onto the session bus).
// ---------------------------------------------------------------------------

export type SubAgentBannerTurn = Extract<ChatTurn, { type: 'sub_agent_banner' }>;

export interface SubAgentBannerProps {
  turn: SubAgentBannerTurn;
  /**
   * Child turns to render inside the banner when expanded. Empty today —
   * the wire is here for when instance-tagged step/assistant events start
   * flowing through the orchestrator's step executor (post-F-140).
   */
  children?: ChatTurn[];
  /**
   * Optional custom navigator used by tests. Production uses
   * `window.location.href = '/agent-monitor?instance=<id>'` which 404s
   * until F-140 registers the route.
   */
  onOpenInMonitor?: (childInstanceId: string) => void;
  /**
   * F-136 §6 "Max depth: 3. Beyond that, child banners render collapsed by
   * default and 'Open in new window' appears." Defaults to 0 at the root.
   */
  depth?: number;
}

const MAX_INLINE_DEPTH = 3;

function shortId(id: string): string {
  // Instance ids are UUIDs; the short 8-char prefix keeps the header
  // readable when no agent_name was enriched.
  return id.length > 8 ? id.slice(0, 8) : id;
}

function formatStartedAt(ms: number): string {
  const d = new Date(ms);
  const hh = String(d.getHours()).padStart(2, '0');
  const mm = String(d.getMinutes()).padStart(2, '0');
  return `${hh}:${mm}`;
}

function statusLabel(status: SubAgentStatus): string {
  return status;
}

export const SubAgentBanner: Component<SubAgentBannerProps> = (props) => {
  // Spec §6 "Collapsed state" — default collapsed; user expands to inspect.
  const [expanded, setExpanded] = createSignal(false);

  const depth = (): number => props.depth ?? 0;
  const beyondMaxDepth = (): boolean => depth() >= MAX_INLINE_DEPTH;

  const displayName = (): string =>
    props.turn.agent_name ?? shortId(props.turn.child_instance_id);

  const toggle = (): void => {
    setExpanded((v) => !v);
  };

  const openInMonitor = (): void => {
    if (props.onOpenInMonitor) {
      props.onOpenInMonitor(props.turn.child_instance_id);
      return;
    }
    // Route 404s until F-140 lands; flagged in the PR body.
    if (typeof window !== 'undefined') {
      window.location.href = `/agent-monitor?instance=${encodeURIComponent(props.turn.child_instance_id)}`;
    }
  };

  const summaryLine = (): string => {
    const last = props.turn.last_step_summary;
    const count = props.turn.step_count;
    if (count !== undefined && count > 0) {
      const stepWord = count === 1 ? 'step' : 'steps';
      return last ? `${count} ${stepWord} · last: ${last}` : `${count} ${stepWord}`;
    }
    // Pre-step-executor phase — nothing to summarise yet.
    return 'waiting for first step';
  };

  return (
    <section
      class="sub-agent-banner"
      data-testid={`sub-agent-banner-${props.turn.child_instance_id}`}
      data-status={props.turn.status}
      data-expanded={expanded() ? 'true' : 'false'}
      aria-label={`Sub-agent ${displayName()} (${statusLabel(props.turn.status)})`}
    >
      {/* Header — click to toggle, double-click jumps to Agent Monitor. */}
      <header
        class="sub-agent-banner__header"
        data-testid={`sub-agent-banner-header-${props.turn.child_instance_id}`}
        onClick={toggle}
        onDblClick={openInMonitor}
        role="button"
        tabIndex={0}
        onKeyDown={(e: KeyboardEvent) => {
          if (e.key === 'Enter' || e.key === ' ') {
            e.preventDefault();
            toggle();
          }
        }}
      >
        <span class="sub-agent-banner__glyph" aria-hidden="true">↳</span>
        <span class="sub-agent-banner__label">
          spawned · <span class="sub-agent-banner__name">{displayName()}</span>
        </span>
        <span
          class="sub-agent-banner__timestamp"
          data-testid={`sub-agent-banner-timestamp-${props.turn.child_instance_id}`}
        >
          delegated at {formatStartedAt(props.turn.started_at)}
        </span>
        <span
          class="sub-agent-banner__state-chip"
          data-testid={`sub-agent-banner-state-${props.turn.child_instance_id}`}
          data-state={props.turn.status}
        >
          {statusLabel(props.turn.status)}
        </span>
        <span
          class="sub-agent-banner__chevron"
          data-testid={`sub-agent-banner-chevron-${props.turn.child_instance_id}`}
          aria-hidden="true"
        >
          {expanded() ? 'v' : '>'}
        </span>
      </header>

      {/* Collapsed body — one-line summary per spec §6 "Collapsed state". */}
      <Show when={!expanded()}>
        <div
          class="sub-agent-banner__summary"
          data-testid={`sub-agent-banner-summary-${props.turn.child_instance_id}`}
        >
          {summaryLine()}
        </div>
      </Show>

      {/* Expanded body — nested turn render. Beyond MAX_INLINE_DEPTH we
          render the "Open in new window" affordance instead of inlining
          grandchildren, per spec §6 "Max depth: 3". */}
      <Show when={expanded()}>
        <div
          class="sub-agent-banner__body"
          data-testid={`sub-agent-banner-body-${props.turn.child_instance_id}`}
        >
          <Show
            when={!beyondMaxDepth()}
            fallback={
              <button
                type="button"
                class="sub-agent-banner__open-monitor"
                data-testid={`sub-agent-banner-open-monitor-${props.turn.child_instance_id}`}
                onClick={openInMonitor}
              >
                Open in new window
              </button>
            }
          >
            <Show
              when={(props.children ?? []).length > 0}
              fallback={
                <p
                  class="sub-agent-banner__empty"
                  data-testid={`sub-agent-banner-empty-${props.turn.child_instance_id}`}
                >
                  No step events yet.
                </p>
              }
            >
              <For each={props.children ?? []}>
                {(child) => <NestedTurnRenderer turn={child} depth={depth() + 1} />}
              </For>
            </Show>
          </Show>
        </div>
      </Show>
    </section>
  );
};

// ---------------------------------------------------------------------------
// NestedTurnRenderer — renders a nested sub-agent banner when a child turn
// of type `sub_agent_banner` shows up inside an expanded banner body. Other
// turn types are intentionally excluded here: the parent ChatPane owns the
// cross-turn switch (user/assistant/tool/error), so nested banners only
// need to know how to recurse into themselves.
// ---------------------------------------------------------------------------

const NestedTurnRenderer: Component<{ turn: ChatTurn; depth: number }> = (props) => {
  return (
    <Show when={props.turn.type === 'sub_agent_banner'}>
      <SubAgentBanner
        turn={props.turn as SubAgentBannerTurn}
        depth={props.depth}
      />
    </Show>
  );
};
