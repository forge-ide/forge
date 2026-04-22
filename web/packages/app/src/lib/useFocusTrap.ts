// F-402: focus-trap hook for role="dialog" surfaces.
//
// Implements the WAI-ARIA APG dialog contract for any surface that declares
// role="dialog" / role="alertdialog":
//
//   - Moves focus into the container on activation (explicit target via
//     opts.initialFocus, else the first focusable descendant).
//   - Cycles Tab / Shift+Tab at the boundaries so keyboard focus cannot
//     escape the dialog.
//   - Restores focus to whatever element was active before activation on
//     cleanup, when that element is still attached.
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
   * from this thunk falls back to that default.
   */
  initialFocus?: () => HTMLElement | undefined;
}

export function useFocusTrap(
  getContainer: () => HTMLElement | undefined,
  opts: UseFocusTrapOptions = {},
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
