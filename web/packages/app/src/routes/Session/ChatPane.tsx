import type { Component } from 'solid-js';
import './ChatPane.css';

/**
 * Chat pane placeholder. Message list, composer, and streaming UX
 * land in F-025; this component exists so the Session window can
 * render the default single-pane layout today.
 */
export const ChatPane: Component = () => {
  return (
    <section class="chat-pane" data-testid="chat-pane" aria-label="Chat pane">
      <span class="chat-pane__type-label">CHAT</span>
      <p class="chat-pane__placeholder">Chat pane awaiting F-025.</p>
    </section>
  );
};
