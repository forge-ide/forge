import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import type { LayoutTree } from '@forge/ipc';
import { GridContainer, type LayoutLeaf } from './GridContainer';

afterEach(() => {
  cleanup();
});

// F-150: GridContainer now consumes the persisted `LayoutTree` shape. Tests
// drive it with a `renderLeaf` prop that dispatches on `pane_type` the same
// way SessionWindow does in production.
function renderByPaneType(leaf: LayoutLeaf) {
  return <div data-testid={`leaf-${leaf.id}`}>{leaf.pane_type}-{leaf.id}</div>;
}

describe('GridContainer — leaf rendering', () => {
  it('renders a single leaf tree by calling renderLeaf', () => {
    const tree: LayoutTree = { kind: 'leaf', id: 'chat-1', pane_type: 'chat' };
    const { getByTestId } = render(() => (
      <GridContainer
        tree={tree}
        renderLeaf={renderByPaneType}
        onRatioChange={vi.fn()}
      />
    ));
    expect(getByTestId('leaf-chat-1').textContent).toBe('chat-chat-1');
    expect(getByTestId('grid-leaf-chat-1')).toBeTruthy();
  });
});

describe('GridContainer — recursive split tree', () => {
  it('renders nested H/V splits with the correct leaves', () => {
    // Shape: v-split [ chat | h-split [ editor / terminal ] ]
    const tree: LayoutTree = {
      kind: 'split',
      id: 'root',
      direction: 'v',
      ratio: 0.5,
      a: { kind: 'leaf', id: 'chat', pane_type: 'chat' },
      b: {
        kind: 'split',
        id: 'right',
        direction: 'h',
        ratio: 0.6,
        a: { kind: 'leaf', id: 'editor', pane_type: 'editor' },
        b: { kind: 'leaf', id: 'terminal', pane_type: 'terminal' },
      },
    };

    const { getAllByTestId, getByTestId } = render(() => (
      <GridContainer
        tree={tree}
        renderLeaf={renderByPaneType}
        onRatioChange={vi.fn()}
      />
    ));

    expect(getByTestId('leaf-chat')).toBeTruthy();
    expect(getByTestId('leaf-editor')).toBeTruthy();
    expect(getByTestId('leaf-terminal')).toBeTruthy();

    const splits = getAllByTestId('split-pane');
    // Two SplitPane nodes: the root v-split and the nested h-split.
    expect(splits.length).toBe(2);
    const [rootSplit, innerSplit] = splits as [HTMLElement, HTMLElement];
    expect(rootSplit.getAttribute('data-direction')).toBe('v');
    expect(innerSplit.getAttribute('data-direction')).toBe('h');
  });

  it('propagates onRatioChange with the originating split node id', () => {
    const onRatioChange = vi.fn();
    const tree: LayoutTree = {
      kind: 'split',
      id: 'root',
      direction: 'v',
      ratio: 0.5,
      a: {
        kind: 'split',
        id: 'left',
        direction: 'h',
        ratio: 0.5,
        a: { kind: 'leaf', id: 'a1', pane_type: 'chat' },
        b: { kind: 'leaf', id: 'a2', pane_type: 'chat' },
      },
      b: { kind: 'leaf', id: 'b1', pane_type: 'chat' },
    };

    const { getAllByTestId } = render(() => (
      <GridContainer
        tree={tree}
        renderLeaf={renderByPaneType}
        onRatioChange={onRatioChange}
      />
    ));

    const splits = getAllByTestId('split-pane') as HTMLElement[];
    // Double-click the inner (h) split divider → 'left' should be the id.
    const innerSplit = splits[1] as HTMLElement;
    const innerDivider = innerSplit.querySelector(
      '[data-testid="split-pane-divider"]',
    ) as HTMLElement;
    innerDivider.dispatchEvent(
      new MouseEvent('dblclick', { bubbles: true, cancelable: true }),
    );

    expect(onRatioChange).toHaveBeenCalledWith('left', 0.5);
  });
});
