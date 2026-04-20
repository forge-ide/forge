import { describe, it, expect, vi } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import { SubAgentBanner } from './SubAgentBanner';
import type { ChatTurn } from '../stores/messages';

// F-136 component tests — the DoD pins expand/collapse, nested rendering for
// sub-of-sub banners, and completion state. Tests pass a `turn` fixture +
// optional `children`/`onOpenInMonitor` overrides; no store wiring required.

function makeTurn(
  overrides: Partial<Extract<ChatTurn, { type: 'sub_agent_banner' }>> = {},
): Extract<ChatTurn, { type: 'sub_agent_banner' }> {
  return {
    type: 'sub_agent_banner',
    child_instance_id: 'child-1',
    parent_instance_id: 'parent-1',
    status: 'running',
    started_at: Date.UTC(2026, 3, 20, 14, 37, 0),
    ...overrides,
  };
}

describe('SubAgentBanner — header + default state', () => {
  it('renders the spawned glyph and agent name in the header', () => {
    const { getByTestId } = render(() => (
      <SubAgentBanner turn={makeTurn({ agent_name: 'test-writer' })} />
    ));
    const header = getByTestId('sub-agent-banner-header-child-1');
    expect(header).toHaveTextContent('spawned');
    expect(header).toHaveTextContent('test-writer');
  });

  it('falls back to a short prefix of the child id when no agent_name is set', () => {
    const { getByTestId } = render(() => (
      <SubAgentBanner turn={makeTurn({ child_instance_id: 'abcd1234-efgh-5678' })} />
    ));
    const header = getByTestId('sub-agent-banner-header-abcd1234-efgh-5678');
    // Short prefix (first 8 chars) is visible; the full id is not.
    expect(header).toHaveTextContent('abcd1234');
    expect(header).not.toHaveTextContent('efgh-5678');
  });

  it('renders a "delegated at HH:MM" timestamp from started_at', () => {
    // Build a fixture whose local-time HH:MM we can predict independent
    // of the runner's TZ by reading the same Date back.
    const at = new Date();
    at.setHours(9, 5, 0, 0);
    const { getByTestId } = render(() => (
      <SubAgentBanner turn={makeTurn({ started_at: at.getTime() })} />
    ));
    const ts = getByTestId('sub-agent-banner-timestamp-child-1');
    expect(ts).toHaveTextContent(/delegated at \d{2}:\d{2}/);
  });

  it('mounts collapsed by default — summary visible, body hidden', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <SubAgentBanner turn={makeTurn()} />
    ));
    expect(getByTestId('sub-agent-banner-child-1')).toHaveAttribute(
      'data-expanded',
      'false',
    );
    expect(getByTestId('sub-agent-banner-summary-child-1')).toBeInTheDocument();
    expect(queryByTestId('sub-agent-banner-body-child-1')).not.toBeInTheDocument();
  });
});

describe('SubAgentBanner — expand/collapse', () => {
  it('toggles to expanded on header click and back to collapsed on second click', () => {
    const { getByTestId, queryByTestId } = render(() => (
      <SubAgentBanner turn={makeTurn()} />
    ));
    const header = getByTestId('sub-agent-banner-header-child-1');

    fireEvent.click(header);
    expect(getByTestId('sub-agent-banner-child-1')).toHaveAttribute(
      'data-expanded',
      'true',
    );
    expect(getByTestId('sub-agent-banner-body-child-1')).toBeInTheDocument();
    expect(queryByTestId('sub-agent-banner-summary-child-1')).not.toBeInTheDocument();

    fireEvent.click(header);
    expect(getByTestId('sub-agent-banner-child-1')).toHaveAttribute(
      'data-expanded',
      'false',
    );
  });

  it('expands when Enter is pressed on the focused header', () => {
    const { getByTestId } = render(() => <SubAgentBanner turn={makeTurn()} />);
    const header = getByTestId('sub-agent-banner-header-child-1');
    fireEvent.keyDown(header, { key: 'Enter' });
    expect(getByTestId('sub-agent-banner-child-1')).toHaveAttribute(
      'data-expanded',
      'true',
    );
  });

  it('expands when Space is pressed on the focused header', () => {
    const { getByTestId } = render(() => <SubAgentBanner turn={makeTurn()} />);
    const header = getByTestId('sub-agent-banner-header-child-1');
    fireEvent.keyDown(header, { key: ' ' });
    expect(getByTestId('sub-agent-banner-child-1')).toHaveAttribute(
      'data-expanded',
      'true',
    );
  });

  it('renders the "No step events yet" placeholder when expanded with no children', () => {
    const { getByTestId } = render(() => <SubAgentBanner turn={makeTurn()} />);
    fireEvent.click(getByTestId('sub-agent-banner-header-child-1'));
    expect(getByTestId('sub-agent-banner-empty-child-1')).toBeInTheDocument();
  });
});

describe('SubAgentBanner — nested rendering for sub-of-sub', () => {
  it('renders a nested SubAgentBanner when an expanded body carries a sub_agent_banner child turn', () => {
    const grandchild: Extract<ChatTurn, { type: 'sub_agent_banner' }> = {
      type: 'sub_agent_banner',
      child_instance_id: 'grandchild-1',
      parent_instance_id: 'child-1',
      status: 'running',
      started_at: Date.now(),
      agent_name: 'deep-helper',
    };
    const { getByTestId } = render(() => (
      <SubAgentBanner turn={makeTurn()} children={[grandchild]} />
    ));
    fireEvent.click(getByTestId('sub-agent-banner-header-child-1'));

    // Nested banner is rendered inside the parent body.
    expect(getByTestId('sub-agent-banner-grandchild-1')).toBeInTheDocument();
    const nestedHeader = getByTestId('sub-agent-banner-header-grandchild-1');
    expect(nestedHeader).toHaveTextContent('deep-helper');
  });

  it('replaces nested inline children with an "Open in new window" button past max depth (3)', () => {
    const grandchild: Extract<ChatTurn, { type: 'sub_agent_banner' }> = {
      type: 'sub_agent_banner',
      child_instance_id: 'too-deep-1',
      parent_instance_id: 'child-1',
      status: 'running',
      started_at: Date.now(),
    };
    const { getByTestId, queryByTestId } = render(() => (
      <SubAgentBanner
        turn={makeTurn()}
        children={[grandchild]}
        depth={3}
      />
    ));
    fireEvent.click(getByTestId('sub-agent-banner-header-child-1'));
    // At depth ≥ 3 the body renders the open-monitor fallback, not the
    // nested banner — its testid must be absent.
    expect(queryByTestId('sub-agent-banner-too-deep-1')).not.toBeInTheDocument();
    expect(getByTestId('sub-agent-banner-open-monitor-child-1')).toBeInTheDocument();
  });
});

describe('SubAgentBanner — completion state', () => {
  it('renders the state chip with data-state="running" while the child is live', () => {
    const { getByTestId, container } = render(() => (
      <SubAgentBanner turn={makeTurn({ status: 'running' })} />
    ));
    const chip = getByTestId('sub-agent-banner-state-child-1');
    expect(chip).toHaveAttribute('data-state', 'running');
    expect(chip).toHaveTextContent('running');
    // The outer section also mirrors the status via data-status so CSS can
    // target completed vs. running styling without prop plumbing.
    const section = container.querySelector('.sub-agent-banner');
    expect(section).toHaveAttribute('data-status', 'running');
  });

  it('renders the state chip with data-state="done" on completion', () => {
    const { getByTestId, container } = render(() => (
      <SubAgentBanner turn={makeTurn({ status: 'done' })} />
    ));
    const chip = getByTestId('sub-agent-banner-state-child-1');
    expect(chip).toHaveAttribute('data-state', 'done');
    expect(chip).toHaveTextContent('done');
    const section = container.querySelector('.sub-agent-banner');
    expect(section).toHaveAttribute('data-status', 'done');
  });

  it('summary line shows step count + last step when provided', () => {
    const { getByTestId } = render(() => (
      <SubAgentBanner
        turn={makeTurn({ step_count: 6, last_step_summary: 'wrote validate.test.ts' })}
      />
    ));
    const summary = getByTestId('sub-agent-banner-summary-child-1');
    expect(summary).toHaveTextContent('6 steps');
    expect(summary).toHaveTextContent('wrote validate.test.ts');
  });

  it('summary line falls back to "waiting for first step" before any step event', () => {
    const { getByTestId } = render(() => <SubAgentBanner turn={makeTurn()} />);
    expect(getByTestId('sub-agent-banner-summary-child-1')).toHaveTextContent(
      'waiting for first step',
    );
  });
});

describe('SubAgentBanner — double-click to Agent Monitor (F-140)', () => {
  it('calls onOpenInMonitor with the child instance id on header double-click', () => {
    const open = vi.fn();
    const { getByTestId } = render(() => (
      <SubAgentBanner turn={makeTurn()} onOpenInMonitor={open} />
    ));
    fireEvent.dblClick(getByTestId('sub-agent-banner-header-child-1'));
    expect(open).toHaveBeenCalledWith('child-1');
  });

  it('invokes the "Open in new window" button when depth exceeds the inline cap', () => {
    const open = vi.fn();
    const grandchild: Extract<ChatTurn, { type: 'sub_agent_banner' }> = {
      type: 'sub_agent_banner',
      child_instance_id: 'too-deep-2',
      parent_instance_id: 'child-1',
      status: 'running',
      started_at: Date.now(),
    };
    const { getByTestId } = render(() => (
      <SubAgentBanner
        turn={makeTurn()}
        children={[grandchild]}
        depth={3}
        onOpenInMonitor={open}
      />
    ));
    fireEvent.click(getByTestId('sub-agent-banner-header-child-1'));
    fireEvent.click(getByTestId('sub-agent-banner-open-monitor-child-1'));
    expect(open).toHaveBeenCalledWith('child-1');
  });
});
