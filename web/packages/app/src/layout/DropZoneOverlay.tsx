import { type Component, For } from 'solid-js';
import type { DropZone } from './dockDrop';
import './DropZoneOverlay.css';

export interface DropZoneOverlayProps {
  /**
   * Zone currently under the pointer, if any. The overlay itself is purely
   * presentational — hit-testing happens in `useDragToDock` against the
   * underlying leaf's bounding rect, so the overlay never intercepts pointer
   * events (it's rendered with `pointer-events: none` end to end).
   */
  activeZone: DropZone | null;
}

const ZONES: readonly DropZone[] = ['top', 'bottom', 'left', 'right', 'center'];

/**
 * Absolutely-positioned overlay that paints five drop zones — top, bottom,
 * left, right, center — per `docs/ui-specs/layout-panes.md` §3.6. The active
 * zone receives the §3.6 ember tint; the rest are invisible placeholders so
 * the overlay imposes no visual noise when the pointer is elsewhere.
 *
 * The parent is expected to position this over the target leaf (the leaf
 * already renders inside a `position: relative`-capable container via
 * `grid-leaf`, so the overlay's `inset: 0` fills it).
 */
export const DropZoneOverlay: Component<DropZoneOverlayProps> = (props) => {
  return (
    <div class="drop-zone-overlay" data-testid="drop-zone-overlay" aria-hidden="true">
      <For each={ZONES}>
        {(zone) => (
          <div
            class="drop-zone-overlay__zone"
            classList={{
              [`drop-zone-overlay__zone--${zone}`]: true,
              'drop-zone-overlay__zone--active': props.activeZone === zone,
            }}
            data-testid={`drop-zone-${zone}`}
            data-zone={zone}
            data-active={props.activeZone === zone ? 'true' : 'false'}
          />
        )}
      </For>
    </div>
  );
};
