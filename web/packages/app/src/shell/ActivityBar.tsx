// Workspace-window activity bar. The session window is conceptually a
// workspace window — its primary navigation lives here, distinct from the
// dashboard window's Sidebar (hidden in session windows per AppShell).
//
// The bar hosts a vertical stack of icon buttons; clicking one toggles
// the matching sidebar pane (Files / Search / Chat / Agents / Skills /
// MCP / Plugins). The parent (AppShell) owns the active-activity signal
// so a keyboard shortcut like `Cmd/Ctrl+Shift+E` can toggle Files without
// reaching through a ref.
//
// The component is a controlled visual — it emits `onSelect(activity)`
// and accepts `active` from the parent. Disabled entries (`search`,
// `plugins` today) keep their visual chrome so the 44px grid renders
// correctly while their backing IPCs are still in flight.

import type { Component } from 'solid-js';
import { For } from 'solid-js';
import { IconButton } from '@forge/design';
import './ActivityBar.css';

export type ActivityId =
  | 'files'
  | 'search'
  | 'chat'
  | 'agents'
  | 'skills'
  | 'mcp'
  | 'plugins';

export interface ActivityBarProps {
  /** Currently selected activity, or `null` when no sidebar is open. */
  active: ActivityId | null;
  /** Emitted when a user clicks an activity icon. Parent decides whether
   *  the click opens, toggles, or no-ops the sidebar. */
  onSelect: (activity: ActivityId) => void;
}

interface ActivityDef {
  id: ActivityId;
  label: string;
  /** Keyboard shortcut shown in the tooltip. */
  shortcut?: string;
  /** Placeholder until the sidebar is wired. Disabled buttons keep the
   *  visual chrome intact so the 44px grid renders correctly. */
  disabled?: boolean;
  /** Inline SVG path data. Hand-rolled icons that match the 1.7px stroke
   *  convention used by the dashboard Sidebar. */
  svg: string;
}

const ACTIVITIES: ActivityDef[] = [
  {
    id: 'files',
    label: 'Files',
    shortcut: 'Cmd/Ctrl+Shift+E',
    svg: 'M3 4h6l2 2h10v12a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V6a2 2 0 0 1 2-2z',
  },
  {
    id: 'search',
    label: 'Search (coming soon)',
    disabled: true,
    svg: 'M11 19a8 8 0 1 0-5.3-14.1A8 8 0 0 0 11 19zm10 2-4.3-4.3',
  },
  {
    id: 'chat',
    label: 'Chat',
    // Speech bubble — sessions list in the sidebar; clicking switches /
    // creates sessions in the active workspace.
    svg: 'M21 12a8 8 0 1 1-3.5-6.6L21 4l-1.2 4.1A8 8 0 0 1 21 12zM8 12h.01M12 12h.01M16 12h.01',
  },
  {
    id: 'agents',
    label: 'Agents',
    // Persona dot + shoulders — same glyph the dashboard Sidebar uses
    // for its Agents row so the two surfaces visually agree.
    svg: 'M12 12a4 4 0 1 0 0-8 4 4 0 0 0 0 8zM4 21a8 8 0 0 1 16 0',
  },
  {
    id: 'skills',
    label: 'Skills',
    // Star — same shape the dashboard Sidebar uses for Skills.
    svg: 'm12 3 2.7 5.5 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.8 1-6.1L3.2 9.4l6.1-.9z',
  },
  {
    id: 'mcp',
    label: 'MCP servers',
    // Stacked rows — matches the dashboard Sidebar's MCP glyph.
    svg: 'M4 7h16M4 12h16M4 17h10',
  },
  {
    id: 'plugins',
    label: 'Plugins (coming soon)',
    disabled: true,
    // Puzzle piece outline — universal "extension" affordance.
    svg: 'M14 4h4v4h2a2 2 0 0 1 0 4h-2v4h-4a2 2 0 1 0-4 0H6v-4H4a2 2 0 1 0 0-4h2V4h4a2 2 0 1 1 4 0z',
  },
];

export const ActivityBar: Component<ActivityBarProps> = (props) => {
  return (
    <nav
      class="activity-bar"
      aria-label="Activity bar"
      data-testid="activity-bar"
    >
      <For each={ACTIVITIES}>
        {(item) => {
          const isActive = () => props.active === item.id;
          const title = () =>
            item.shortcut ? `${item.label} (${item.shortcut})` : item.label;
          return (
            <IconButton
              class={`activity-bar__item${isActive() ? ' activity-bar__item--active' : ''}`}
              label={item.label}
              title={title()}
              pressed={isActive()}
              disabled={item.disabled}
              data-testid={`activity-bar-${item.id}`}
              onClick={() => props.onSelect(item.id)}
              icon={
                <svg
                  class="activity-bar__icon"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  stroke-width="1.7"
                  stroke-linecap="round"
                  stroke-linejoin="round"
                  aria-hidden="true"
                >
                  <path d={item.svg} />
                </svg>
              }
            />
          );
        }}
      </For>
    </nav>
  );
};
