// Workspace search pane — placeholder.
//
// The eventual implementation will shell out to ripgrep (or similar)
// through a `workspace_search` IPC scoped to the active workspace root,
// stream matches into a virtualised list, and pipe a click into
// `openFile(path, line)` against the active session's bridge. Until that
// lands, the pane renders an honest "coming soon" panel that explains
// what the activity-bar entry is for so the icon doesn't feel broken.

import type { Component } from 'solid-js';
import './panes.css';

export const SearchPane: Component = () => (
  <aside
    class="workspace-pane"
    aria-label="Search"
    data-testid="workspace-pane-search"
  >
    <header class="workspace-pane__header">
      <span class="workspace-pane__title">SEARCH</span>
    </header>
    <div class="workspace-pane__placeholder">
      <p class="workspace-pane__placeholder-title">Coming soon</p>
      <p class="workspace-pane__placeholder-detail">
        Workspace-wide text search across files. The backend ripgrep
        bridge is on the roadmap.
      </p>
    </div>
  </aside>
);
