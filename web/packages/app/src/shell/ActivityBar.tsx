// F-126: 44px left-edge activity bar. Per `docs/ui-specs/shell.md` §2 the
// bar hosts a vertical stack of icon buttons that toggle activity-bar
// content in the adjacent sidebar slot. `shell.md` only extracts §2; the
// finer §2.1 icon-set is not yet copied into the repo, so for this landing
// we render three buttons — Files (active), Search, Git — with the latter
// two as visible-but-disabled placeholders. F-127/F-128 will wire Search
// and Git to their own sidebars.
//
// The component is a controlled visual — it emits `onSelect(activity)` and
// accepts `active` from the parent. The parent (SessionWindow) owns the
// sidebar-visibility signal so a keyboard shortcut (`Cmd/Ctrl+Shift+E`)
// can toggle it without reaching through a ref.

import type { Component } from 'solid-js';
import { For } from 'solid-js';
import { IconButton } from '@forge/design';
import './ActivityBar.css';

export type ActivityId = 'files' | 'search' | 'git';

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
  /** Keyboard shortcut shown in the tooltip. Files is Cmd/Ctrl+Shift+E per
   *  the issue. Search / Git shortcuts land with F-127 / F-128. */
  shortcut?: string;
  /** Placeholder until the sidebar is wired. Disabled buttons keep the
   *  visual chrome intact so the 44px grid renders correctly. */
  disabled?: boolean;
  /** Inline SVG path data. Tiny hand-rolled icons that match the linework
   *  weight in `docs/forge-mocks.html` (1.7px stroke). Replaced with a
   *  shared icon set when F-148 lands the chrome pass. */
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
    id: 'git',
    label: 'Source control (coming soon)',
    disabled: true,
    svg: 'M5 3v10a4 4 0 0 0 4 4h6M5 7a2 2 0 1 0 0-4 2 2 0 0 0 0 4zm14 14a2 2 0 1 0 0-4 2 2 0 0 0 0 4z',
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
