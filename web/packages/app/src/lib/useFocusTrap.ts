// F-402: focus-trap / menu-dismiss hook for role="dialog" and role="menu" surfaces.
//
// Two modes, selected by `opts.trap`:
//
//   trap: true (default) — modal path, WAI-ARIA APG dialog contract:
//     - Moves focus into the container on activation (explicit target via
//       opts.initialFocus, else the first focusable descendant).
//     - Cycles Tab / Shift+Tab at the boundaries so keyboard focus cannot
//       escape the dialog.
//     - Restores focus to whatever element was active before activation on
//       cleanup, when that element is still attached.
//
//   trap: false — menu / non-modal popover path (F-402-followup):
//     - Does NOT move focus or cycle Tab.
//     - Registers window-level `keydown` (Escape) and `mousedown`
//       (outside-click) listeners that call `opts.onDismiss`.
//     - Listener lifetime is tied to the reactive scope — cleanup removes
//       both listeners. Hosts no longer duplicate this wiring inline.
//
// Callers must invoke this inside a reactive scope (component setup or
// createRoot) so the onMount/onCleanup lifecycle fires.

import { onCleanup, onMount } from 'solid-js';

const FOCUSABLE_SELECTOR =
  'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

export interface UseFocusTrapOptions {
  /**
   * Element to receive focus on activation. When omitted, the first
   * focusable descendant of the container is used. Returning `undefined`
   * from this thunk falls back to that default. Ignored when `trap` is false.
   */
  initialFocus?: () => HTMLElement | undefined;
  /**
   * When true (default), apply modal focus-trap semantics: auto-focus,
   * Tab cycling, and focus restoration on cleanup. When false, the hook
   * runs in menu mode: no focus movement, no Tab cycling; instead it
   * installs window-level Esc and outside-click dismissal wired to
   * `onDismiss`.
   */
  trap?: boolean;
  /**
   * Called on Escape (window-level) or mousedown outside the container.
   * Required when `trap` is false — this is how the menu path signals
   * dismissal to the host. Ignored when `trap` is true.
   */
  onDismiss?: () => void;
}

export function useFocusTrap(
  getContainer: () => HTMLElement | undefined,
  opts: UseFocusTrapOptions = {},
): void {
  const trap = opts.trap ?? true;

  if (trap) {
    useFocusTrapModal(getContainer, opts);
  } else {
    useFocusTrapMenu(getContainer, opts.onDismiss);
  }
}

function useFocusTrapModal(
  getContainer: () => HTMLElement | undefined,
  opts: UseFocusTrapOptions,
): void {
  // Capture the pre-activation active element synchronously during setup
  // so we can restore focus on cleanup even if onMount steals focus first.
  const previouslyFocused =
    typeof document !== 'undefined'
      ? (document.activeElement as HTMLElement | null)
      : null;

  const focusables = (): HTMLElement[] => {
    const root = getContainer();
    if (!root) return [];
    return Array.from(root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR));
  };

  const handleKeyDown = (e: KeyboardEvent): void => {
    if (e.key !== 'Tab') return;
    const list = focusables();
    const first = list[0];
    const last = list[list.length - 1];
    if (!first || !last) return;
    const active = document.activeElement as HTMLElement | null;
    if (e.shiftKey && active === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && active === last) {
      e.preventDefault();
      first.focus();
    }
  };

  onMount(() => {
    const root = getContainer();
    if (!root) return;
    root.addEventListener('keydown', handleKeyDown);
    const explicit = opts.initialFocus?.();
    const target = explicit ?? focusables()[0];
    target?.focus();
  });

  onCleanup(() => {
    const root = getContainer();
    root?.removeEventListener('keydown', handleKeyDown);
    if (
      previouslyFocused &&
      typeof previouslyFocused.focus === 'function' &&
      previouslyFocused.isConnected
    ) {
      previouslyFocused.focus();
    }
  });
}

function useFocusTrapMenu(
  getContainer: () => HTMLElement | undefined,
  onDismiss: (() => void) | undefined,
): void {
  if (!onDismiss) return;

  const handleKeyDown = (e: KeyboardEvent): void => {
    if (e.key !== 'Escape') return;
    e.preventDefault();
    onDismiss();
  };

  const handleOutsideMouseDown = (e: MouseEvent): void => {
    const root = getContainer();
    if (!root) return;
    const target = e.target as Node | null;
    if (target && root.contains(target)) return;
    onDismiss();
  };

  onMount(() => {
    window.addEventListener('keydown', handleKeyDown);
    document.addEventListener('mousedown', handleOutsideMouseDown);
  });

  onCleanup(() => {
    window.removeEventListener('keydown', handleKeyDown);
    document.removeEventListener('mousedown', handleOutsideMouseDown);
  });
}
