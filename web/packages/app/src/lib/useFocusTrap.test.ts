// F-402: Reusable focus-trap hook for role="dialog" surfaces.
//
// Contract (WAI-ARIA APG dialog pattern):
//   - On activation, move focus into the container (explicit initialFocus
//     element, or the first focusable descendant if none given).
//   - While active, Tab at the last focusable cycles to the first; Shift+Tab
//     at the first cycles to the last.
//   - On deactivation, restore focus to the element that had it before
//     activation, when that element is still connected.
//
// These tests drive the hook directly by exercising createRoot scopes so
// the onMount/onCleanup lifecycle is observable without mounting a real
// component.

import { describe, expect, it, beforeEach, vi } from 'vitest';
import { createRoot } from 'solid-js';
import { useFocusTrap } from './useFocusTrap';

function makeContainer(buttons: string[]): HTMLDivElement {
  const div = document.createElement('div');
  for (const label of buttons) {
    const btn = document.createElement('button');
    btn.textContent = label;
    btn.setAttribute('data-label', label);
    div.appendChild(btn);
  }
  document.body.appendChild(div);
  return div;
}

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('useFocusTrap — initial focus', () => {
  it('focuses the first focusable descendant by default on mount', async () => {
    const container = makeContainer(['one', 'two', 'three']);
    createRoot(() => {
      useFocusTrap(() => container);
    });
    // onMount runs synchronously for Solid under jsdom in tests.
    expect(document.activeElement).toBe(container.querySelector('[data-label="one"]'));
  });

  it('focuses the element returned by opts.initialFocus when provided', () => {
    const container = makeContainer(['one', 'two', 'three']);
    const target = container.querySelector<HTMLButtonElement>('[data-label="two"]')!;
    createRoot(() => {
      useFocusTrap(() => container, { initialFocus: () => target });
    });
    expect(document.activeElement).toBe(target);
  });
});

describe('useFocusTrap — Tab cycling', () => {
  it('Tab at the last focusable cycles focus to the first', () => {
    const container = makeContainer(['one', 'two', 'three']);
    createRoot(() => {
      useFocusTrap(() => container);
    });
    const last = container.querySelector<HTMLButtonElement>('[data-label="three"]')!;
    last.focus();
    const ev = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    last.dispatchEvent(ev);
    expect(document.activeElement).toBe(container.querySelector('[data-label="one"]'));
    expect(ev.defaultPrevented).toBe(true);
  });

  it('Shift+Tab at the first focusable cycles focus to the last', () => {
    const container = makeContainer(['one', 'two', 'three']);
    createRoot(() => {
      useFocusTrap(() => container);
    });
    const first = container.querySelector<HTMLButtonElement>('[data-label="one"]')!;
    first.focus();
    const ev = new KeyboardEvent('keydown', {
      key: 'Tab',
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    first.dispatchEvent(ev);
    expect(document.activeElement).toBe(container.querySelector('[data-label="three"]'));
    expect(ev.defaultPrevented).toBe(true);
  });

  it('Tab in the middle of the sequence does not intercept — browser default', () => {
    const container = makeContainer(['one', 'two', 'three']);
    createRoot(() => {
      useFocusTrap(() => container);
    });
    const middle = container.querySelector<HTMLButtonElement>('[data-label="two"]')!;
    middle.focus();
    const ev = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    middle.dispatchEvent(ev);
    expect(ev.defaultPrevented).toBe(false);
  });
});

describe('useFocusTrap — focus restore', () => {
  it('restores focus to the element active before the hook ran, on cleanup', () => {
    const outside = document.createElement('button');
    outside.textContent = 'outside';
    document.body.appendChild(outside);
    outside.focus();
    expect(document.activeElement).toBe(outside);

    const container = makeContainer(['one', 'two']);
    const dispose = createRoot((disposeFn) => {
      useFocusTrap(() => container);
      return disposeFn;
    });

    // Trap moved focus into the container.
    expect(document.activeElement).toBe(container.querySelector('[data-label="one"]'));

    dispose();
    expect(document.activeElement).toBe(outside);
  });

  it('does not throw when the previously-focused element has been removed from the DOM', () => {
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    outside.focus();

    const container = makeContainer(['only']);
    const dispose = createRoot((disposeFn) => {
      useFocusTrap(() => container);
      return disposeFn;
    });

    outside.remove();
    expect(() => dispose()).not.toThrow();
  });
});

// F-402-followup: menu path — when the caller is a non-modal popover/menu,
// opt out of focus-trap semantics by passing `trap: false` and wire dismissal
// via `onDismiss`. The hook then owns window-level Esc + outside-click
// handlers, so the host component no longer duplicates that wiring inline.
describe('useFocusTrap — menu path (trap: false + onDismiss)', () => {
  it('does not move focus into the container when trap is false', () => {
    const outside = document.createElement('button');
    outside.textContent = 'outside';
    document.body.appendChild(outside);
    outside.focus();

    const container = makeContainer(['one', 'two']);
    createRoot(() => {
      useFocusTrap(() => container, { trap: false, onDismiss: () => {} });
    });

    // Focus stayed on the trigger; the menu path never steals focus.
    expect(document.activeElement).toBe(outside);
  });

  it('fires onDismiss on window-level Escape even when popover is not focused', () => {
    // Regression: with `trap: false`, Esc must fire regardless of where focus
    // lives. The host should not need to force-focus the container on open.
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    outside.focus();

    const container = makeContainer(['one']);
    const onDismiss = vi.fn();
    createRoot(() => {
      useFocusTrap(() => container, { trap: false, onDismiss });
    });

    expect(document.activeElement).toBe(outside);
    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('fires onDismiss on mousedown outside the container', () => {
    const container = makeContainer(['one']);
    const onDismiss = vi.fn();
    createRoot(() => {
      useFocusTrap(() => container, { trap: false, onDismiss });
    });

    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });

  it('does NOT fire onDismiss on mousedown inside the container', () => {
    const container = makeContainer(['one']);
    const onDismiss = vi.fn();
    createRoot(() => {
      useFocusTrap(() => container, { trap: false, onDismiss });
    });

    const inside = container.querySelector<HTMLButtonElement>('[data-label="one"]')!;
    inside.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it('removes window-level listeners on cleanup', () => {
    const container = makeContainer(['one']);
    const onDismiss = vi.fn();
    const dispose = createRoot((disposeFn) => {
      useFocusTrap(() => container, { trap: false, onDismiss });
      return disposeFn;
    });

    dispose();

    window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }));
    document.body.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
    expect(onDismiss).not.toHaveBeenCalled();
  });

  it('does not install Tab-cycling when trap is false', () => {
    // The menu path never intercepts Tab — focus should behave exactly as
    // the browser dictates for whatever happens to be focused.
    const outside = document.createElement('button');
    outside.textContent = 'outside';
    document.body.appendChild(outside);
    const container = makeContainer(['one']);
    createRoot(() => {
      useFocusTrap(() => container, { trap: false, onDismiss: () => {} });
    });

    const only = container.querySelector<HTMLButtonElement>('[data-label="one"]')!;
    only.focus();
    const ev = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    only.dispatchEvent(ev);
    // In modal/trap mode this would have been prevented (one focusable = first=last);
    // in menu mode the hook never touches Tab.
    expect(ev.defaultPrevented).toBe(false);
  });
});
