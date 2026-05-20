// Workspace plugins pane — placeholder.
//
// Forge doesn't yet have a public extension/plugin surface; the entry
// reserves the slot in the activity bar so the eventual extension manager
// has a natural home. When that lands, this pane will host the installed-
// extensions list, per-row enable toggle, and a "Browse marketplace"
// affordance — mirroring the Agents/Skills/MCP panes' shape.

import type { Component } from 'solid-js';
import './panes.css';

export const PluginsPane: Component = () => (
  <aside
    class="workspace-pane"
    aria-label="Plugins"
    data-testid="workspace-pane-plugins"
  >
    <header class="workspace-pane__header">
      <span class="workspace-pane__title">PLUGINS</span>
    </header>
    <div class="workspace-pane__placeholder">
      <p class="workspace-pane__placeholder-title">Coming soon</p>
      <p class="workspace-pane__placeholder-detail">
        Third-party extensions and marketplace integrations will land here.
      </p>
    </div>
  </aside>
);
