import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render } from '@solidjs/testing-library';
import type { SessionId } from '@forge/ipc';
import { EditorPane } from './EditorPane';
import { TerminalPane } from './TerminalPane';
import { setActiveSessionId } from '../stores/session';
import { setInvokeForTesting } from '../lib/tauri';

// F-394 regression: rendering both panes in the same window must produce a
// single banner landmark for the document (ideally zero sub-structural
// banners — the real top-level banner, if any, is owned by the window
// shell, not by individual panes). Prior to F-394 both EditorPane and
// PaneHeader stamped role="banner" on their sub-structural <header>, so
// this assertion would have failed with two `[role="banner"]` elements.

// xterm + matchMedia / IntersectionObserver polyfills — copied from
// TerminalPane.test.tsx so this suite can mount TerminalPane without the
// xterm boot path blowing up in jsdom.
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
(globalThis as { IntersectionObserver?: unknown }).IntersectionObserver = class {
  constructor() {}
  observe() {}
  unobserve() {}
  disconnect() {}
  takeRecords() {
    return [];
  }
};

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async () => () => {}),
}));

const SID = 'session-landmark-test' as SessionId;

beforeEach(() => {
  setActiveSessionId(SID);
  setInvokeForTesting(vi.fn().mockResolvedValue(undefined) as never);
  (globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
});

afterEach(() => {
  cleanup();
  setActiveSessionId(null);
  setInvokeForTesting(null);
});

describe('F-394 — single banner landmark across coexisting panes', () => {
  it('produces at most one role="banner" when editor + terminal panes co-render', () => {
    const { container } = render(() => (
      <div>
        <EditorPane
          path="/ws/file.ts"
          src="about:blank"
          readFile={vi.fn().mockResolvedValue({
            path: '/ws/file.ts',
            content: '',
            bytes: 0,
            sha256: '',
          })}
          writeFile={vi.fn().mockResolvedValue(undefined)}
          onClose={vi.fn()}
        />
        <TerminalPane cwd="/ws" shell="/bin/zsh" onClose={vi.fn()} />
      </div>
    ));
    // Post-F-394: both panes drop their sub-structural banner. The test
    // tolerates a single shell-level banner the surrounding window might
    // add, but never two banners from the panes themselves.
    const banners = container.querySelectorAll('[role="banner"]');
    expect(banners.length).toBeLessThanOrEqual(1);
  });
});
