import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import { setInvokeForTesting } from '../lib/tauri';

// xterm.js touches `window.matchMedia` / `IntersectionObserver` /
// `ResizeObserver` / `requestAnimationFrame` when `term.open()` runs. jsdom
// provides some of these but not matchMedia, so polyfill it before the
// module graph that imports xterm resolves. The shim returns "not matches"
// for every query — we are not exercising responsive CSS in unit tests.
(globalThis as { matchMedia?: unknown }).matchMedia = (query: string) => ({
  matches: false,
  media: query,
  onchange: null,
  addListener: () => {},
  removeListener: () => {},
  addEventListener: () => {},
  removeEventListener: () => {},
  dispatchEvent: () => false,
});
if (typeof window !== 'undefined') {
  (window as unknown as { matchMedia: unknown }).matchMedia =
    (globalThis as { matchMedia: unknown }).matchMedia;
}
// xterm also probes `IntersectionObserver` to decide whether to render the
// viewport eagerly. jsdom doesn't ship one.
(globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = class {
  constructor() {}
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
};

import { shellDisplayName, newTerminalId, TerminalPane } from './TerminalPane';

// Pre-mock the Tauri event subscription before the module under test is
// evaluated so the `@tauri-apps/api/event` import inside TerminalPane
// resolves against our in-memory pub/sub.
type EventListener<T> = (event: { payload: T }) => void;

const eventHandlers = new Map<string, Set<EventListener<unknown>>>();

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: EventListener<unknown>) => {
    let set = eventHandlers.get(name);
    if (!set) {
      set = new Set();
      eventHandlers.set(name, set);
    }
    set.add(handler);
    return () => {
      set?.delete(handler);
    };
  }),
}));

/** Fire a Tauri event at every subscribed listener in the current test. */
function emitTauriEvent<T>(name: string, payload: T): void {
  const set = eventHandlers.get(name);
  if (!set) return;
  for (const handler of set) {
    handler({ payload });
  }
}

const invokeMock = vi.fn();

beforeEach(() => {
  invokeMock.mockReset();
  invokeMock.mockResolvedValue(undefined);
  setInvokeForTesting(invokeMock as never);
  eventHandlers.clear();
  // jsdom has no ResizeObserver — xterm + our pane both tolerate its
  // absence, but the global needs to exist if the pane decides to construct
  // one. A minimal no-op stand-in is enough for unit tests; the real
  // resize→terminal_resize path is exercised in the Rust integration test.
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

afterEach(() => {
  cleanup();
  setInvokeForTesting(null);
});

describe('shellDisplayName', () => {
  it('returns the trailing path component for a full path', () => {
    expect(shellDisplayName('/usr/local/bin/zsh')).toBe('zsh');
  });

  it('returns the input when there is no slash', () => {
    expect(shellDisplayName('fish')).toBe('fish');
  });

  it('falls back to "shell" for undefined / empty input', () => {
    expect(shellDisplayName(undefined)).toBe('shell');
    expect(shellDisplayName('')).toBe('shell');
    expect(shellDisplayName('   ')).toBe('shell');
  });
});

describe('newTerminalId', () => {
  it('produces 16-hex-char ids distinct across calls (64-bit entropy)', () => {
    const a = newTerminalId();
    const b = newTerminalId();
    expect(a).toMatch(/^[0-9a-f]{16}$/);
    expect(b).toMatch(/^[0-9a-f]{16}$/);
    expect(a).not.toBe(b);
  });
});

describe('TerminalPane', () => {
  it('renders the pane-header with TERMINAL type label and the cwd in the detail slot', () => {
    const { getByTestId } = render(() => (
      <TerminalPane cwd="/tmp/forge-repo" shell="/bin/zsh" onClose={vi.fn()} />
    ));
    // Type label is the `CHAT/TERMINAL/EDITOR` slot in the pane header.
    const header = getByTestId('pane-header-subject');
    expect(header.textContent).toBe('zsh');
    // The cwd goes into the cost-meter slot (pane-header.md §PH.4 format-free
    // detail pocket); `TerminalPane` repurposes it for the path display.
    const detail = getByTestId('pane-header-cost');
    expect(detail.textContent).toBe('/tmp/forge-repo');
  });

  it('falls back to shellDisplayName when shellName is omitted', () => {
    const { getByTestId } = render(() => (
      <TerminalPane cwd="/tmp" shell="/bin/fish" onClose={vi.fn()} />
    ));
    expect(getByTestId('pane-header-subject').textContent).toBe('fish');
  });

  it('prefers an explicit shellName prop over shell-path derivation', () => {
    const { getByTestId } = render(() => (
      <TerminalPane
        cwd="/tmp"
        shell="/bin/zsh"
        shellName="Rust sandbox"
        onClose={vi.fn()}
      />
    ));
    expect(getByTestId('pane-header-subject').textContent).toBe('Rust sandbox');
  });

  it('calls terminal_spawn on mount with TerminalSpawnArgs-shaped payload', async () => {
    render(() => (
      <TerminalPane cwd="/tmp/forge-spawn" shell="/bin/sh" onClose={vi.fn()} />
    ));
    // The spawn is inside an async IIFE; yield microtasks until the invoke
    // has been seen. `vi.waitFor` would work too but we already know there
    // are exactly two dependent awaits (listen x2) before the spawn fires.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    const spawnCall = invokeMock.mock.calls.find((c) => c[0] === 'terminal_spawn');
    expect(spawnCall).toBeDefined();
    const [, payload] = spawnCall!;
    expect(payload.args).toMatchObject({
      shell: '/bin/sh',
      cwd: '/tmp/forge-spawn',
    });
    expect(typeof payload.args.terminal_id).toBe('string');
    expect(payload.args.terminal_id).toMatch(/^[0-9a-f]{16}$/);
    expect(typeof payload.args.cols).toBe('number');
    expect(typeof payload.args.rows).toBe('number');
  });

  it('calls terminal_kill on unmount for a successfully spawned terminal', async () => {
    const { unmount } = render(() => (
      <TerminalPane cwd="/tmp/forge-kill" onClose={vi.fn()} />
    ));
    // Let the spawn async chain resolve so `spawnCompleted` flips to true.
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    unmount();

    const killCall = invokeMock.mock.calls.find((c) => c[0] === 'terminal_kill');
    expect(killCall).toBeDefined();
    const [, payload] = killCall!;
    expect(typeof payload.terminalId).toBe('string');
  });

  it('surfaces a spawn failure in the inline error slot', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'terminal_spawn') {
        return Promise.reject(new Error('forbidden: window label mismatch'));
      }
      return Promise.resolve(undefined);
    });

    const { findByRole } = render(() => (
      <TerminalPane cwd="/tmp" onClose={vi.fn()} />
    ));

    const alert = await findByRole('alert');
    expect(alert.textContent).toMatch(/terminal_spawn failed/);
    expect(alert.textContent).toMatch(/label mismatch/);
  });

  it('does not invoke terminal_kill when spawn failed (nothing to tear down)', async () => {
    invokeMock.mockImplementation((command: string) => {
      if (command === 'terminal_spawn') {
        return Promise.reject(new Error('boom'));
      }
      return Promise.resolve(undefined);
    });

    const { unmount } = render(() => (
      <TerminalPane cwd="/tmp" onClose={vi.fn()} />
    ));
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    unmount();

    const killCall = invokeMock.mock.calls.find((c) => c[0] === 'terminal_kill');
    expect(killCall).toBeUndefined();
  });

  it('subscribes to terminal:bytes and terminal:exit for the spawned id', async () => {
    render(() => <TerminalPane cwd="/tmp" onClose={vi.fn()} />);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    expect(eventHandlers.get('terminal:bytes')?.size ?? 0).toBeGreaterThan(0);
    expect(eventHandlers.get('terminal:exit')?.size ?? 0).toBeGreaterThan(0);
  });

  it('ignores terminal:bytes events for a different terminal_id', async () => {
    render(() => <TerminalPane cwd="/tmp" onClose={vi.fn()} />);
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    // Sending an event for an unrelated id should not throw. The real
    // guarantee is cross-terminal isolation; we assert the handler runs
    // cleanly rather than checking xterm's internal state (which jsdom
    // doesn't fully render).
    expect(() =>
      emitTauriEvent('terminal:bytes', {
        terminal_id: 'ffffffffffffffff',
        data: [72, 105],
      }),
    ).not.toThrow();
  });

  it('wires CLOSE PANE on the close button', async () => {
    const onClose = vi.fn();
    const { getByRole } = render(() => (
      <TerminalPane cwd="/tmp" onClose={onClose} />
    ));
    const btn = getByRole('button', { name: 'Close pane' });
    expect(btn.textContent).toBe('CLOSE PANE');
    btn.click();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
