import { type Component, Show } from 'solid-js';
import './BranchSelectorStrip.css';

/**
 * F-145 — variant selector strip.
 *
 * Spec (`docs/ui-specs/branching.md` §15.3):
 *
 *   ┌────────────────────────────────────────────┐
 *   │ ◀  variant 2 of 3  ▶     [branch info ⓘ]  │ ← 20px strip, surface-2 bg
 *
 * The strip is only rendered when the owning branch group has more than one
 * live variant (the component doesn't hide itself — callers gate the mount
 * by consulting `liveVariantCount`). Left/right arrows cycle through live
 * siblings (the parent does the cycle math via `neighbourVariantId`); the
 * info icon toggles the metadata popover.
 *
 * Keyboard:
 *   - ArrowLeft / ArrowRight when the strip has keyboard focus → prev / next
 *   - Enter on the info icon → open popover
 *
 * The strip is surface-2 background (iron-800) with a 20px height, matching
 * the spec's mock. Arrow/icon buttons use the standard ghost-button treatment
 * defined in the CSS.
 */
export interface BranchSelectorStripProps {
  /** 1-indexed position of the active variant among live siblings. */
  position: number;
  /** Total number of live (non-deleted) variants in this group. */
  total: number;
  /** Invoked when the user requests the previous sibling. */
  onPrev: () => void;
  /** Invoked when the user requests the next sibling. */
  onNext: () => void;
  /** Invoked when the user toggles the metadata popover. */
  onToggleInfo: () => void;
  /** When `true`, the info button reflects the "open" state. */
  infoOpen: boolean;
}

export const BranchSelectorStrip: Component<BranchSelectorStripProps> = (props) => {
  const handleKey = (e: KeyboardEvent): void => {
    if (e.key === 'ArrowLeft') {
      e.preventDefault();
      props.onPrev();
      return;
    }
    if (e.key === 'ArrowRight') {
      e.preventDefault();
      props.onNext();
      return;
    }
  };

  return (
    <div
      class="branch-strip"
      data-testid="branch-selector-strip"
      role="group"
      aria-label="Message variant selector"
      tabIndex={0}
      onKeyDown={handleKey}
    >
      <button
        type="button"
        class="branch-strip__arrow branch-strip__arrow--prev"
        data-testid="branch-strip-prev"
        aria-label="Previous variant"
        onClick={props.onPrev}
      >
        {'\u25c0'}
      </button>
      <span class="branch-strip__label" data-testid="branch-strip-label">
        variant {props.position} of {props.total}
      </span>
      <button
        type="button"
        class="branch-strip__arrow branch-strip__arrow--next"
        data-testid="branch-strip-next"
        aria-label="Next variant"
        onClick={props.onNext}
      >
        {'\u25b6'}
      </button>
      <span class="branch-strip__spacer" aria-hidden="true" />
      <button
        type="button"
        class="branch-strip__info"
        data-testid="branch-strip-info"
        aria-label="Branch variant details"
        aria-expanded={props.infoOpen}
        onClick={props.onToggleInfo}
      >
        <Show when={props.infoOpen} fallback={<span aria-hidden="true">{'\u24d8'}</span>}>
          <span aria-hidden="true">{'\u24d8'}</span>
        </Show>
      </button>
    </div>
  );
};
