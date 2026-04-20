import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import { GridContainer, type LayoutNode } from './GridContainer';

afterEach(() => {
  cleanup();
});

describe('GridContainer — leaf rendering', () => {
  it('renders a single leaf tree by calling its render callback', () => {
    const tree: LayoutNode = {
      kind: 'leaf',
      id: 'chat-1',
      render: () => <div data-testid="leaf-content">hello</div>,
    };
    const { getByTestId } = render(() => (
      <GridContainer tree={tree} onRatioChange={vi.fn()} />
    ));
    expect(getByTestId('leaf-content').textContent).toBe('hello');
    expect(getByTestId('grid-leaf-chat-1')).toBeTruthy();
  });
});

describe('GridContainer — recursive split tree', () => {
  it('renders nested H/V splits with the correct leaves', () => {
    // Shape: v-split [ chat | h-split [ editor / terminal ] ]
    const tree: LayoutNode = {
      kind: 'split',
      id: 'root',
      direction: 'v',
      ratio: 0.5,
      a: {
        kind: 'leaf',
        id: 'chat',
        render: () => <div data-testid="chat">chat</div>,
      },
      b: {
        kind: 'split',
        id: 'right',
        direction: 'h',
        ratio: 0.6,
        a: {
          kind: 'leaf',
          id: 'editor',
          render: () => <div data-testid="editor">editor</div>,
        },
        b: {
          kind: 'leaf',
          id: 'terminal',
          render: () => <div data-testid="terminal">terminal</div>,
        },
      },
    };

    const { getAllByTestId, getByTestId } = render(() => (
      <GridContainer tree={tree} onRatioChange={vi.fn()} />
    ));

    expect(getByTestId('chat')).toBeTruthy();
    expect(getByTestId('editor')).toBeTruthy();
    expect(getByTestId('terminal')).toBeTruthy();

    const splits = getAllByTestId('split-pane');
    // Two SplitPane nodes: the root v-split and the nested h-split.
    expect(splits.length).toBe(2);
    const [rootSplit, innerSplit] = splits as [HTMLElement, HTMLElement];
    expect(rootSplit.getAttribute('data-direction')).toBe('v');
    expect(innerSplit.getAttribute('data-direction')).toBe('h');
  });

  it('propagates onRatioChange with the originating split node id', () => {
    const onRatioChange = vi.fn();
    const tree: LayoutNode = {
      kind: 'split',
      id: 'root',
      direction: 'v',
      ratio: 0.5,
      a: {
        kind: 'split',
        id: 'left',
        direction: 'h',
        ratio: 0.5,
        a: { kind: 'leaf', id: 'a1', render: () => <div>a1</div> },
        b: { kind: 'leaf', id: 'a2', render: () => <div>a2</div> },
      },
      b: { kind: 'leaf', id: 'b1', render: () => <div>b1</div> },
    };

    const { getAllByTestId } = render(() => (
      <GridContainer tree={tree} onRatioChange={onRatioChange} />
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
