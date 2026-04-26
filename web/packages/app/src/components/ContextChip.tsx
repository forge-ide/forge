import {
  type Component,
  createSignal,
  onCleanup,
  Show,
} from 'solid-js';
import { IconButton } from '@forge/design';
import type { ContextCategory } from './ContextPicker';
import './ContextChip.css';

// ---------------------------------------------------------------------------
// ContextChip — pill shown in the composer's `ctx-chips` row once a picker
// result is inserted (F-141). The chip is intentionally minimal: icon +
// label + dismiss-×.
//
// F-142 adds the "lazy file preview" per the DoD: hovering a file chip
// expands a read-only preview popover with the file's content. Non-file
// chips show only their label — the preview is category-gated so a selection
// or terminal chip does not try to load something.
// ---------------------------------------------------------------------------

export interface ContextChipProps {
  category: ContextCategory;
  label: string;
  onDismiss: () => void;
  /**
   * Optional loader for the preview popover. Called with the chip's
   * identifier (`value`); expected to return the preview text. The chip only
   * shows a preview when `category === 'file'` AND `loadPreview` is provided
   * — other categories display only the chip label.
   */
  loadPreview?: (value: string) => Promise<string>;
  /** Chip identifier passed to `loadPreview`. Defaults to label. */
  value?: string;
}

function iconFor(category: ContextCategory): string {
  switch (category) {
    case 'file':
      return '[F]';
    case 'directory':
      return '[D]';
    case 'selection':
      return '[S]';
    case 'terminal':
      return '[T]';
    case 'agent':
      return '[A]';
    case 'skill':
      return '[K]';
    case 'url':
      return '[U]';
  }
}

/** Clip preview bodies so the popover never grows past a manageable size. */
const PREVIEW_MAX_CHARS = 4000;

export const ContextChip: Component<ContextChipProps> = (props) => {
  const [preview, setPreview] = createSignal<string | null>(null);
  const [hovered, setHovered] = createSignal(false);
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  let loadToken = 0;

  const canPreview = (): boolean =>
    props.category === 'file' && props.loadPreview !== undefined;

  const retry = (): void => {
    // Reset to allow a fresh load attempt.
    setPreview(null);
    setError(null);
    handleEnter();
  };

  const handleEnter = (): void => {
    if (!canPreview()) return;
    setHovered(true);
    // Clear any previous error so the next hover gets a fresh attempt (F-399).
    setError(null);
    if (preview() !== null || loading()) return;
    const token = ++loadToken;
    const value = props.value ?? props.label;
    setLoading(true);
    props
      .loadPreview!(value)
      .then((body) => {
        if (token !== loadToken) return;
        const clipped =
          body.length > PREVIEW_MAX_CHARS
            ? `${body.slice(0, PREVIEW_MAX_CHARS - 1)}…`
            : body;
        setPreview(clipped);
      })
      .catch((err: unknown) => {
        if (token !== loadToken) return;
        setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (token === loadToken) setLoading(false);
      });
  };

  const handleLeave = (): void => {
    setHovered(false);
  };

  onCleanup(() => {
    // Invalidate any in-flight loader on unmount.
    loadToken++;
  });

  return (
    <span
      class="ctx-chip"
      data-testid="ctx-chip"
      data-category={props.category}
      role="group"
      aria-label={props.label}
      onMouseEnter={handleEnter}
      onMouseLeave={handleLeave}
      onFocus={handleEnter}
      onBlur={handleLeave}
    >
      <span class="ctx-chip__icon" aria-hidden="true">
        {iconFor(props.category)}
      </span>
      <span class="ctx-chip__label">{props.label}</span>
      <IconButton
        size="sm"
        class="ctx-chip__dismiss"
        data-testid="ctx-chip-dismiss"
        label={`Remove ${props.label}`}
        onClick={() => props.onDismiss()}
        icon={'×'}
      />
      <Show when={canPreview() && hovered()}>
        <div
          class="ctx-chip__preview"
          data-testid="ctx-chip-preview"
          role="tooltip"
        >
          <Show when={loading()}>
            <span class="ctx-chip__preview-loading">loading…</span>
          </Show>
          <Show when={error() !== null}>
            <span class="ctx-chip__preview-error" role="alert">
              {error()}
            </span>
            <button
              type="button"
              class="ctx-chip__preview-retry"
              data-testid="ctx-chip-retry"
              onMouseDown={(e) => {
                // Prevent the chip's blur handler from hiding the popover
                // before the retry click registers.
                e.preventDefault();
                retry();
              }}
            >
              Retry
            </button>
          </Show>
          <Show when={preview() !== null}>
            <pre class="ctx-chip__preview-body">{preview()}</pre>
          </Show>
        </div>
      </Show>
    </span>
  );
};
