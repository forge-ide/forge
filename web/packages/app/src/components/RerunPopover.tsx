import { type Component } from 'solid-js';
import { Button } from '@forge/design';
import type { RerunVariant } from '@forge/ipc';
import { useFocusTrap } from '../lib/useFocusTrap';
import './RerunPopover.css';

/**
 * F-600 — Re-run popover (chat-pane.md §4 message hover-row `Re-run ▾`).
 *
 * Surfaced when the user opens the per-message Re-run dropdown on an
 * assistant turn. Three buttons map 1:1 to [`RerunVariant`] values handled
 * by the orchestrator:
 *
 *   - `Replace` (F-143) — truncate the transcript at this turn and
 *     regenerate in place. The original assistant message is superseded;
 *     replays show only the new one.
 *   - `Branch`  (F-144) — keep the original alongside a new sibling under
 *     the same branch root. The selector strip exposes both.
 *   - `Fresh`   (F-600) — truncate to the originating user message only,
 *     regenerate as a new root. Useful for "what if I started over here"
 *     experiments without comparing to prior context.
 *
 * Each button carries a verbose `title` tooltip explaining the difference,
 * keyed off the spec wording in `docs/ui-specs/branching.md` /
 * `docs/product/CONCEPT.md` §10.3. The popover uses surface-2 background +
 * border-default per the component principles.
 */
export interface RerunPopoverProps {
  /**
   * Fired with the chosen variant. The parent dispatches the IPC and
   * dismisses the popover.
   */
  onPick: (variant: RerunVariant) => void;
  /**
   * Fired on Escape / outside-click dismissal. The parent flips its
   * popover-open signal so the surface unmounts.
   */
  onDismiss: () => void;
}

export const RerunPopover: Component<RerunPopoverProps> = (props) => {
  let rootRef: HTMLDivElement | undefined;
  // Non-modal: this is a menu, not a dialog. Esc + outside-click dismiss
  // via the menu-mode useFocusTrap path (matches BranchMetadataPopover).
  useFocusTrap(() => rootRef, { trap: false, onDismiss: () => props.onDismiss() });

  const pick = (variant: RerunVariant): void => {
    props.onPick(variant);
  };

  return (
    <div
      ref={rootRef}
      class="rerun-popover"
      data-testid="rerun-popover"
      role="menu"
      aria-label="Re-run this turn"
    >
      <header class="rerun-popover__header">Re-run this turn</header>
      <ul class="rerun-popover__list">
        <li class="rerun-popover__row">
          <Button
            variant="ghost"
            size="sm"
            class="rerun-popover__btn"
            data-testid="rerun-popover-replace"
            role="menuitem"
            title="Replace: truncate the transcript at this turn and regenerate in place. The original answer is hidden on replay."
            onClick={() => pick('Replace')}
          >
            REPLACE TURN
          </Button>
        </li>
        <li class="rerun-popover__row">
          <Button
            variant="ghost"
            size="sm"
            class="rerun-popover__btn"
            data-testid="rerun-popover-branch"
            role="menuitem"
            title="Branch: keep the original answer and add a new variant alongside it. Use the branch selector strip to switch between variants."
            onClick={() => pick('Branch')}
          >
            BRANCH TURN
          </Button>
        </li>
        <li class="rerun-popover__row">
          <Button
            variant="ghost"
            size="sm"
            class="rerun-popover__btn"
            data-testid="rerun-popover-fresh"
            role="menuitem"
            title="Fresh: discard everything before this turn's user message and regenerate as if you started over from there. Unlike Replace (in-place rewrite) and Branch (preserve both), Fresh produces a new root."
            onClick={() => pick('Fresh')}
          >
            FRESH TURN
          </Button>
        </li>
      </ul>
    </div>
  );
};
