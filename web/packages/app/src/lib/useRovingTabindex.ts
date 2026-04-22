// F-416: WAI-ARIA roving-tabindex hook.
//
// Implements the WAI-ARIA APG pattern for a group of related items (grid,
// toolbar, listbox-like surfaces) where keyboard users should Tab into the
// group once, then navigate within it with arrow keys:
//
//   - Exactly one item has tabindex="0" at any time; the rest are
//     tabindex="-1". Tab enters and exits the group as a single stop.
//   - ArrowRight / ArrowDown advance the tab stop; ArrowLeft / ArrowUp go
//     back; both wrap at the boundaries. Home / End jump to first / last.
//   - A pointer focus (mouse click) on any item makes it the new tab stop.
//
// The hook queries items on every interaction so appending / removing items
// after mount stays consistent without a MutationObserver. The container
// accessor is evaluated reactively — a component can defer ref assignment
// behind a <Show> gate and the hook will attach as soon as the element
// appears in the DOM.

import { createEffect, onCleanup } from 'solid-js';

const NAV_KEYS = new Set([
  'ArrowRight',
  'ArrowDown',
  'ArrowLeft',
  'ArrowUp',
  'Home',
  'End',
]);

export function useRovingTabindex(
  getContainer: () => HTMLElement | undefined,
  itemSelector: string,
): void {
  let attached: HTMLElement | null = null;

  const itemsOf = (root: HTMLElement): HTMLElement[] =>
    Array.from(root.querySelectorAll<HTMLElement>(itemSelector));

  const sync = (root: HTMLElement, activeIndex: number): void => {
    const list = itemsOf(root);
    list.forEach((el, i) => {
      el.setAttribute('tabindex', i === activeIndex ? '0' : '-1');
    });
  };

  const indexOf = (root: HTMLElement, target: EventTarget | null): number => {
    if (!(target instanceof HTMLElement)) return -1;
    return itemsOf(root).indexOf(target);
  };

  const handleKeyDown = (e: KeyboardEvent): void => {
    if (!NAV_KEYS.has(e.key)) return;
    const root = attached;
    if (!root) return;
    const list = itemsOf(root);
    if (list.length === 0) return;
    const current = indexOf(root, e.target);
    if (current === -1) return;

    let next = current;
    switch (e.key) {
      case 'ArrowRight':
      case 'ArrowDown':
        next = (current + 1) % list.length;
        break;
      case 'ArrowLeft':
      case 'ArrowUp':
        next = (current - 1 + list.length) % list.length;
        break;
      case 'Home':
        next = 0;
        break;
      case 'End':
        next = list.length - 1;
        break;
    }
    if (next === current) return;
    e.preventDefault();
    sync(root, next);
    list[next]?.focus();
  };

  const handleFocusIn = (e: FocusEvent): void => {
    const root = attached;
    if (!root) return;
    const idx = indexOf(root, e.target);
    if (idx === -1) return;
    sync(root, idx);
  };

  const detach = (): void => {
    if (!attached) return;
    attached.removeEventListener('keydown', handleKeyDown);
    attached.removeEventListener('focusin', handleFocusIn);
    attached = null;
  };

  createEffect(() => {
    const root = getContainer();
    if (root === attached) return;
    detach();
    if (!root) return;
    attached = root;
    if (itemsOf(root).length > 0) sync(root, 0);
    root.addEventListener('keydown', handleKeyDown);
    root.addEventListener('focusin', handleFocusIn);
  });

  onCleanup(detach);
}
