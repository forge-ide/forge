import type { Component } from 'solid-js';
import './PaneHeader.css';

export interface PaneHeaderProps {
  subject: string;
  providerLabel: string;
  costLabel: string;
  onClose: () => void;
}

/**
 * Pane header (28px) per docs/ui-specs/layout-panes.md §3.3. Shows the
 * session subject, provider label, and a cost meter placeholder, with a
 * close action on the right.
 */
export const PaneHeader: Component<PaneHeaderProps> = (props) => {
  return (
    <header class="pane-header" role="banner">
      <span class="pane-header__type-label">CHAT</span>
      <span class="pane-header__subject" data-testid="pane-header-subject">
        {props.subject}
      </span>
      <span class="pane-header__provider" data-testid="pane-header-provider">
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
