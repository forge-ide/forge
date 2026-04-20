import { describe, expect, it, vi } from 'vitest';
import { render, fireEvent } from '@solidjs/testing-library';
import {
  ContextPicker,
  CATEGORIES,
  computePopupPlacement,
  detectAtTrigger,
  type PickerResult,
} from './ContextPicker';

// ---------------------------------------------------------------------------
// Pure-function tests (F-141)
// ---------------------------------------------------------------------------
//
// `computePopupPlacement` and `detectAtTrigger` are extracted as pure
// functions because jsdom does not compute layout — the placement flip
// cannot be tested by rendering and asserting against `getBoundingClientRect`.
// Asserting against the pure function directly is the TDD contract for
// the DoD's "popup anchors intelligently (flip to above when near bottom
// viewport edge)" checkbox.

describe('computePopupPlacement (F-141)', () => {
  it('places the popup below when there is room below the anchor', () => {
    const placement = computePopupPlacement({
      anchorTop: 100,
      anchorBottom: 160,
      viewportHeight: 1080,
      popupHeight: 360,
    });
    expect(placement).toBe('below');
  });

  it('flips the popup above when the anchor is near the bottom of the viewport', () => {
    // Anchor at the bottom of a standard viewport — the composer is
    // pinned to the bottom of the chat pane in practice. A 360px popup
    // below would be clipped, so the function must flip to 'above'.
    const placement = computePopupPlacement({
      anchorTop: 700,
      anchorBottom: 780,
      viewportHeight: 800,
      popupHeight: 360,
    });
    expect(placement).toBe('above');
  });

  it('respects the gap when deciding flip', () => {
    // Space below (minus gap) is exactly the popup height — should stay below.
    const placement = computePopupPlacement({
      anchorTop: 400,
      anchorBottom: 436,
      viewportHeight: 800,
      popupHeight: 360,
      gap: 4,
    });
    expect(placement).toBe('below');
  });

  it('flips to above when the viewport is shorter than the popup + anchor bottom', () => {
    const placement = computePopupPlacement({
      anchorTop: 500,
      anchorBottom: 540,
      viewportHeight: 800,
      popupHeight: 360,
    });
    // 800 - 540 = 260 < 360 → above
    expect(placement).toBe('above');
  });
});

describe('detectAtTrigger (F-141)', () => {
  it('matches a fresh `@` at the start of the textarea', () => {
    const match = detectAtTrigger('@', 1);
    expect(match).toEqual({ start: 0, query: '' });
  });

  it('matches a `@` preceded by a space, with a query', () => {
    const text = 'hello @fo';
    const match = detectAtTrigger(text, text.length);
    expect(match).toEqual({ start: 6, query: 'fo' });
  });

  it('does not match when `@` is preceded by a non-whitespace character', () => {
    // e.g. an email address shouldn't open the picker.
    const text = 'reach me at me@example.com';
    const match = detectAtTrigger(text, text.length);
    expect(match).toBeNull();
  });

  it('does not match once the user types whitespace after the `@`', () => {
    const text = 'hello @foo ';
    const match = detectAtTrigger(text, text.length);
    expect(match).toBeNull();
  });

  it('matches at the caret inside the middle of text', () => {
    // Caret sits inside an active `@token`.
    const text = 'leading @src/foo more';
    const caret = 'leading @src/foo'.length;
    const match = detectAtTrigger(text, caret);
    expect(match).toEqual({ start: 8, query: 'src/foo' });
  });

  it('returns null when the caret is before any `@`', () => {
    const text = 'hello @foo';
    const match = detectAtTrigger(text, 3);
    expect(match).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// Component tests (F-141)
// ---------------------------------------------------------------------------

describe('ContextPicker rendering', () => {
  const anchor = { top: 100, bottom: 160, left: 0, right: 360 };

  it('renders the popup root', () => {
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    expect(getByTestId('context-picker')).toBeInTheDocument();
  });

  it('renders all seven category tabs (F-141 DoD item 5)', () => {
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    for (const cat of CATEGORIES) {
      expect(getByTestId(`context-picker-tab-${cat.id}`)).toBeInTheDocument();
    }
    expect(CATEGORIES).toHaveLength(7);
  });

  it('marks the first category (file) as active on initial render', () => {
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    const fileTab = getByTestId('context-picker-tab-file');
    expect(fileTab.getAttribute('aria-selected')).toBe('true');
  });

  it('echoes the current `@` query in the search row', () => {
    const { getByTestId } = render(() => (
      <ContextPicker
        query="src/foo"
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    expect(getByTestId('context-picker-query')).toHaveTextContent('src/foo');
  });

  it('renders an empty state for the active category when items are not provided (F-141)', () => {
    // F-141 ships the component shell with empty category tabs; F-142
    // populates the results. The empty placeholder must render so the
    // popup is not collapsed / unusable until F-142 lands.
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    expect(getByTestId('context-picker-empty')).toBeInTheDocument();
  });
});

describe('ContextPicker keyboard navigation', () => {
  const anchor = { top: 100, bottom: 160, left: 0, right: 360 };

  it('Tab moves forward through categories', () => {
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    // file is active initially
    expect(getByTestId('context-picker-tab-file').getAttribute('aria-selected'))
      .toBe('true');
    fireEvent.keyDown(window, { key: 'Tab' });
    expect(
      getByTestId('context-picker-tab-directory').getAttribute('aria-selected'),
    ).toBe('true');
    fireEvent.keyDown(window, { key: 'Tab' });
    expect(
      getByTestId('context-picker-tab-selection').getAttribute('aria-selected'),
    ).toBe('true');
  });

  it('Shift+Tab moves backward through categories', () => {
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    // From `file`, Shift+Tab wraps to the last category (`url`).
    fireEvent.keyDown(window, { key: 'Tab', shiftKey: true });
    expect(
      getByTestId('context-picker-tab-url').getAttribute('aria-selected'),
    ).toBe('true');
  });

  it('Escape fires onDismiss', () => {
    const onDismiss = vi.fn();
    render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={onDismiss}
      />
    ));
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onDismiss).toHaveBeenCalled();
  });

  it('Enter with an active item fires onPick', () => {
    const onPick = vi.fn();
    const items: PickerResult[] = [
      { category: 'file', label: 'a.ts', value: 'a.ts' },
      { category: 'file', label: 'b.ts', value: 'b.ts' },
    ];
    render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        items={{ file: items }}
        onPick={onPick}
        onDismiss={() => {}}
      />
    ));
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith({
      category: 'file',
      label: 'a.ts',
      value: 'a.ts',
    });
  });

  it('ArrowDown / ArrowUp move the result cursor', () => {
    const onPick = vi.fn();
    const items: PickerResult[] = [
      { category: 'file', label: 'a.ts', value: 'a.ts' },
      { category: 'file', label: 'b.ts', value: 'b.ts' },
      { category: 'file', label: 'c.ts', value: 'c.ts' },
    ];
    render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        items={{ file: items }}
        onPick={onPick}
        onDismiss={() => {}}
      />
    ));
    fireEvent.keyDown(window, { key: 'ArrowDown' });
    fireEvent.keyDown(window, { key: 'ArrowDown' });
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith(items[2]);
  });
});

describe('ContextPicker chip insertion flow (F-141)', () => {
  // Exercises the "selected result inserts a chip" path end-to-end through
  // the public `onPick` callback. The composer-side mutation (removing the
  // `@text` span and appending to ctx-chips) is covered by ChatPane.test.tsx;
  // here we pin that picking a result with Enter fires onPick with the
  // selected PickerResult shape the composer will turn into a chip.
  const anchor = { top: 100, bottom: 160, left: 0, right: 360 };

  it('clicking a result invokes onPick with that item', () => {
    const onPick = vi.fn();
    const items: PickerResult[] = [
      { category: 'file', label: 'alpha.ts', value: 'src/alpha.ts' },
      { category: 'file', label: 'beta.ts', value: 'src/beta.ts' },
    ];
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        items={{ file: items }}
        onPick={onPick}
        onDismiss={() => {}}
      />
    ));
    fireEvent.mouseDown(getByTestId('context-picker-result-1'));
    expect(onPick).toHaveBeenCalledWith(items[1]);
  });

  it('resets the result cursor to 0 when Tab switches category', () => {
    const onPick = vi.fn();
    const fileItems: PickerResult[] = [
      { category: 'file', label: 'a.ts', value: 'a.ts' },
      { category: 'file', label: 'b.ts', value: 'b.ts' },
    ];
    const dirItems: PickerResult[] = [
      { category: 'directory', label: 'src/', value: 'src/' },
      { category: 'directory', label: 'tests/', value: 'tests/' },
    ];
    render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        items={{ file: fileItems, directory: dirItems }}
        onPick={onPick}
        onDismiss={() => {}}
      />
    ));
    // Move to file[1], then Tab to directory — cursor should reset to 0.
    fireEvent.keyDown(window, { key: 'ArrowDown' });
    fireEvent.keyDown(window, { key: 'Tab' });
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(onPick).toHaveBeenCalledWith(dirItems[0]);
  });
});

describe('ContextPicker placement data attribute', () => {
  it('flags placement=above when the anchor sits near the viewport bottom', () => {
    // jsdom's default innerHeight is 768. Put the anchor bottom at 700 →
    // 68px of space → under the 360px popup → should flip to above.
    const anchor = { top: 640, bottom: 700, left: 0, right: 360 };
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    const root = getByTestId('context-picker');
    expect(root.getAttribute('data-placement')).toBe('above');
  });

  it('flags placement=below when the anchor sits near the viewport top', () => {
    const anchor = { top: 50, bottom: 110, left: 0, right: 360 };
    const { getByTestId } = render(() => (
      <ContextPicker
        query=""
        anchorRect={anchor}
        onPick={() => {}}
        onDismiss={() => {}}
      />
    ));
    const root = getByTestId('context-picker');
    expect(root.getAttribute('data-placement')).toBe('below');
  });
});
