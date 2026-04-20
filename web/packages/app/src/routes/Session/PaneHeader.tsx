import type { Component, JSX } from 'solid-js';
import { Show } from 'solid-js';
import type { ProviderId } from '@forge/ipc';
import { providerAccent } from './providerAccent';
import './PaneHeader.css';

/**
 * Visual pane type (`pane-header.md §PH.2` type label). `CHAT` is the default
 * so existing session-window callers do not need to pass it explicitly; new
 * TERMINAL/EDITOR panes opt in.
 */
export type PaneHeaderType = 'CHAT' | 'TERMINAL' | 'EDITOR';

export interface PaneHeaderProps {
  subject: string;
  /**
   * Active provider id (F-091). Drives the pill accent color via the
   * `--pane-header-provider-accent` custom property; the display text remains
   * the caller-supplied `providerLabel`.
   *
   * Only meaningful for chat panes. Non-chat panes (terminal, editor) omit
   * the provider pill by leaving `providerId`/`providerLabel` `undefined`.
   */
  providerId?: ProviderId;
  providerLabel?: string;
  /**
   * Cost meter text. Chat panes render an `in … · out … · $…` string; other
   * pane types pass a short status string (e.g. a terminal's `cwd`) or
   * `undefined` to suppress the slot entirely.
   */
  costLabel?: string;
  /**
   * Close-button label copy (`pane-header.md §PH.6`). Chat panes use
   * `CLOSE SESSION`; terminal/editor panes pass `CLOSE PANE`/`CLOSE TAB`.
   * Defaults to `CLOSE SESSION` so existing callers keep their behavior.
   */
  closeLabel?: string;
  /**
   * Accessible name for the close button — chat uses `Close session window`;
   * terminal/editor variants pass `Close pane` / `Close tab`.
   */
  closeAriaLabel?: string;
  /**
   * `CHAT` | `TERMINAL` | `EDITOR` label shown in the type slot. Defaults to
   * `CHAT` for call-site compatibility with the Phase-1 session window.
   */
  typeLabel?: PaneHeaderType;
  onClose: () => void;
}

/**
 * Pane header (28px) per `docs/ui-specs/pane-header.md`. Shows the type
 * label + subject, an optional provider pill (chat), an optional cost/detail
 * meter (chat cost, terminal cwd), and a close action.
 *
 * F-125: non-chat panes opt out of the provider pill by omitting `providerId`
 * / `providerLabel`. `costLabel` is also optional so the terminal variant can
 * render the cwd in the same margin-left-auto slot, and `closeLabel` lets the
 * terminal variant render `CLOSE PANE` instead of `CLOSE SESSION`.
 */
export const PaneHeader: Component<PaneHeaderProps> = (props) => {
  // Inline custom property keeps `.pane-header__provider` rule generic; the
  // rule reads `var(--pane-header-provider-accent, var(--color-provider-local))`.
  const pillStyle = (): JSX.CSSProperties | undefined => {
    if (!props.providerId) return undefined;
    return {
      '--pane-header-provider-accent': providerAccent(props.providerId),
    };
  };
  return (
    <header class="pane-header" role="banner">
      <span class="pane-header__type-label">{props.typeLabel ?? 'CHAT'}</span>
      <span class="pane-header__subject" data-testid="pane-header-subject">
        {props.subject}
      </span>
      <Show when={props.providerId !== undefined && props.providerLabel !== undefined}>
        <span
          class="pane-header__provider"
          data-testid="pane-header-provider"
          style={pillStyle()}
        >
          {props.providerLabel}
        </span>
      </Show>
      <Show when={props.costLabel !== undefined}>
        <span class="pane-header__cost" data-testid="pane-header-cost">
          {props.costLabel}
        </span>
      </Show>
      <button
        type="button"
        class="pane-header__close"
        aria-label={props.closeAriaLabel ?? 'Close session window'}
        onClick={props.onClose}
      >
        {props.closeLabel ?? 'CLOSE SESSION'}
      </button>
    </header>
  );
};
