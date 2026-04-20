import {
  type Accessor,
  createComputed,
  createSignal,
  onCleanup,
} from 'solid-js';

/**
 * Compactness levels derived from the pane's content-box width.
 *
 * Thresholds come from `docs/ui-specs/layout-panes.md §3.7` and
 * `docs/ui-specs/pane-header.md §2.3`:
 *   - `full`        — width ≥ 320px; show all chrome at normal density
 *   - `compact`     — 240 ≤ width < 320; label → icon, secondary badges
 *                     auto-hide, editor/terminal hosts drop the minimap
 *   - `icon-only`   — width < 240; all non-essential badges/chrome collapse
 *
 * The 320px floor is the pane-minimum from §3.7; 240px is the "icon-only"
 * threshold noted in §3.7 and `pane-header.md §2.3` for the narrowest docked
 * layout (four-pane grid on a 1080px window, roughly).
 */
export type Compactness = 'full' | 'compact' | 'icon-only';

/** Threshold below which panes drop to `compact` (label→icon, badges hide). */
export const COMPACT_THRESHOLD_PX = 320;
/** Threshold below which panes drop to `icon-only` (all secondary chrome off). */
export const ICON_ONLY_THRESHOLD_PX = 240;

export interface UsePaneWidthApi {
  /** Current compactness bucket derived from the observed width. */
  compactness: Accessor<Compactness>;
  /**
   * True when the pane should suppress its minimap — i.e. any compactness
   * narrower than `full`. Future Editor/Terminal hosts read this signal.
   */
  hideMinimap: Accessor<boolean>;
}

/**
 * Bucket a measured width into a `Compactness` level. Exported for tests
 * and for callers that have a width already (e.g. imperative layout code).
 */
export function compactnessForWidth(width: number): Compactness {
  if (width < ICON_ONLY_THRESHOLD_PX) return 'icon-only';
  if (width < COMPACT_THRESHOLD_PX) return 'compact';
  return 'full';
}

/**
 * Observe a pane host element's content-box width via `ResizeObserver` and
 * expose its derived `compactness` level + a convenience `hideMinimap`
 * signal.
 *
 * The target accessor is re-read inside a `createComputed` so both call
 * patterns are supported:
 *
 *   - **Imperative** — `usePaneWidth(() => someElement)`. The accessor
 *     returns the element synchronously; the computation attaches
 *     immediately. Tests use this form.
 *   - **Ref signal** — `const [el, setEl] = createSignal<HTMLElement | null>(null);
 *     usePaneWidth(el); return <div ref={setEl} />`. The accessor returns
 *     `null` at hook-call time because JSX ref callbacks fire *after* the
 *     component body returns. The computation re-runs once `el()` flips
 *     to the element, which attaches the observer at the right moment.
 *
 *   `createComputed` (not `createEffect` or `createRenderEffect`) is the
 *   only Solid primitive that re-runs **synchronously** on signal change
 *   inside `createRoot`. The hook needs synchronous re-attach so tests can
 *   drive `setEl(el)` followed by `emitWidth(el, …)` in a straight line,
 *   and so production mounts of `SessionWindow` see the observer attach
 *   in the same tick their ref callback fires. `onCleanup` inside the
 *   computation releases the previous observer when the accessor changes
 *   and on root disposal.
 *
 * Starts at `full` until the first `ResizeObserver` callback fires — the
 * default pane width in F-117 is wider than 320px, and starting at `full`
 * avoids a mount-time flash to `icon-only` while the DOM is still laying
 * out. If `targetAccessor()` returns `null`, the hook stays inert until
 * the accessor resolves.
 */
export function usePaneWidth(
  targetAccessor: Accessor<Element | null>,
): UsePaneWidthApi {
  const [compactness, setCompactness] = createSignal<Compactness>('full');

  // Derived accessor instead of a signal so the two stay in lockstep — no
  // chance of `hideMinimap` lagging a `compactness` update by a tick.
  const hideMinimap: Accessor<boolean> = () => compactness() !== 'full';

  createComputed(() => {
    const target = targetAccessor();
    if (target === null) return;

    // `ResizeObserver` is present in every modern browser the Tauri shell
    // embeds. jsdom's absence is handled by callers stubbing the global in
    // tests that drive the hook directly. If a caller (e.g. SessionWindow)
    // uses the hook in a test environment without that stub, degrade to
    // `full` — the hook must not throw.
    if (typeof ResizeObserver === 'undefined') return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        // A single observer instance can receive entries for elements that
        // are not this hook's target if the caller reuses the observer —
        // filter to the one we care about.
        if (entry.target !== target) continue;
        const width = entry.contentRect.width;
        const next = compactnessForWidth(width);
        // Avoid redundant setSignal calls so downstream memos don't churn.
        if (next !== compactness()) setCompactness(next);
      }
    });
    observer.observe(target);

    onCleanup(() => {
      observer.disconnect();
    });
  });

  return { compactness, hideMinimap };
}
