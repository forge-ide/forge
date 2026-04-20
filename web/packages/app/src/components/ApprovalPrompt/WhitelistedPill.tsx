import { type Component, createSignal, Show } from 'solid-js';
import type { ApprovalLevel } from '@forge/ipc';
import './WhitelistedPill.css';

export interface WhitelistedPillProps {
  label: string;
  /**
   * F-036: persistence tier this approval came from. Drives both the
   * provenance suffix ("· workspace" / "· user") and which file the revoke
   * button should drop the entry from.
   */
  level: ApprovalLevel;
  onRevoke: () => void;
}

/**
 * F-036: human-readable provenance word shown after the label.
 * - `session` → no suffix (legacy default, matches existing UI for in-memory
 *   entries).
 * - `workspace` / `user` → ` · workspace` / ` · user` so the user sees which
 *   tier persists the approval.
 */
function provenanceSuffix(level: ApprovalLevel): string {
  return level === 'session' ? '' : ` · ${level}`;
}

function revokeLabel(level: ApprovalLevel): string {
  switch (level) {
    case 'session':
      return 'Revoke for this session';
    case 'workspace':
      return 'Revoke for this workspace';
    case 'user':
      return 'Revoke for this user';
  }
}

export const WhitelistedPill: Component<WhitelistedPillProps> = (props) => {
  const [popoverOpen, setPopoverOpen] = createSignal(false);

  return (
    <div class="whitelisted-pill-wrapper">
      <button
        type="button"
        class="whitelisted-pill"
        data-testid="whitelisted-pill"
        aria-haspopup="true"
        aria-expanded={popoverOpen()}
        onClick={() => setPopoverOpen((v) => !v)}
      >
        <span class="whitelisted-pill__dot" aria-hidden="true" />
        whitelisted · {props.label}
        {provenanceSuffix(props.level)}
      </button>

      <Show when={popoverOpen()}>
        <div class="whitelisted-pill__popover" data-testid="whitelist-popover" role="dialog">
          <button
            type="button"
            class="whitelisted-pill__revoke-btn"
            data-testid="revoke-btn"
            onClick={() => {
              setPopoverOpen(false);
              props.onRevoke();
            }}
          >
            {revokeLabel(props.level)}
          </button>
        </div>
      </Show>
    </div>
  );
};
