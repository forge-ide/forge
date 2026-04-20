import { createSignal, onCleanup } from 'solid-js';
import type { LayoutNode } from './GridContainer';
import { applyDockDrop, type DropZone, zoneForPoint } from './dockDrop';

/**
 * Active-drag state exposed to consumers so they can render the drag visuals
 * (e.g. `DropZoneOverlay` on the hovered pane).
 */
export interface DragState {
  /** Id of the leaf being dragged by its header. */
  sourceId: string;
  /** Id of the leaf currently under the pointer, if any. */
  targetId: string | null;
  /** Zone of the target currently under the pointer, if any. */
  zone: DropZone | null;
}

export interface UseDragToDockOptions {
  /** Get the current layout tree. Called lazily on drop. */
  getTree: () => LayoutNode;
  /** Emit a mutated tree on a successful drop. */
  onTreeChange: (next: LayoutNode) => void;
}

export interface UseDragToDockApi {
  /** Current drag state — `null` when no drag is in progress. */
  drag: () => DragState | null;
  /**
   * Start a drag from a pane header. The returned handler is wired onto the
   * header element (e.g. `onPointerDown`). Pass the leaf id the header
   * belongs to; the handler sets up the global move/up/key listeners and
   * tears them down on drop or abort.
   */
  startDrag: (sourceId: string) => (e: PointerEvent) => void;
}

/**
 * Drag-to-dock coordinator.
 *
 * Wiring: attach the handler from `startDrag(sourceId)` to a pane header's
 * `onPointerDown`. Until the user releases or presses Escape, the hook owns
 * global pointermove / pointerup / keydown listeners. On each pointermove it
 * resolves the leaf under the pointer via `document.elementFromPoint` → the
 * nearest `[data-leaf-id]` ancestor — that's the marker F-117 already puts
 * on every grid leaf. The hook then computes the zone via `zoneForPoint`
 * and updates its drag-state signal.
 *
 * On pointerup over a valid target the hook calls `applyDockDrop` and emits
 * the new tree via `onTreeChange`. Pointerup outside any target, or an
 * Escape keydown, aborts without mutating. The source is never dropped on
 * itself (center-on-self would otherwise be a structural no-op) — that case
 * aborts cleanly too.
 *
 * Listeners are attached to `window` with capture so the drag keeps tracking
 * even when the pointer hovers non-reactive native elements. Solid's
 * `onCleanup` tears them down if the component unmounts mid-drag.
 *
 * Follows `docs/ui-specs/layout-panes.md` §3.6 for zone semantics and §3.1
 * for pointer-driven interactions. Does not handle touch (F-118 targets the
 * desktop Tauri shell; the header uses `onPointerDown` so touch inherits
 * the model if the shell ever ships on tablet).
 */
export function useDragToDock(options: UseDragToDockOptions): UseDragToDockApi {
  const [drag, setDrag] = createSignal<DragState | null>(null);

  // Listener refs held on the closure so we can detach exactly the same
  // bound function we attached. Declared inline because they close over
  // `state` and must re-read the current value each call.
  let detach: (() => void) | null = null;

  const teardown = () => {
    if (detach !== null) {
      detach();
      detach = null;
    }
    setDrag(null);
  };

  const commitDrop = () => {
    const current = drag();
    if (current === null || current.targetId === null || current.zone === null) {
      teardown();
      return;
    }
    // A center drop on the source itself would try to remove-and-reinsert at
    // the same position — the tree-mutation helper treats `source === target`
    // as a no-op, which is the correct abort semantics.
    if (current.sourceId === current.targetId) {
      teardown();
      return;
    }
    const next = applyDockDrop(
      options.getTree(),
      current.sourceId,
      current.targetId,
      current.zone,
    );
    options.onTreeChange(next);
    teardown();
  };

  const resolveTargetAt = (clientX: number, clientY: number): {
    targetId: string | null;
    zone: DropZone | null;
  } => {
    // `elementFromPoint` needs a Document. In jsdom it's stubbed to null by
    // default; tests supply a stub. Guard the call so we fail safe.
    const doc = typeof document !== 'undefined' ? document : null;
    if (doc === null || typeof doc.elementFromPoint !== 'function') {
      return { targetId: null, zone: null };
    }
    const el = doc.elementFromPoint(clientX, clientY);
    if (el === null) return { targetId: null, zone: null };
    const leafEl = (el as Element).closest('[data-leaf-id]') as HTMLElement | null;
    if (leafEl === null) return { targetId: null, zone: null };
    const targetId = leafEl.getAttribute('data-leaf-id');
    if (targetId === null) return { targetId: null, zone: null };
    const rect = leafEl.getBoundingClientRect();
    const zone = zoneForPoint(clientX, clientY, rect);
    return { targetId, zone };
  };

  const startDrag = (sourceId: string) => (e: PointerEvent) => {
    // Primary-button only — matches SplitPane's convention for the divider.
    if (e.button !== 0) return;
    // Multiple drags at once would corrupt the listener bookkeeping. Ignore
    // a second pointerdown while one is in flight.
    if (drag() !== null) return;

    e.preventDefault();
    setDrag({ sourceId, targetId: null, zone: null });

    const handleMove = (ev: PointerEvent) => {
      const { targetId, zone } = resolveTargetAt(ev.clientX, ev.clientY);
      // If the resolved target is the source itself, treat it as no-op (no
      // highlight) so the user doesn't think they can split a pane onto
      // itself. `zone` stays null; drop will abort.
      if (targetId !== null && targetId === sourceId) {
        setDrag({ sourceId, targetId: null, zone: null });
        return;
      }
      setDrag({ sourceId, targetId, zone });
    };

    const handleUp = (_ev: PointerEvent) => {
      commitDrop();
    };

    const handleKey = (ev: KeyboardEvent) => {
      if (ev.key === 'Escape') {
        ev.preventDefault();
        teardown();
      }
    };

    // `capture: true` keeps the drag tracking even when the pointer crosses
    // native (non-framework) widgets — same rationale as divider drag.
    window.addEventListener('pointermove', handleMove, { capture: true });
    window.addEventListener('pointerup', handleUp, { capture: true });
    window.addEventListener('pointercancel', handleUp, { capture: true });
    window.addEventListener('keydown', handleKey, { capture: true });

    detach = () => {
      window.removeEventListener('pointermove', handleMove, { capture: true });
      window.removeEventListener('pointerup', handleUp, { capture: true });
      window.removeEventListener('pointercancel', handleUp, { capture: true });
      window.removeEventListener('keydown', handleKey, { capture: true });
    };
  };

  onCleanup(() => {
    teardown();
  });

  return { drag, startDrag };
}
