import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import type { LayoutTree } from '@forge/ipc';
import { GridContainer, type LayoutLeaf } from './GridContainer';
import { DropZoneOverlay } from './DropZoneOverlay';

afterEach(() => {
  cleanup();
});

function renderByPaneType(leaf: LayoutLeaf) {
  return <div data-testid={leaf.id}>{leaf.id}</div>;
}

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
  const tree: LayoutTree = {
    kind: 'split',
    id: 'root',
    direction: 'v',
    ratio: 0.5,
    a: { kind: 'leaf', id: 'a', pane_type: 'chat' },
    b: { kind: 'leaf', id: 'b', pane_type: 'chat' },
  };

  it('renders no overlay when dragState is null', () => {
    const { queryByTestId } = render(() => (
      <GridContainer
        tree={tree}
        renderLeaf={renderByPaneType}
        onRatioChange={() => {}}
        dragState={null}
      />
    ));
    expect(queryByTestId('drop-zone-overlay')).toBeNull();
  });

  it('paints the overlay only on the targeted leaf and marks the active zone', () => {
    // F-573: previously the overlay mounted on every leaf during an
    // active drag (O(N) DOM churn on drag-start/end). The overlay is now
    // gated on `targetId === leaf.id`, so exactly one subtree mounts —
    // the one whose leaf is under the pointer. Inactive zones on the
    // overlay are `background: transparent` per drop-zone-overlay.css,
    // so the user-visible result is identical: only the active zone
    // renders the §3.6 ember tint.
    render(() => (
      <GridContainer
        tree={tree}
        renderLeaf={renderByPaneType}
        onRatioChange={() => {}}
        dragState={{ sourceId: 'a', targetId: 'b', zone: 'right' }}
      />
    ));
    const overlays = document.querySelectorAll('[data-testid="drop-zone-overlay"]');
    expect(overlays.length).toBe(1);
    const overlayLeaf = overlays[0]?.closest('[data-leaf-id]');
    expect(overlayLeaf?.getAttribute('data-leaf-id')).toBe('b');
    // Exactly one zone is active — on leaf `b`, the `right` zone.
    const active = document.querySelectorAll('[data-active="true"]');
    expect(active.length).toBe(1);
    expect(active[0]?.getAttribute('data-zone')).toBe('right');
  });
});
