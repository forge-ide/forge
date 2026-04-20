import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createRoot, createSignal } from 'solid-js';
import { usePaneWidth, type Compactness } from './usePaneWidth';

// -----------------------------------------------------------------------------
// ResizeObserver mock — jsdom doesn't ship ResizeObserver. Capture the last
// instance's callback so tests can drive synthetic entries through it.
// -----------------------------------------------------------------------------

type ROCallback = (entries: ResizeObserverEntry[]) => void;

interface MockROHandle {
  callback: ROCallback;
  observed: Element[];
  disconnected: boolean;
}

let lastObserver: MockROHandle | null = null;

class MockResizeObserver implements ResizeObserver {
  private readonly handle: MockROHandle;
  constructor(cb: ROCallback) {
    this.handle = { callback: cb, observed: [], disconnected: false };
    lastObserver = this.handle;
  }
  observe(target: Element): void {
    this.handle.observed.push(target);
  }
  unobserve(target: Element): void {
    this.handle.observed = this.handle.observed.filter((e) => e !== target);
  }
  disconnect(): void {
    this.handle.disconnected = true;
    this.handle.observed = [];
  }
}

/**
 * Synthesize the tiny slice of ResizeObserverEntry the hook reads — just
 * `contentRect.width` on the target element. Other ResizeObserver consumers
 * should migrate to a shared helper if this mock grows.
 */
const emitWidth = (target: Element, width: number): void => {
  if (lastObserver === null) throw new Error('no observer registered');
  const entry = {
    target,
    contentRect: { width } as DOMRectReadOnly,
  } as unknown as ResizeObserverEntry;
  lastObserver.callback([entry]);
};

beforeEach(() => {
  lastObserver = null;
  // Stub the global — vi.stubGlobal auto-unstubs via the afterEach below.
  vi.stubGlobal('ResizeObserver', MockResizeObserver);
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// -----------------------------------------------------------------------------
// Breakpoint semantics (per docs/ui-specs/layout-panes.md §3.7 and
// pane-header.md §2.3)
//   full:       width >= 320
//   compact:    240 <= width < 320
//   icon-only:  width < 240
// hideMinimap === true for compactness !== 'full'.
// -----------------------------------------------------------------------------

describe('usePaneWidth — compactness thresholds', () => {
  const makeTarget = (): HTMLDivElement => {
    const el = document.createElement('div');
    document.body.appendChild(el);
    return el;
  };

  it('starts at "full" when no measurement has arrived yet (>=320px assumed)', () => {
    createRoot((dispose) => {
      const target = makeTarget();
      const { compactness, hideMinimap } = usePaneWidth(() => target);
      expect(compactness()).toBe<Compactness>('full');
      expect(hideMinimap()).toBe(false);
      dispose();
    });
  });

  it('reports "full" for width >= 320', () => {
    createRoot((dispose) => {
      const target = makeTarget();
      const { compactness, hideMinimap } = usePaneWidth(() => target);
      emitWidth(target, 400);
      expect(compactness()).toBe<Compactness>('full');
      expect(hideMinimap()).toBe(false);

      emitWidth(target, 320);
      expect(compactness()).toBe<Compactness>('full');
      expect(hideMinimap()).toBe(false);
      dispose();
    });
  });

  it('reports "compact" for 240 <= width < 320', () => {
    createRoot((dispose) => {
      const target = makeTarget();
      const { compactness, hideMinimap } = usePaneWidth(() => target);
      emitWidth(target, 319);
      expect(compactness()).toBe<Compactness>('compact');
      expect(hideMinimap()).toBe(true);

      emitWidth(target, 280);
      expect(compactness()).toBe<Compactness>('compact');
      expect(hideMinimap()).toBe(true);

      emitWidth(target, 240);
      expect(compactness()).toBe<Compactness>('compact');
      expect(hideMinimap()).toBe(true);
      dispose();
    });
  });

  it('reports "icon-only" for width < 240', () => {
    createRoot((dispose) => {
      const target = makeTarget();
      const { compactness, hideMinimap } = usePaneWidth(() => target);
      emitWidth(target, 239);
      expect(compactness()).toBe<Compactness>('icon-only');
      expect(hideMinimap()).toBe(true);

      emitWidth(target, 120);
      expect(compactness()).toBe<Compactness>('icon-only');
      expect(hideMinimap()).toBe(true);
      dispose();
    });
  });

  it('transitions full → compact → icon-only → compact → full as width shrinks and grows', () => {
    createRoot((dispose) => {
      const target = makeTarget();
      const { compactness } = usePaneWidth(() => target);

      emitWidth(target, 500);
      expect(compactness()).toBe<Compactness>('full');

      emitWidth(target, 319);
      expect(compactness()).toBe<Compactness>('compact');

      emitWidth(target, 239);
      expect(compactness()).toBe<Compactness>('icon-only');

      emitWidth(target, 260);
      expect(compactness()).toBe<Compactness>('compact');

      emitWidth(target, 321);
      expect(compactness()).toBe<Compactness>('full');

      dispose();
    });
  });
});

describe('usePaneWidth — lifecycle', () => {
  it('observes the target element exactly once on mount', () => {
    createRoot((dispose) => {
      const target = document.createElement('div');
      document.body.appendChild(target);
      usePaneWidth(() => target);
      if (lastObserver === null) throw new Error('expected an observer');
      expect(lastObserver.observed).toEqual([target]);
      dispose();
    });
  });

  it('disconnects the ResizeObserver on disposal', () => {
    createRoot((dispose) => {
      const target = document.createElement('div');
      document.body.appendChild(target);
      usePaneWidth(() => target);
      if (lastObserver === null) throw new Error('expected an observer');
      expect(lastObserver.disconnected).toBe(false);
      dispose();
      expect(lastObserver.disconnected).toBe(true);
    });
  });

  it('ignores entries whose target is not the observed element', () => {
    createRoot((dispose) => {
      const target = document.createElement('div');
      const other = document.createElement('div');
      document.body.append(target, other);
      const { compactness } = usePaneWidth(() => target);

      // A ResizeObserver can receive entries for other elements if the
      // consumer re-uses the same instance — the hook should filter.
      emitWidth(other, 100);
      expect(compactness()).toBe<Compactness>('full');

      emitWidth(target, 100);
      expect(compactness()).toBe<Compactness>('icon-only');
      dispose();
    });
  });

  it('stays inert with a null target accessor — no observer registered', () => {
    createRoot((dispose) => {
      // Callers often hold a Solid ref that's null before mount. The hook
      // should no-op until the ref resolves. Observe that no
      // MockResizeObserver is constructed for a persistent-null accessor.
      const api = usePaneWidth(() => null);
      expect(api.compactness()).toBe<Compactness>('full');
      expect(api.hideMinimap()).toBe(false);
      expect(lastObserver).toBeNull();
      dispose();
    });
  });

  it('attaches the observer once a ref-signal accessor resolves to an element', () => {
    // This is the ref-callback pattern SessionWindow uses:
    //   const [el, setEl] = createSignal<HTMLElement | null>(null);
    //   usePaneWidth(el);  <section ref={setEl} />
    // JSX ref callbacks fire *after* the component body returns, so the
    // hook's target accessor returns null at call time. The hook must
    // reactively re-attach once the ref resolves.
    createRoot((dispose) => {
      const [el, setEl] = createSignal<HTMLElement | null>(null);
      const { compactness } = usePaneWidth(el);

      // No observer yet — the accessor is null.
      expect(lastObserver).toBeNull();
      expect(compactness()).toBe<Compactness>('full');

      // Simulate the ref callback firing after the component returns.
      const target = document.createElement('div');
      document.body.appendChild(target);
      setEl(target);

      // Render-effect re-runs queue synchronously in Solid's reactive
      // graph; once the setEl call returns, the effect has re-executed
      // and the observer is attached to the new target.
      if (lastObserver === null) throw new Error('expected an observer');
      expect(lastObserver.observed).toEqual([target]);
      emitWidth(target, 200);
      expect(compactness()).toBe<Compactness>('icon-only');
      dispose();
    });
  });
});
