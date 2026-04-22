// F-416: WAI-ARIA roving-tabindex hook.
//
// Contract (WAI-ARIA APG grid / toolbar pattern):
//   - Exactly one descendant matching the item selector has tabindex="0"
//     at any time; the rest are tabindex="-1" so Tab enters the container
//     once and exits once.
//   - ArrowRight / ArrowDown move the active index forward (wrap); ArrowLeft /
//     ArrowUp move it backward (wrap). Home jumps to first, End to last.
//   - Clicking (or focusing via a pointer) an item makes it the new active
//     tab stop.
//   - Adding or removing items re-syncs the single-tab-stop invariant.

import { describe, expect, it, beforeEach } from 'vitest';
import { createRoot } from 'solid-js';
import { useRovingTabindex } from './useRovingTabindex';

function makeGrid(labels: string[]): HTMLDivElement {
  const div = document.createElement('div');
  for (const label of labels) {
    const btn = document.createElement('button');
    btn.textContent = label;
    btn.setAttribute('data-label', label);
    btn.setAttribute('data-roving-item', '');
    div.appendChild(btn);
  }
  document.body.appendChild(div);
  return div;
}

function dispatchKey(target: HTMLElement, key: string): KeyboardEvent {
  const ev = new KeyboardEvent('keydown', {
    key,
    bubbles: true,
    cancelable: true,
  });
  target.dispatchEvent(ev);
  return ev;
}

beforeEach(() => {
  document.body.innerHTML = '';
});

describe('useRovingTabindex — initial tab-stop', () => {
  it('sets tabindex=0 on the first item and tabindex=-1 on the rest on mount', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const items = grid.querySelectorAll('[data-roving-item]');
    expect(items[0]?.getAttribute('tabindex')).toBe('0');
    expect(items[1]?.getAttribute('tabindex')).toBe('-1');
    expect(items[2]?.getAttribute('tabindex')).toBe('-1');
  });

  it('is a no-op when the container has no matching items', () => {
    const grid = document.createElement('div');
    document.body.appendChild(grid);
    expect(() => {
      createRoot(() => {
        useRovingTabindex(() => grid, '[data-roving-item]');
      });
    }).not.toThrow();
  });
});

describe('useRovingTabindex — arrow navigation', () => {
  it('ArrowRight from the first item focuses the second and moves the tab stop', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const first = grid.querySelector<HTMLButtonElement>('[data-label="a"]')!;
    const second = grid.querySelector<HTMLButtonElement>('[data-label="b"]')!;
    first.focus();
    const ev = dispatchKey(first, 'ArrowRight');
    expect(document.activeElement).toBe(second);
    expect(first.getAttribute('tabindex')).toBe('-1');
    expect(second.getAttribute('tabindex')).toBe('0');
    expect(ev.defaultPrevented).toBe(true);
  });

  it('ArrowDown behaves the same as ArrowRight (2-D wrap grid)', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const first = grid.querySelector<HTMLButtonElement>('[data-label="a"]')!;
    const second = grid.querySelector<HTMLButtonElement>('[data-label="b"]')!;
    first.focus();
    dispatchKey(first, 'ArrowDown');
    expect(document.activeElement).toBe(second);
  });

  it('ArrowLeft from the first item wraps to the last', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const first = grid.querySelector<HTMLButtonElement>('[data-label="a"]')!;
    const last = grid.querySelector<HTMLButtonElement>('[data-label="c"]')!;
    first.focus();
    dispatchKey(first, 'ArrowLeft');
    expect(document.activeElement).toBe(last);
    expect(last.getAttribute('tabindex')).toBe('0');
  });

  it('ArrowRight from the last item wraps to the first', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const first = grid.querySelector<HTMLButtonElement>('[data-label="a"]')!;
    const last = grid.querySelector<HTMLButtonElement>('[data-label="c"]')!;
    last.focus();
    dispatchKey(last, 'ArrowRight');
    expect(document.activeElement).toBe(first);
  });
});

describe('useRovingTabindex — Home and End', () => {
  it('Home jumps focus to the first item', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const middle = grid.querySelector<HTMLButtonElement>('[data-label="b"]')!;
    const first = grid.querySelector<HTMLButtonElement>('[data-label="a"]')!;
    middle.focus();
    const ev = dispatchKey(middle, 'Home');
    expect(document.activeElement).toBe(first);
    expect(first.getAttribute('tabindex')).toBe('0');
    expect(ev.defaultPrevented).toBe(true);
  });

  it('End jumps focus to the last item', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const middle = grid.querySelector<HTMLButtonElement>('[data-label="b"]')!;
    const last = grid.querySelector<HTMLButtonElement>('[data-label="c"]')!;
    middle.focus();
    dispatchKey(middle, 'End');
    expect(document.activeElement).toBe(last);
    expect(last.getAttribute('tabindex')).toBe('0');
  });
});

describe('useRovingTabindex — focus adoption', () => {
  it('focusing an item via pointer makes it the new tab stop', () => {
    const grid = makeGrid(['a', 'b', 'c']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    const first = grid.querySelector<HTMLButtonElement>('[data-label="a"]')!;
    const third = grid.querySelector<HTMLButtonElement>('[data-label="c"]')!;
    // Simulate the focus-by-click path (jsdom does not fire focus on click
    // automatically for buttons without focusing them first).
    third.focus();
    third.dispatchEvent(new FocusEvent('focus', { bubbles: true }));
    expect(first.getAttribute('tabindex')).toBe('-1');
    expect(third.getAttribute('tabindex')).toBe('0');
  });
});

describe('useRovingTabindex — dynamic item sets', () => {
  it('re-syncs the tab stop when items are added after mount', () => {
    const grid = makeGrid(['a']);
    createRoot(() => {
      useRovingTabindex(() => grid, '[data-roving-item]');
    });
    // Append a new item; the hook must keep exactly one tabindex=0 item.
    const added = document.createElement('button');
    added.setAttribute('data-label', 'b');
    added.setAttribute('data-roving-item', '');
    grid.appendChild(added);
    // Drive a keypress so the hook recomputes (MutationObserver would be
    // async; we accept an explicit recompute path on next interaction).
    const first = grid.querySelector<HTMLButtonElement>('[data-label="a"]')!;
    first.focus();
    dispatchKey(first, 'ArrowRight');
    const tabStops = grid.querySelectorAll('[data-roving-item][tabindex="0"]');
    expect(tabStops).toHaveLength(1);
    expect(document.activeElement).toBe(added);
  });
});
