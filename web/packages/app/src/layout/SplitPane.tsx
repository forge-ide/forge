import { type Component, type JSX, createSignal } from 'solid-js';
import './SplitPane.css';

/**
 * Minimum ratio floor for either child. F-117 is ratio-only — the 320px
 * pixel minimum from `layout-panes.md` §3.7 is F-119 and intentionally
 * out of scope here. This floor just stops the divider from producing
 * a zero-width/height child or a negative value.
 */
export const MIN_RATIO = 0.1;

/**
 * Pixel snap increment for divider drag, per `layout-panes.md` §3.1:
 * "Dragging resizes in 4px snaps; Alt+drag is unsnapped."
 */
export const SNAP_PX = 4;

/** Balanced split used by the §3.1 double-click reset. */
export const RESET_RATIO = 0.5;

export interface SplitPaneProps {
  /** `"h"` → children stacked top/bottom; `"v"` → side by side. */
  direction: 'h' | 'v';
  /** First child's size as a fraction of the container (0..1). */
  ratio: number;
  /** Emitted on drag and on double-click reset. Caller owns the ratio. */
  onRatioChange: (next: number) => void;
  /** First child (top for `"h"`, left for `"v"`). */
  children: [JSX.Element, JSX.Element] | JSX.Element[];
}

/**
 * Two-child splitter with a draggable divider. Ratio is controlled — the
 * parent owns the value and receives updates through `onRatioChange`.
 *
 * Behavior comes from `docs/ui-specs/layout-panes.md`:
 *   - §3.1 "Gridlines are 1px in `--color-border-1`. Dragging resizes in
 *     4px snaps; Alt+drag is unsnapped. Double-clicking a gridline resets
 *     to balanced split."
 *   - §3.5 divider handle is the resize affordance — the ≥8px hit area and
 *     `col-resize` / `row-resize` cursors live here; the buttons in the tab
 *     bar are a separate concern.
 *
 * Out of scope (separate issues):
 *   F-118 drag-to-dock · F-119 320px min-width collapse · F-120 persistence
 */
export const SplitPane: Component<SplitPaneProps> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  const [dragging, setDragging] = createSignal(false);

  const clamp = (r: number): number => {
    if (r < MIN_RATIO) return MIN_RATIO;
    if (r > 1 - MIN_RATIO) return 1 - MIN_RATIO;
    return r;
  };

  // Translate an absolute pointer position into a clamped ratio along the
  // split axis. `getBoundingClientRect()` is the right primitive for this —
  // tests stub it on the container to supply a non-zero size in jsdom.
  const ratioFromPointer = (clientX: number, clientY: number): number | null => {
    if (!containerRef) return null;
    const rect = containerRef.getBoundingClientRect();
    const extent = props.direction === 'v' ? rect.width : rect.height;
    if (extent <= 0) return null;
    const offset =
      props.direction === 'v' ? clientX - rect.left : clientY - rect.top;
    return clamp(offset / extent);
  };

  // 4px snap per §3.1. Alt+drag bypasses the snap.
  const applySnap = (raw: number, altPressed: boolean): number => {
    if (altPressed || !containerRef) return raw;
    const rect = containerRef.getBoundingClientRect();
    const extent = props.direction === 'v' ? rect.width : rect.height;
    if (extent <= 0) return raw;
    const px = raw * extent;
    const snapped = Math.round(px / SNAP_PX) * SNAP_PX;
    return clamp(snapped / extent);
  };

  const handlePointerDown = (e: PointerEvent) => {
    // Only primary button — Tauri's non-native window chrome makes the
    // right/middle-button semantics of system panes less relevant here.
    if (e.button !== 0) return;
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    setDragging(true);
    e.preventDefault();
  };

  const handlePointerMove = (e: PointerEvent) => {
    if (!dragging()) return;
    const raw = ratioFromPointer(e.clientX, e.clientY);
    if (raw === null) return;
    props.onRatioChange(applySnap(raw, e.altKey));
  };

  const handlePointerUp = (e: PointerEvent) => {
    if (!dragging()) return;
    const target = e.currentTarget as HTMLElement;
    if (target.hasPointerCapture(e.pointerId)) {
      target.releasePointerCapture(e.pointerId);
    }
    setDragging(false);
  };

  // §3.1: "Double-clicking a gridline resets to balanced split."
  const handleDoubleClick = () => {
    props.onRatioChange(RESET_RATIO);
  };

  const firstStyle = (): JSX.CSSProperties => {
    const r = clamp(props.ratio);
    return props.direction === 'v'
      ? { width: `${r * 100}%` }
      : { height: `${r * 100}%` };
  };

  const secondStyle = (): JSX.CSSProperties => {
    const r = clamp(props.ratio);
    const rest = 1 - r;
    return props.direction === 'v'
      ? { width: `${rest * 100}%` }
      : { height: `${rest * 100}%` };
  };

  const [first, second] = props.children as [JSX.Element, JSX.Element];

  return (
    <div
      class="split-pane"
      classList={{
        'split-pane--h': props.direction === 'h',
        'split-pane--v': props.direction === 'v',
      }}
      data-direction={props.direction}
      data-testid="split-pane"
      ref={containerRef}
    >
      <div class="split-pane__child split-pane__child--first" style={firstStyle()}>
        {first}
      </div>
      <div
        class="split-pane__divider"
        data-testid="split-pane-divider"
        role="separator"
        aria-orientation={props.direction === 'v' ? 'vertical' : 'horizontal'}
        tabIndex={0}
        onPointerDown={handlePointerDown}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerCancel={handlePointerUp}
        onDblClick={handleDoubleClick}
      />
      <div class="split-pane__child split-pane__child--second" style={secondStyle()}>
        {second}
      </div>
    </div>
  );
};
