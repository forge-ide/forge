import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import type { LayoutNode } from './GridContainer';
import { GridContainer } from './GridContainer';
import { DropZoneOverlay } from './DropZoneOverlay';

afterEach(() => {
  cleanup();
});

describe('DropZoneOverlay — standalone', () => {
  it('renders all five zones, none active when `activeZone` is null', () => {
    const { getByTestId } = render(() => <DropZoneOverlay activeZone={null} />);
    for (const zone of ['top', 'bottom', 'left', 'right', 'center'] as const) {
      const el = getByTestId(`drop-zone-${zone}`);
      expect(el).toBeTruthy();
      expect(el.getAttribute('data-active')).toBe('false');
    }
  });

  it('marks only the hovered zone as active', () => {
    const { getByTestId } = render(() => <DropZoneOverlay activeZone="right" />);
    expect(getByTestId('drop-zone-right').getAttribute('data-active')).toBe('true');
    expect(getByTestId('drop-zone-left').getAttribute('data-active')).toBe('false');
    expect(getByTestId('drop-zone-center').getAttribute('data-active')).toBe('false');
  });
});

describe('GridContainer — drag overlay threading', () => {
  const tree: LayoutNode = {
    kind: 'split',
    id: 'root',
    direction: 'v',
    ratio: 0.5,
    a: { kind: 'leaf', id: 'a', render: () => <div data-testid="a">a</div> },
    b: { kind: 'leaf', id: 'b', render: () => <div data-testid="b">b</div> },
  };

  it('renders no overlay when dragState is null', () => {
    const { queryByTestId } = render(() => (
      <GridContainer tree={tree} onRatioChange={() => {}} dragState={null} />
    ));
    expect(queryByTestId('drop-zone-overlay')).toBeNull();
  });

  it('paints overlays on leaves during a drag and marks the target zone', () => {
    render(() => (
      <GridContainer
        tree={tree}
        onRatioChange={() => {}}
        dragState={{ sourceId: 'a', targetId: 'b', zone: 'right' }}
      />
    ));
    // Overlays appear on each leaf during an active drag.
    const overlays = document.querySelectorAll('[data-testid="drop-zone-overlay"]');
    expect(overlays.length).toBe(2);
    // Exactly one zone is active — on leaf `b`, the `right` zone.
    const active = document.querySelectorAll('[data-active="true"]');
    expect(active.length).toBe(1);
    expect(active[0]?.getAttribute('data-zone')).toBe('right');
    const activeLeaf = active[0]?.closest('[data-leaf-id]');
    expect(activeLeaf?.getAttribute('data-leaf-id')).toBe('b');
  });
});
