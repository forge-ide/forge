import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import type { LayoutNode } from './GridContainer';
import { applyDockDrop, zoneForPoint } from './dockDrop';
import { useDragToDock } from './useDragToDock';

// -----------------------------------------------------------------------------
// applyDockDrop — pure tree mutation
// -----------------------------------------------------------------------------

describe('applyDockDrop — edge zones', () => {
  const makeTree = (): LayoutNode => ({
    kind: 'split',
    id: 'root',
    direction: 'v',
    ratio: 0.5,
    a: { kind: 'leaf', id: 'chat', render: () => null },
    b: {
      kind: 'split',
      id: 'right',
      direction: 'h',
      ratio: 0.5,
      a: { kind: 'leaf', id: 'editor', render: () => null },
      b: { kind: 'leaf', id: 'terminal', render: () => null },
    },
  });

  it('right zone: target=a, source=b in a v-split', () => {
    const next = applyDockDrop(makeTree(), 'chat', 'editor', 'right');
    if (next.kind !== 'split') throw new Error('expected split');
    // root collapsed to right subtree after removing 'chat'; 'editor' became
    // a v-split (editor | chat).
    expect(next.direction).toBe('h');
    if (next.a.kind !== 'split') throw new Error('expected split');
    expect(next.a.direction).toBe('v');
    expect((next.a.a as { id: string }).id).toBe('editor');
    expect((next.a.b as { id: string }).id).toBe('chat');
  });

  it('left zone: source=a, target=b in a v-split', () => {
    const next = applyDockDrop(makeTree(), 'chat', 'terminal', 'left');
    if (next.kind !== 'split') throw new Error('expected split');
    const inner = next.b;
    if (inner.kind !== 'split') throw new Error('expected split');
    expect(inner.direction).toBe('v');
    expect((inner.a as { id: string }).id).toBe('chat');
    expect((inner.b as { id: string }).id).toBe('terminal');
  });

  it('bottom zone: target=a, source=b in an h-split', () => {
    const next = applyDockDrop(makeTree(), 'chat', 'editor', 'bottom');
    if (next.kind !== 'split') throw new Error('expected split');
    const inner = next.a;
    if (inner.kind !== 'split') throw new Error('expected split');
    expect(inner.direction).toBe('h');
    expect((inner.a as { id: string }).id).toBe('editor');
    expect((inner.b as { id: string }).id).toBe('chat');
  });

  it('top zone: source=a, target=b in an h-split', () => {
    const next = applyDockDrop(makeTree(), 'chat', 'terminal', 'top');
    if (next.kind !== 'split') throw new Error('expected split');
    const inner = next.b;
    if (inner.kind !== 'split') throw new Error('expected split');
    expect(inner.direction).toBe('h');
    expect((inner.a as { id: string }).id).toBe('chat');
    expect((inner.b as { id: string }).id).toBe('terminal');
  });
});

describe('applyDockDrop — center zone', () => {
  it('replaces the target leaf with the source leaf', () => {
    const tree: LayoutNode = {
      kind: 'split',
      id: 'root',
      direction: 'v',
      ratio: 0.5,
      a: { kind: 'leaf', id: 'a', render: () => null },
      b: {
        kind: 'split',
        id: 'inner',
        direction: 'h',
        ratio: 0.5,
        a: { kind: 'leaf', id: 'b', render: () => null },
        b: { kind: 'leaf', id: 'c', render: () => null },
      },
    };
    const next = applyDockDrop(tree, 'a', 'c', 'center');
    if (next.kind !== 'split') throw new Error('expected split');
    expect(next.id).toBe('inner');
    expect((next.a as { id: string }).id).toBe('b');
    expect((next.b as { id: string }).id).toBe('a');
  });
});

describe('applyDockDrop — malformed drops are no-ops', () => {
  const baseTree: LayoutNode = {
    kind: 'split',
    id: 'root',
    direction: 'v',
    ratio: 0.5,
    a: { kind: 'leaf', id: 'a', render: () => null },
    b: { kind: 'leaf', id: 'b', render: () => null },
  };

  it('returns tree unchanged when sourceId === targetId', () => {
    expect(applyDockDrop(baseTree, 'a', 'a', 'right')).toBe(baseTree);
  });

  it('returns tree unchanged when source leaf is missing', () => {
    expect(applyDockDrop(baseTree, 'missing', 'a', 'right')).toBe(baseTree);
  });

  it('returns tree unchanged when target leaf is missing', () => {
    expect(applyDockDrop(baseTree, 'a', 'missing', 'right')).toBe(baseTree);
  });

  it('refuses to remove the sole leaf of a root-leaf tree', () => {
    const lonely: LayoutNode = { kind: 'leaf', id: 'only', render: () => null };
    expect(applyDockDrop(lonely, 'only', 'only', 'center')).toBe(lonely);
  });
});

// -----------------------------------------------------------------------------
// zoneForPoint — pointer-to-zone classification
// -----------------------------------------------------------------------------

describe('zoneForPoint', () => {
  const rect = { left: 0, top: 0, right: 1000, bottom: 600, width: 1000, height: 600 };

  it('returns null when the pointer is outside the rect', () => {
    expect(zoneForPoint(-10, 100, rect)).toBeNull();
    expect(zoneForPoint(100, 900, rect)).toBeNull();
  });

  it('classifies the center', () => {
    expect(zoneForPoint(500, 300, rect)).toBe('center');
  });

  it('classifies each edge within 25% of its side', () => {
    expect(zoneForPoint(100, 300, rect)).toBe('left');
    expect(zoneForPoint(900, 300, rect)).toBe('right');
    expect(zoneForPoint(500, 50, rect)).toBe('top');
    expect(zoneForPoint(500, 550, rect)).toBe('bottom');
  });
});

// -----------------------------------------------------------------------------
// useDragToDock — pointer sequence → tree mutation / abort
//
// The hook is rendered via `createRoot` (no JSX) so this file keeps the exact
// name the F-118 DoD asks for (`useDragToDock.test.ts`). Leaf markers are
// placed directly on the DOM so `document.elementFromPoint` + the
// `[data-leaf-id]` ancestor lookup resolves the target the same way it does
// in the GridContainer-rendered app.
// -----------------------------------------------------------------------------

interface LeafGeometry {
  [id: string]: { left: number; top: number; right: number; bottom: number; width: number; height: number };
}

function installStubs(geometry: LeafGeometry): () => void {
  const originalEfp = document.elementFromPoint;
  const originalRect = Element.prototype.getBoundingClientRect;

  Element.prototype.getBoundingClientRect = function stubbed(this: Element): DOMRect {
    if (this instanceof HTMLElement) {
      const id = this.getAttribute('data-leaf-id');
      if (id !== null && geometry[id] !== undefined) {
        const g = geometry[id];
        return {
          ...g,
          x: g.left,
          y: g.top,
          toJSON() {
            return g;
          },
        } as DOMRect;
      }
    }
    return originalRect.call(this);
  };

  document.elementFromPoint = function stubbed(x: number, y: number): Element | null {
    for (const [id, g] of Object.entries(geometry)) {
      if (x >= g.left && x <= g.right && y >= g.top && y <= g.bottom) {
        const el = document.querySelector(`[data-leaf-id="${id}"]`);
        if (el !== null) return el;
      }
    }
    return null;
  };

  return () => {
    Element.prototype.getBoundingClientRect = originalRect;
    document.elementFromPoint = originalEfp;
  };
}

function makePointerDown(clientX: number, clientY: number): PointerEvent {
  const ev = new MouseEvent('pointerdown', {
    bubbles: true,
    cancelable: true,
    clientX,
    clientY,
    button: 0,
  });
  Object.defineProperty(ev, 'pointerId', { value: 1 });
  return ev as unknown as PointerEvent;
}

function fireWindowPointer(type: 'pointermove' | 'pointerup', clientX: number, clientY: number): void {
  const ev = new MouseEvent(type, { bubbles: true, cancelable: true, clientX, clientY });
  Object.defineProperty(ev, 'pointerId', { value: 1 });
  window.dispatchEvent(ev);
}

interface Harness {
  tree: () => LayoutNode;
  api: ReturnType<typeof useDragToDock>;
  dispose: () => void;
}

function makeHarness(initial: LayoutNode, leafIds: string[]): Harness {
  // Render the leaf marker DOM by hand. The hook reads leaves via
  // `document.elementFromPoint(...).closest('[data-leaf-id]')`, so the
  // `data-leaf-id` attribute is all that matters for hit-testing.
  for (const id of leafIds) {
    const div = document.createElement('div');
    div.setAttribute('data-leaf-id', id);
    div.setAttribute('data-testid', `leaf-${id}`);
    document.body.appendChild(div);
  }
  let api!: ReturnType<typeof useDragToDock>;
  let tree!: () => LayoutNode;
  const dispose = createRoot((d) => {
    const [t, setT] = createSignal(initial);
    tree = t;
    api = useDragToDock({
      getTree: () => t(),
      onTreeChange: (next) => setT(next),
    });
    return d;
  });
  return {
    tree,
    api,
    dispose: () => {
      dispose();
      for (const id of leafIds) {
        document.querySelector(`[data-leaf-id="${id}"]`)?.remove();
      }
    },
  };
}

describe('useDragToDock — integration', () => {
  // Baseline two-leaf tree `[a | b]` with `b` in the right half of the screen.
  const baseTree: LayoutNode = {
    kind: 'split',
    id: 'root',
    direction: 'v',
    ratio: 0.5,
    a: { kind: 'leaf', id: 'a', render: () => null },
    b: { kind: 'leaf', id: 'b', render: () => null },
  };
  const geometry: LeafGeometry = {
    a: { left: 0, top: 0, right: 500, bottom: 600, width: 500, height: 600 },
    b: { left: 500, top: 0, right: 1000, bottom: 600, width: 500, height: 600 },
  };

  let restore: (() => void) | null = null;
  let harness: Harness | null = null;

  beforeEach(() => {
    restore = installStubs(geometry);
    harness = makeHarness(baseTree, ['a', 'b']);
  });

  afterEach(() => {
    harness?.dispose();
    harness = null;
    restore?.();
    restore = null;
  });

  it('drop on right edge: v-split with b on the left, a on the right', () => {
    const h = harness!;
    // `startDrag` returns the header handler; invoke it directly with a
    // synthesized pointerdown.
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 950, 300);
    fireWindowPointer('pointerup', 950, 300);

    const next = h.tree();
    if (next.kind !== 'split') throw new Error('expected split');
    expect(next.direction).toBe('v');
    expect((next.a as { id: string }).id).toBe('b');
    expect((next.b as { id: string }).id).toBe('a');
  });

  it('drop on left edge: v-split with a on the left, b on the right', () => {
    const h = harness!;
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 520, 300);
    fireWindowPointer('pointerup', 520, 300);

    const next = h.tree();
    if (next.kind !== 'split') throw new Error('expected split');
    expect(next.direction).toBe('v');
    expect((next.a as { id: string }).id).toBe('a');
    expect((next.b as { id: string }).id).toBe('b');
  });

  it('drop on top edge: h-split with a on top, b below', () => {
    const h = harness!;
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 700, 50);
    fireWindowPointer('pointerup', 700, 50);

    const next = h.tree();
    if (next.kind !== 'split') throw new Error('expected split');
    expect(next.direction).toBe('h');
    expect((next.a as { id: string }).id).toBe('a');
    expect((next.b as { id: string }).id).toBe('b');
  });

  it('drop on bottom edge: h-split with b on top, a below', () => {
    const h = harness!;
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 700, 550);
    fireWindowPointer('pointerup', 700, 550);

    const next = h.tree();
    if (next.kind !== 'split') throw new Error('expected split');
    expect(next.direction).toBe('h');
    expect((next.a as { id: string }).id).toBe('b');
    expect((next.b as { id: string }).id).toBe('a');
  });

  it('drop on center: target replaced by source (flatten to single leaf a)', () => {
    const h = harness!;
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 750, 300);
    fireWindowPointer('pointerup', 750, 300);

    const next = h.tree();
    expect(next.kind).toBe('leaf');
    if (next.kind !== 'leaf') throw new Error('expected leaf');
    expect(next.id).toBe('a');
  });

  it('aborts on Escape and leaves the tree untouched', () => {
    const h = harness!;
    const before = h.tree();
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 750, 300);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    fireWindowPointer('pointerup', 750, 300);

    expect(h.tree()).toBe(before);
    expect(h.api.drag()).toBeNull();
  });

  it('aborts on drop outside any leaf', () => {
    const h = harness!;
    const before = h.tree();
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 5000, 5000);
    fireWindowPointer('pointerup', 5000, 5000);

    expect(h.tree()).toBe(before);
    expect(h.api.drag()).toBeNull();
  });

  it('treats hovering over the source leaf as no target (drop is a no-op)', () => {
    const h = harness!;
    const before = h.tree();
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 250, 300); // inside leaf a itself
    expect(h.api.drag()?.targetId).toBeNull();
    fireWindowPointer('pointerup', 250, 300);

    expect(h.tree()).toBe(before);
  });

  it('exposes (sourceId, targetId, zone) on the drag signal while in flight', () => {
    const h = harness!;
    h.api.startDrag('a')(makePointerDown(10, 10));
    fireWindowPointer('pointermove', 950, 300);

    const state = h.api.drag();
    expect(state?.sourceId).toBe('a');
    expect(state?.targetId).toBe('b');
    expect(state?.zone).toBe('right');

    // Clean tear-down.
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
  });

  it('ignores a pointerdown while a drag is already in flight', () => {
    const h = harness!;
    h.api.startDrag('a')(makePointerDown(10, 10));
    const first = h.api.drag();
    // Second startDrag must not overwrite source.
    h.api.startDrag('b')(makePointerDown(10, 10));
    expect(h.api.drag()?.sourceId).toBe(first?.sourceId);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
  });
});
