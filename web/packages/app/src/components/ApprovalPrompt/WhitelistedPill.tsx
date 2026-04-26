import {
  type Component,
  createEffect,
  createSignal,
  onCleanup,
  Show,
} from 'solid-js';
import { Button, MenuItem } from '@forge/design';
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
  let wrapperRef: HTMLDivElement | undefined;

  // F-402: the popover is a non-modal menu — attach window-level Esc and
  // outside-click dismissal while open, detach when closed. Keeps listener
  // lifetimes tied to visibility, not to component lifetime.
  createEffect(() => {
    if (!popoverOpen()) return;

    const onKeyDown = (e: KeyboardEvent): void => {
      if (e.key === 'Escape') {
        e.preventDefault();
        setPopoverOpen(false);
      }
    };

    const onOutsideMouseDown = (e: MouseEvent): void => {
      if (!wrapperRef) return;
      const target = e.target as Node | null;
      if (target && wrapperRef.contains(target)) return;
      setPopoverOpen(false);
    };

    window.addEventListener('keydown', onKeyDown);
    document.addEventListener('mousedown', onOutsideMouseDown);

    onCleanup(() => {
      window.removeEventListener('keydown', onKeyDown);
      document.removeEventListener('mousedown', onOutsideMouseDown);
    });
  });

  return (
    <div ref={wrapperRef} class="whitelisted-pill-wrapper">
      <Button
        variant="ghost"
        size="sm"
        class="whitelisted-pill"
        data-testid="whitelisted-pill"
        aria-haspopup="true"
        aria-expanded={popoverOpen()}
        onClick={() => setPopoverOpen((v) => !v)}
      >
        <span class="whitelisted-pill__dot" aria-hidden="true" />
        whitelisted · {props.label}
        {provenanceSuffix(props.level)}
      </Button>

      <Show when={popoverOpen()}>
        <div
          class="whitelisted-pill__popover"
          data-testid="whitelist-popover"
          role="menu"
          aria-label="Revoke approval"
        >
          <MenuItem
            class="whitelisted-pill__revoke-btn"
            data-testid="revoke-btn"
            onClick={() => {
              setPopoverOpen(false);
              props.onRevoke();
            }}
          >
            {revokeLabel(props.level)}
          </MenuItem>
        </div>
      </Show>
    </div>
  );
};
