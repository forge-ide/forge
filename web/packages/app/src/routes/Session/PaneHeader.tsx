import type { Component, JSX } from 'solid-js';
import type { ProviderId } from '@forge/ipc';
import { providerAccent } from './providerAccent';
import './PaneHeader.css';

export interface PaneHeaderProps {
  subject: string;
  /**
   * Active provider id (F-091). Drives the pill accent color via the
   * `--pane-header-provider-accent` custom property; the display text remains
   * the caller-supplied `providerLabel`.
   */
  providerId: ProviderId;
  providerLabel: string;
  costLabel: string;
  onClose: () => void;
}

/**
 * Pane header (28px) per docs/ui-specs/layout-panes.md §3.3. Shows the
 * session subject, provider label, and a cost meter placeholder, with a
 * close action on the right.
 *
 * The provider pill color follows `ai-patterns.md §7` — anthropic→ember,
 * openai→amber, ollama/lm-studio/local→steel, otherwise custom (iron-200).
 * The accent is plumbed through an inline CSS custom property so adding a
 * new provider requires only a `providerAccent` mapping update — no CSS edit.
 */
export const PaneHeader: Component<PaneHeaderProps> = (props) => {
  // Inline custom property keeps `.pane-header__provider` rule generic; the
  // rule reads `var(--pane-header-provider-accent, var(--color-provider-local))`.
  const pillStyle = (): JSX.CSSProperties => ({
    '--pane-header-provider-accent': providerAccent(props.providerId),
  });
  return (
    <header class="pane-header" role="banner">
      <span class="pane-header__type-label">CHAT</span>
      <span class="pane-header__subject" data-testid="pane-header-subject">
        {props.subject}
      </span>
      <span
        class="pane-header__provider"
        data-testid="pane-header-provider"
        style={pillStyle()}
      >
        {props.providerLabel}
      </span>
      <span class="pane-header__cost" data-testid="pane-header-cost">
        {props.costLabel}
      </span>
      <button
        type="button"
        class="pane-header__close"
        aria-label="Close session window"
        onClick={props.onClose}
      >
        CLOSE SESSION
      </button>
    </header>
  );
};
