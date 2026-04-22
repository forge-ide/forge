import { type Component, For, Show, createMemo, onCleanup, onMount } from 'solid-js';
import './BranchMetadataPopover.css';

/**
 * F-145 — branch metadata popover.
 *
 * Spec `docs/ui-specs/branching.md` §15.5:
 *
 *   ┌─────────────────────────────────────┐
 *   │ 3 variants of this response         │
 *   ├─────────────────────────────────────┤
 *   │ ● variant 1   sonnet-4.5  14:22:11  │
 *   │   "I'll read the current…"          │
 *   ...
 *   │ [Delete variant 1]  [Export all]    │
 *   └─────────────────────────────────────┘
 *
 * Lists every live variant with provider/model label, timestamp, and a
 * preview line. Clicking a row selects that variant. Per-row Delete and
 * a footer Export-all action are provided — Delete is disabled on
 * variant 0 when siblings remain (the orchestrator will refuse it anyway
 * but we gate here too so the UI never offers an action that will bounce).
 */

export interface VariantRow {
  /** Index in the branch group's `variantIds` array (0 for the root). */
  index: number;
  /** Stable MessageId. */
  message_id: string;
  /** Provider tag (e.g. `"anthropic"`). Absent on legacy fixtures. */
  provider?: string;
  /** Model name (e.g. `"sonnet-4.5"`). Absent on legacy fixtures. */
  model?: string;
  /** ISO-8601 timestamp. Absent on legacy fixtures. */
  at?: string;
  /** Preview text — first line of the variant's body, pre-clipped by caller. */
  preview: string;
}

export interface BranchMetadataPopoverProps {
  /** Rows to render, ordered by ascending `index`. */
  variants: VariantRow[];
  /** Active variant message_id. Highlighted in the list. */
  activeVariantId: string;
  /** Fired when the user picks a row. */
  onSelect: (messageId: string) => void;
  /**
   * Fired when the user clicks a row's Delete button. `variantIndex`
   * identifies the sibling in the branch group. The parent decides whether
   * to dispatch the `delete_branch` IPC command or to roll back.
   */
  onDelete: (variantIndex: number) => void;
  /**
   * Fired when the user clicks the footer "Export all" action. The
   * parent is responsible for serialising the active branch path and
   * writing it to the clipboard.
   */
  onExportAll: () => void;
  /** Fired when the user dismisses the popover (Esc or outside-click). */
  onDismiss: () => void;
}

const PREVIEW_MAX_CHARS = 80;

/**
 * Format an ISO timestamp into `HH:MM:SS`. The spec's mock shows exactly
 * this shape. Falls back to the raw string when Date parsing fails.
 */
function formatTime(at: string | undefined): string {
  if (!at) return '';
  const d = new Date(at);
  if (Number.isNaN(d.getTime())) return at;
  const pad = (n: number): string => String(n).padStart(2, '0');
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export const BranchMetadataPopover: Component<BranchMetadataPopoverProps> = (props) => {
  const liveCount = createMemo(() => props.variants.length);

  // F-402: this surface is not modal; it is a contextual menu with its own
  // dismiss affordances (Esc, outside-click). Exposed as role="menu" with
  // window-level outside-click dismissal.
  let rootRef: HTMLDivElement | undefined;

  const handleKey = (e: KeyboardEvent): void => {
    if (e.key === 'Escape') {
      e.preventDefault();
      props.onDismiss();
    }
  };

  const handleOutsideMouseDown = (e: MouseEvent): void => {
    if (!rootRef) return;
    const target = e.target as Node | null;
    if (target && rootRef.contains(target)) return;
    props.onDismiss();
  };

  onMount(() => {
    document.addEventListener('mousedown', handleOutsideMouseDown);
  });

  onCleanup(() => {
    document.removeEventListener('mousedown', handleOutsideMouseDown);
  });

  const canDelete = (row: VariantRow): boolean => {
    // Guard against deleting the root while siblings remain. The sibling
    // count here is the caller-provided `variants.length` minus this row;
    // if the user deletes the root and only this row exists, there's one
    // variant left — not a problem. If deleting the root would leave
    // siblings orphaned (index 0 with N > 1), reject.
    if (row.index === 0 && liveCount() > 1) return false;
    return true;
  };

  return (
    <div
      ref={rootRef}
      class="branch-popover"
      data-testid="branch-metadata-popover"
      role="menu"
      aria-label="Branch variants"
      tabIndex={-1}
      onKeyDown={handleKey}
    >
      <header class="branch-popover__header">
        {liveCount()} variant{liveCount() === 1 ? '' : 's'} of this response
      </header>
      <ul class="branch-popover__list">
        <For each={props.variants}>
          {(row) => {
            const isActive = (): boolean => row.message_id === props.activeVariantId;
            const preview =
              row.preview.length > PREVIEW_MAX_CHARS
                ? `${row.preview.slice(0, PREVIEW_MAX_CHARS - 1)}\u2026`
                : row.preview;
            return (
              <li
                class="branch-popover__row"
                classList={{ 'branch-popover__row--active': isActive() }}
                data-testid={`branch-popover-row-${row.index}`}
                data-variant-index={String(row.index)}
              >
                <button
                  type="button"
                  class="branch-popover__row-body"
                  data-testid={`branch-popover-select-${row.index}`}
                  onClick={() => props.onSelect(row.message_id)}
                  aria-current={isActive() ? 'true' : undefined}
                >
                  <span class="branch-popover__row-head">
                    <span class="branch-popover__bullet" aria-hidden="true">
                      {'\u25cf'}
                    </span>
                    <span class="branch-popover__variant-label">
                      variant {row.index}
                    </span>
                    <Show when={row.model}>
                      <span class="branch-popover__model">{row.model}</span>
                    </Show>
                    <Show when={row.at}>
                      <span class="branch-popover__time">{formatTime(row.at)}</span>
                    </Show>
                    <Show when={isActive()}>
                      <span class="branch-popover__active-tag">(active)</span>
                    </Show>
                  </span>
                  <span class="branch-popover__preview">
                    &quot;{preview}&quot;
                  </span>
                </button>
                <button
                  type="button"
                  class="branch-popover__delete"
                  data-testid={`branch-popover-delete-${row.index}`}
                  aria-label={`Delete variant ${row.index}`}
                  disabled={!canDelete(row)}
                  onClick={() => props.onDelete(row.index)}
                >
                  DELETE VARIANT
                </button>
              </li>
            );
          }}
        </For>
      </ul>
      <footer class="branch-popover__footer">
        <button
          type="button"
          class="branch-popover__export"
          data-testid="branch-popover-export"
          onClick={props.onExportAll}
        >
          EXPORT ALL
        </button>
      </footer>
    </div>
  );
};
