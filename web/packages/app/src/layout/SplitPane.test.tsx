import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, fireEvent } from '@solidjs/testing-library';
import { createSignal } from 'solid-js';
import {
  SplitPane,
  MIN_RATIO,
  RESET_RATIO,
  SNAP_PX,
  KEYBOARD_STEP,
  KEYBOARD_SHIFT_MULTIPLIER,
} from './SplitPane';

// jsdom returns {0,0,0,0} for getBoundingClientRect by default. The divider
// drag math multiplies pointer delta by container extent, so we stub a
// deterministic rect on every SplitPane root at render time. This runs at
// the Element prototype level to avoid having to wire a ref through.
function stubBoundingRect({ width, height }: { width: number; height: number }): () => void {
  const original = Element.prototype.getBoundingClientRect;
  Element.prototype.getBoundingClientRect = function stubbed(this: Element): DOMRect {
    if (this instanceof HTMLElement && this.classList.contains('split-pane')) {
      return {
        x: 0,
        y: 0,
        top: 0,
        left: 0,
        right: width,
        bottom: height,
        width,
        height,
        toJSON() {
          return this;
        },
      } as DOMRect;
    }
    return original.call(this);
  };
  return () => {
    Element.prototype.getBoundingClientRect = original;
  };
}

// Minimal pointer-event dispatch for jsdom, which does not implement the
// constructor out of the box. Using a PointerEvent-shaped MouseEvent is fine
// for Solid's synthetic handlers — they read the familiar fields.
function firePointer(
  el: Element,
  type: 'pointerdown' | 'pointermove' | 'pointerup',
  init: { clientX: number; clientY: number; altKey?: boolean; pointerId?: number; button?: number },
) {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    clientX: init.clientX,
    clientY: init.clientY,
    altKey: init.altKey ?? false,
    button: init.button ?? 0,
  });
  // Augment with pointerId + capture methods so setPointerCapture / has / release
  // resolve without throwing under jsdom.
  Object.defineProperty(event, 'pointerId', { value: init.pointerId ?? 1 });
  if (!(el as HTMLElement).setPointerCapture) {
    (el as HTMLElement).setPointerCapture = vi.fn();
    (el as HTMLElement).releasePointerCapture = vi.fn();
    (el as HTMLElement).hasPointerCapture = vi.fn(() => true);
  }
  fireEvent(el, event);
}

function Harness(initial: {
  direction: 'h' | 'v';
  ratio: number;
  onChange?: (r: number) => void;
}) {
  const [r, setR] = createSignal(initial.ratio);
  return {
    component: () => (
      <SplitPane
        direction={initial.direction}
        ratio={r()}
        onRatioChange={(next) => {
          initial.onChange?.(next);
          setR(next);
        }}
      >
        <div data-testid="first">first</div>
        <div data-testid="second">second</div>
      </SplitPane>
    ),
    ratio: r,
  };
}

afterEach(() => {
  cleanup();
});

describe('SplitPane — direction', () => {
  it('vertical split lays children side by side', () => {
    const h = Harness({ direction: 'v', ratio: 0.5 });
    const { getByTestId } = render(h.component);
    const root = getByTestId('split-pane');
    expect(root.getAttribute('data-direction')).toBe('v');
    const first = root.querySelector('.split-pane__child--first') as HTMLElement;
    const second = root.querySelector('.split-pane__child--second') as HTMLElement;
    expect(first.style.width).toBe('50%');
    expect(second.style.width).toBe('50%');
    // Height should NOT be set on children in vertical split — flex row.
    expect(first.style.height).toBe('');
  });

  it('horizontal split stacks children top to bottom', () => {
    const h = Harness({ direction: 'h', ratio: 0.3 });
    const { getByTestId } = render(h.component);
    const root = getByTestId('split-pane');
    expect(root.getAttribute('data-direction')).toBe('h');
    const first = root.querySelector('.split-pane__child--first') as HTMLElement;
    const second = root.querySelector('.split-pane__child--second') as HTMLElement;
    expect(first.style.height).toBe('30%');
    expect(second.style.height).toBe('70%');
    expect(first.style.width).toBe('');
  });
});

describe('SplitPane — divider hit area & cursor', () => {
  it('vertical divider has a ≥8px hit area and a vertical separator role', () => {
    const h = Harness({ direction: 'v', ratio: 0.5 });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    expect(divider.getAttribute('role')).toBe('separator');
    expect(divider.getAttribute('aria-orientation')).toBe('vertical');
    // Width is driven by CSS (`.split-pane--v > .split-pane__divider { width: 8px }`).
    // jsdom doesn't apply the stylesheet, so we assert the class wiring
    // that makes the 8px rule match. Visual width is covered by CSS itself.
    expect(divider.parentElement?.classList.contains('split-pane--v')).toBe(true);
  });

  it('horizontal divider exposes horizontal orientation for AT', () => {
    const h = Harness({ direction: 'h', ratio: 0.5 });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    expect(divider.getAttribute('aria-orientation')).toBe('horizontal');
    expect(divider.parentElement?.classList.contains('split-pane--h')).toBe(true);
  });
});

describe('SplitPane — resize drag (vertical)', () => {
  it('drags the divider and emits the new ratio', () => {
    const restore = stubBoundingRect({ width: 1000, height: 600 });
    try {
      const onChange = vi.fn();
      const h = Harness({ direction: 'v', ratio: 0.5, onChange });
      const { getByTestId } = render(h.component);
      const divider = getByTestId('split-pane-divider');

      // Drag from the 500px mark to the 700px mark → ratio ≈ 0.7 (snapped to 4px).
      firePointer(divider, 'pointerdown', { clientX: 500, clientY: 300 });
      firePointer(divider, 'pointermove', { clientX: 700, clientY: 300 });
      firePointer(divider, 'pointerup', { clientX: 700, clientY: 300 });

      expect(onChange).toHaveBeenCalled();
      const last = onChange.mock.calls.at(-1)?.[0] as number;
      // 700 / 1000 = 0.7, and 700 is already a 4px multiple → exactly 0.7.
      expect(last).toBeCloseTo(0.7, 5);
    } finally {
      restore();
    }
  });

  it('does not emit ratio changes on pointermove without pointerdown', () => {
    const restore = stubBoundingRect({ width: 1000, height: 600 });
    try {
      const onChange = vi.fn();
      const h = Harness({ direction: 'v', ratio: 0.5, onChange });
      const { getByTestId } = render(h.component);
      const divider = getByTestId('split-pane-divider');
      firePointer(divider, 'pointermove', { clientX: 800, clientY: 300 });
      expect(onChange).not.toHaveBeenCalled();
    } finally {
      restore();
    }
  });
});

describe('SplitPane — resize drag (horizontal)', () => {
  it('uses the Y axis for ratio when direction is "h"', () => {
    const restore = stubBoundingRect({ width: 1000, height: 600 });
    try {
      const onChange = vi.fn();
      const h = Harness({ direction: 'h', ratio: 0.5, onChange });
      const { getByTestId } = render(h.component);
      const divider = getByTestId('split-pane-divider');
      firePointer(divider, 'pointerdown', { clientX: 500, clientY: 300 });
      firePointer(divider, 'pointermove', { clientX: 500, clientY: 180 });
      firePointer(divider, 'pointerup', { clientX: 500, clientY: 180 });
      const last = onChange.mock.calls.at(-1)?.[0] as number;
      // 180 / 600 = 0.3, 180 is a 4px multiple.
      expect(last).toBeCloseTo(0.3, 5);
    } finally {
      restore();
    }
  });
});

describe('SplitPane — clamp to MIN_RATIO', () => {
  it('clamps to MIN_RATIO when dragging below the floor', () => {
    const restore = stubBoundingRect({ width: 1000, height: 600 });
    try {
      const onChange = vi.fn();
      const h = Harness({ direction: 'v', ratio: 0.5, onChange });
      const { getByTestId } = render(h.component);
      const divider = getByTestId('split-pane-divider');
      firePointer(divider, 'pointerdown', { clientX: 500, clientY: 300 });
      // Well below the MIN_RATIO = 0.1 floor (50 / 1000 = 0.05).
      firePointer(divider, 'pointermove', { clientX: 50, clientY: 300 });
      firePointer(divider, 'pointerup', { clientX: 50, clientY: 300 });
      const last = onChange.mock.calls.at(-1)?.[0] as number;
      expect(last).toBeCloseTo(MIN_RATIO, 5);
      expect(last).toBeGreaterThan(0);
    } finally {
      restore();
    }
  });

  it('clamps to 1 - MIN_RATIO when dragging past the ceiling', () => {
    const restore = stubBoundingRect({ width: 1000, height: 600 });
    try {
      const onChange = vi.fn();
      const h = Harness({ direction: 'v', ratio: 0.5, onChange });
      const { getByTestId } = render(h.component);
      const divider = getByTestId('split-pane-divider');
      firePointer(divider, 'pointerdown', { clientX: 500, clientY: 300 });
      firePointer(divider, 'pointermove', { clientX: 980, clientY: 300 });
      firePointer(divider, 'pointerup', { clientX: 980, clientY: 300 });
      const last = onChange.mock.calls.at(-1)?.[0] as number;
      expect(last).toBeCloseTo(1 - MIN_RATIO, 5);
      expect(last).toBeLessThan(1);
    } finally {
      restore();
    }
  });

  it('clamps the rendered size for out-of-range controlled ratios', () => {
    // Negative ratio should never produce a negative width.
    const h1 = Harness({ direction: 'v', ratio: -0.3 });
    const { getByTestId: get1, unmount: unmount1 } = render(h1.component);
    const root1 = get1('split-pane');
    const first1 = root1.querySelector('.split-pane__child--first') as HTMLElement;
    expect(first1.style.width).toBe(`${MIN_RATIO * 100}%`);
    unmount1();

    // Over-1.0 should not produce a >100% width.
    const h2 = Harness({ direction: 'v', ratio: 1.5 });
    const { getByTestId: get2 } = render(h2.component);
    const root2 = get2('split-pane');
    const first2 = root2.querySelector('.split-pane__child--first') as HTMLElement;
    expect(first2.style.width).toBe(`${(1 - MIN_RATIO) * 100}%`);
  });
});

describe('SplitPane — 4px snap & Alt-unsnapped (§3.1)', () => {
  it('snaps drag to 4px increments by default', () => {
    const restore = stubBoundingRect({ width: 1000, height: 600 });
    try {
      const onChange = vi.fn();
      const h = Harness({ direction: 'v', ratio: 0.5, onChange });
      const { getByTestId } = render(h.component);
      const divider = getByTestId('split-pane-divider');
      firePointer(divider, 'pointerdown', { clientX: 500, clientY: 300 });
      // 503px is between snaps — should land on 504 → 0.504.
      firePointer(divider, 'pointermove', { clientX: 503, clientY: 300 });
      firePointer(divider, 'pointerup', { clientX: 503, clientY: 300 });
      const last = onChange.mock.calls.at(-1)?.[0] as number;
      const asPixels = last * 1000;
      expect(asPixels % SNAP_PX).toBeCloseTo(0, 5);
    } finally {
      restore();
    }
  });

  it('skips the snap when Alt is held', () => {
    const restore = stubBoundingRect({ width: 1000, height: 600 });
    try {
      const onChange = vi.fn();
      const h = Harness({ direction: 'v', ratio: 0.5, onChange });
      const { getByTestId } = render(h.component);
      const divider = getByTestId('split-pane-divider');
      firePointer(divider, 'pointerdown', { clientX: 500, clientY: 300 });
      firePointer(divider, 'pointermove', { clientX: 503, clientY: 300, altKey: true });
      firePointer(divider, 'pointerup', { clientX: 503, clientY: 300, altKey: true });
      const last = onChange.mock.calls.at(-1)?.[0] as number;
      expect(last).toBeCloseTo(0.503, 5);
    } finally {
      restore();
    }
  });
});

describe('SplitPane — double-click reset (§3.1)', () => {
  it('resets to a balanced ratio when the divider is double-clicked', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.22, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.dblClick(divider);
    expect(onChange).toHaveBeenCalledWith(RESET_RATIO);
  });
});

describe('SplitPane — ARIA value state (F-404)', () => {
  it('exposes aria-valuenow/min/max reflecting the current ratio as a percentage', () => {
    const h = Harness({ direction: 'v', ratio: 0.42 });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    expect(divider.getAttribute('aria-valuenow')).toBe('42');
    expect(divider.getAttribute('aria-valuemin')).toBe(String(Math.round(MIN_RATIO * 100)));
    expect(divider.getAttribute('aria-valuemax')).toBe(String(Math.round((1 - MIN_RATIO) * 100)));
  });

  it('clamps aria-valuenow into [valuemin, valuemax] for out-of-range ratios', () => {
    const h = Harness({ direction: 'v', ratio: -0.3 });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    expect(divider.getAttribute('aria-valuenow')).toBe(String(Math.round(MIN_RATIO * 100)));
  });
});

describe('SplitPane — keyboard resize (F-404, ARIA APG splitter)', () => {
  it('ArrowRight nudges the ratio up by KEYBOARD_STEP on a vertical divider', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'ArrowRight' });
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onChange.mock.calls.at(-1)?.[0] as number).toBeCloseTo(0.5 + KEYBOARD_STEP, 5);
  });

  it('ArrowLeft nudges the ratio down by KEYBOARD_STEP', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'ArrowLeft' });
    expect(onChange.mock.calls.at(-1)?.[0] as number).toBeCloseTo(0.5 - KEYBOARD_STEP, 5);
  });

  it('ArrowDown nudges the ratio up on a horizontal divider', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'h', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'ArrowDown' });
    expect(onChange.mock.calls.at(-1)?.[0] as number).toBeCloseTo(0.5 + KEYBOARD_STEP, 5);
  });

  it('ArrowUp nudges the ratio down on a horizontal divider', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'h', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'ArrowUp' });
    expect(onChange.mock.calls.at(-1)?.[0] as number).toBeCloseTo(0.5 - KEYBOARD_STEP, 5);
  });

  it('Shift+Arrow multiplies the step by KEYBOARD_SHIFT_MULTIPLIER', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'ArrowRight', shiftKey: true });
    expect(onChange.mock.calls.at(-1)?.[0] as number).toBeCloseTo(
      0.5 + KEYBOARD_STEP * KEYBOARD_SHIFT_MULTIPLIER,
      5,
    );
  });

  it('Home snaps to MIN_RATIO', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'Home' });
    expect(onChange).toHaveBeenCalledWith(MIN_RATIO);
  });

  it('End snaps to 1 - MIN_RATIO', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'End' });
    expect(onChange).toHaveBeenCalledWith(1 - MIN_RATIO);
  });

  it('Enter resets to RESET_RATIO', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.22, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'Enter' });
    expect(onChange).toHaveBeenCalledWith(RESET_RATIO);
  });

  it('clamps ArrowLeft against MIN_RATIO at the low boundary', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: MIN_RATIO, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'ArrowLeft' });
    expect(onChange.mock.calls.at(-1)?.[0] as number).toBeCloseTo(MIN_RATIO, 5);
  });

  it('clamps ArrowRight against 1 - MIN_RATIO at the high boundary', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 1 - MIN_RATIO, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'ArrowRight' });
    expect(onChange.mock.calls.at(-1)?.[0] as number).toBeCloseTo(1 - MIN_RATIO, 5);
  });

  it('ignores unrelated keys (does not emit a ratio change)', () => {
    const onChange = vi.fn();
    const h = Harness({ direction: 'v', ratio: 0.5, onChange });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'a' });
    fireEvent.keyDown(divider, { key: 'Tab' });
    fireEvent.keyDown(divider, { key: 'Escape' });
    expect(onChange).not.toHaveBeenCalled();
  });

  it('calls preventDefault on handled keys to stop page scroll', () => {
    const h = Harness({ direction: 'v', ratio: 0.5 });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    for (const key of ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'Enter']) {
      const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true });
      fireEvent(divider, event);
      expect(event.defaultPrevented, `expected preventDefault() on "${key}"`).toBe(true);
    }
  });

  it('updates aria-valuenow after a keyboard-driven resize', () => {
    const h = Harness({ direction: 'v', ratio: 0.5 });
    const { getByTestId } = render(h.component);
    const divider = getByTestId('split-pane-divider');
    fireEvent.keyDown(divider, { key: 'Home' });
    expect(divider.getAttribute('aria-valuenow')).toBe(String(Math.round(MIN_RATIO * 100)));
  });
});
