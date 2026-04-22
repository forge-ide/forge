// F-157: CommandPalette component tests.
//
// Covers the DoD:
//   - keyboard-invoked (Cmd/Ctrl+K and Cmd/Ctrl+Shift+P both open it)
//   - selecting a command calls its `run`
//   - Escape closes
//   - fuzzy search filters the list as the user types
//   - the F-153 bonus: palette → type "agent" → select → navigate('/agents')
//
// We assert behaviour through the public props (`isOpen` uncontrolled) and
// test IDs. The palette listens on `window.keydown` in capture phase so the
// shortcut works regardless of focus target; tests dispatch the event on
// `window` directly to match that wiring.

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { MemoryRouter, Route, createMemoryHistory } from '@solidjs/router';
import { CommandPalette } from './CommandPalette';
import {
  __resetRegistryForTests,
  registerCommand,
} from './registry';

afterEach(() => {
  cleanup();
  __resetRegistryForTests();
});

function ctrlKey(key: string, shift = false): KeyboardEventInit {
  return { key, ctrlKey: true, metaKey: false, shiftKey: shift, bubbles: true };
}

function openWithCtrlK() {
  const ev = new KeyboardEvent('keydown', ctrlKey('k'));
  window.dispatchEvent(ev);
}

function openWithCtrlShiftP() {
  const ev = new KeyboardEvent('keydown', ctrlKey('P', true));
  window.dispatchEvent(ev);
}

describe('CommandPalette open/close (F-157)', () => {
  it('is closed by default — no palette in the DOM', () => {
    const { queryByTestId } = render(() => <CommandPalette />);
    expect(queryByTestId('command-palette')).toBeNull();
  });

  it('opens on Cmd/Ctrl+K', async () => {
    const { findByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const root = await findByTestId('command-palette');
    expect(root).toBeInTheDocument();
  });

  it('opens on Cmd/Ctrl+Shift+P as an alternate shortcut', async () => {
    const { findByTestId } = render(() => <CommandPalette />);
    openWithCtrlShiftP();
    const root = await findByTestId('command-palette');
    expect(root).toBeInTheDocument();
  });

  it('closes on Escape', async () => {
    const { findByTestId, queryByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(queryByTestId('command-palette')).toBeNull();
  });

  it('closes on backdrop click', async () => {
    const { findByTestId, queryByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const backdrop = await findByTestId('command-palette-backdrop');
    fireEvent.click(backdrop);
    expect(queryByTestId('command-palette')).toBeNull();
  });

  it('pressing the shortcut while open toggles the palette closed', async () => {
    const { findByTestId, queryByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    await findByTestId('command-palette');
    // Second Ctrl+K toggles off.
    openWithCtrlK();
    expect(queryByTestId('command-palette')).toBeNull();
  });
});

describe('CommandPalette entries + selection (F-157)', () => {
  it('renders registered commands as a list', async () => {
    registerCommand({ id: 'a', title: 'Alpha Action', run: vi.fn() });
    registerCommand({ id: 'b', title: 'Bravo Action', run: vi.fn() });
    const { findByTestId, findAllByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    await findByTestId('command-palette');
    const items = await findAllByTestId('command-palette-item');
    expect(items).toHaveLength(2);
    expect(items[0]!.textContent).toContain('Alpha Action');
    expect(items[1]!.textContent).toContain('Bravo Action');
  });

  it('typing filters the list fuzzily', async () => {
    registerCommand({ id: 'a', title: 'Open Agent Monitor', run: vi.fn() });
    registerCommand({ id: 'b', title: 'Restart Session', run: vi.fn() });
    registerCommand({ id: 'c', title: 'Open Settings', run: vi.fn() });
    const { findByTestId, findAllByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'open' } });
    const items = await findAllByTestId('command-palette-item');
    expect(items.map((el) => el.textContent?.trim())).toEqual([
      'Open Agent Monitor',
      'Open Settings',
    ]);
  });

  it('fuzzy subsequence match works beyond plain substring (e.g. "am" → "Open Agent Monitor")', async () => {
    registerCommand({ id: 'a', title: 'Open Agent Monitor', run: vi.fn() });
    registerCommand({ id: 'b', title: 'Restart Session', run: vi.fn() });
    const { findByTestId, findAllByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    // "am" is not a substring of either title, but it IS a subsequence of
    // "Open Agent Monitor" (A in "Agent", m in "Monitor"). Proves the
    // fuzzy-match path through the component, not just substring filtering.
    fireEvent.input(input, { target: { value: 'am' } });
    const items = await findAllByTestId('command-palette-item');
    expect(items.map((el) => el.textContent?.trim())).toEqual([
      'Open Agent Monitor',
    ]);
  });

  it('clicking a list item invokes its run handler and closes the palette', async () => {
    const run = vi.fn();
    registerCommand({ id: 'a', title: 'Alpha', run });
    const { findByTestId, findAllByTestId, queryByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    await findByTestId('command-palette');
    const items = await findAllByTestId('command-palette-item');
    fireEvent.click(items[0]!);
    expect(run).toHaveBeenCalledTimes(1);
    expect(queryByTestId('command-palette')).toBeNull();
  });

  it('Enter on the input invokes the active command and closes', async () => {
    const run = vi.fn();
    registerCommand({ id: 'a', title: 'Alpha', run });
    const { findByTestId, queryByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(run).toHaveBeenCalledTimes(1);
    expect(queryByTestId('command-palette')).toBeNull();
  });

  it('ArrowDown/ArrowUp move the active index and Enter runs the active row', async () => {
    const runA = vi.fn();
    const runB = vi.fn();
    registerCommand({ id: 'a', title: 'Alpha', run: runA });
    registerCommand({ id: 'b', title: 'Bravo', run: runB });
    const { findByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(runA).not.toHaveBeenCalled();
    expect(runB).toHaveBeenCalledTimes(1);
  });
});

// F-402: dialog contract — aria-modal, focus trap on Tab, focus restore on
// close.
describe('CommandPalette — a11y dialog contract (F-402)', () => {
  it('declares aria-modal="true" when open', async () => {
    const { findByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const root = await findByTestId('command-palette');
    expect(root.getAttribute('aria-modal')).toBe('true');
  });

  it('Tab at the last focusable cycles to the first (traps focus)', async () => {
    registerCommand({ id: 'a', title: 'Alpha', run: vi.fn() });
    const { findByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    // Only the input is focusable inside the dialog (list items are clickable
    // but not tab-stops). So Tab from the input should wrap back to itself;
    // we assert focus stays inside the dialog rather than escaping to the
    // trigger element.
    input.focus();
    expect(document.activeElement).toBe(input);
    const root = await findByTestId('command-palette');
    const ev = new KeyboardEvent('keydown', { key: 'Tab', bubbles: true, cancelable: true });
    input.dispatchEvent(ev);
    expect(root.contains(document.activeElement)).toBe(true);
  });

  it('restores focus to the previously-focused element on close', async () => {
    // Mount a trigger button and focus it; open palette; close; assert the
    // trigger regains focus.
    const trigger = document.createElement('button');
    trigger.textContent = 'open palette';
    document.body.appendChild(trigger);
    trigger.focus();
    expect(document.activeElement).toBe(trigger);

    const { findByTestId, queryByTestId } = render(() => <CommandPalette />);
    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    // Sanity — focus moved into the palette.
    expect(input).toBe(document.activeElement);
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(queryByTestId('command-palette')).toBeNull();
    expect(document.activeElement).toBe(trigger);
  });
});

// ---------------------------------------------------------------------------
// F-153 bonus — the "Open Agent Monitor" builtin navigates to `/agents`.
// ---------------------------------------------------------------------------
describe('CommandPalette + F-153 builtin (F-157 bonus)', () => {
  it('registering "Open Agent Monitor" and selecting it navigates to /agents', async () => {
    // Route-level harness: mount the palette inside a Router so `useNavigate`
    // works; assert navigation by reading the memory history.
    // Import lazily to avoid circular-import issues with Router children.
    const { registerBuiltins } = await import('./registerBuiltins');

    const history = createMemoryHistory();
    history.set({ value: '/' });

    // Minimal dashboard component that calls registerBuiltins on mount —
    // mirrors how the real App will register entries.
    function Harness() {
      // Register once on first mount. Router has initialized by now so
      // `useNavigate` inside the builtin works.
      registerBuiltins();
      return (
        <>
          <CommandPalette />
          <div data-testid="route-marker">/</div>
        </>
      );
    }

    const { findByTestId, findAllByTestId } = render(() => (
      <MemoryRouter history={history}>
        <Route path="/" component={Harness} />
        <Route path="/agents" component={() => <div data-testid="agents-marker">agents</div>} />
      </MemoryRouter>
    ));

    openWithCtrlK();
    const input = (await findByTestId('command-palette-input')) as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'agent' } });
    const items = await findAllByTestId('command-palette-item');
    expect(items.length).toBeGreaterThan(0);
    // Click the first match — "Open Agent Monitor" should rank first for "agent".
    fireEvent.click(items[0]!);
    // Memory history should have navigated to /agents.
    await findByTestId('agents-marker');
    expect(history.get()).toBe('/agents');
  });
});
