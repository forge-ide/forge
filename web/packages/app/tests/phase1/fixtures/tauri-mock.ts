// Playwright fixture that installs a mock `window.__TAURI_INTERNALS__` before
// the app boots. Commands registered via `onInvoke()` intercept `invoke()`
// calls; events fired via `emit()` reach handlers registered with
// `@tauri-apps/api/event`'s `listen()`.
//
// The fixture is intentionally thin. It mirrors Tauri 2's runtime contract:
//   - `invoke(cmd, args)` dispatches to a command handler
//   - `listen(event, handler)` routes through `invoke('plugin:event|listen', …)`
//     and parks the callback under a numeric id; emitted payloads invoke every
//     registered callback for that event name
//
// Extend as new UATs need more surface. Document any fields you add.

import { test as base, expect, type Page } from '@playwright/test';

export type InvokeHandler<T = unknown> = (args: Record<string, unknown>) => Promise<T> | T;

export interface TauriMockHandle {
  /** Register a response for an `invoke(cmd, …)` call. Last registration wins. */
  onInvoke<T>(cmd: string, handler: InvokeHandler<T>): void;
  /** Emit a Tauri event to every registered `listen(eventName, …)` handler. */
  emit(eventName: string, payload: unknown): Promise<void>;
  /** Return the ordered list of `[cmd, args]` invocations observed so far. */
  calls(): Promise<Array<{ cmd: string; args: Record<string, unknown> }>>;
  /** Clear recorded calls. Does not unregister handlers. */
  reset(): Promise<void>;
}

declare global {
  interface Window {
    __PHASE1_FIXTURE__: {
      events: Map<string, Map<number, (payload: unknown) => void>>;
      calls: Array<{ cmd: string; args: Record<string, unknown> }>;
    };
    __phase1_dispatch: (cmd: string, args: Record<string, unknown>) => Promise<unknown>;
  }
}

const INIT_SCRIPT = `
(() => {
  if (window.__PHASE1_FIXTURE__) return;
  const fixture = {
    events: new Map(),
    calls: [],
  };
  window.__PHASE1_FIXTURE__ = fixture;

  async function invoke(cmd, args = {}) {
    fixture.calls.push({ cmd, args });

    if (cmd === 'plugin:event|listen') {
      const eventName = args.event;
      const callbackId = args.handler;
      if (!fixture.events.has(eventName)) fixture.events.set(eventName, new Map());
      const slot = fixture.events.get(eventName);
      slot.set(callbackId, (payload) => {
        const cb = window['_' + callbackId];
        if (typeof cb === 'function') cb({ event: eventName, id: callbackId, payload });
      });
      return callbackId;
    }
    if (cmd === 'plugin:event|unlisten') {
      const slot = fixture.events.get(args.event);
      if (slot) slot.delete(args.eventId);
      return undefined;
    }

    // Delegate all non-event commands to the host-side dispatcher. Tests may
    // register, replace, or remove handlers at any time; a missing handler
    // surfaces as a rejected promise with a clear message.
    if (typeof window.__phase1_dispatch !== 'function') {
      return Promise.reject(new Error('phase1 fixture: dispatcher not installed before ' + cmd));
    }
    return await window.__phase1_dispatch(cmd, args);
  }

  let callbackCounter = 1;
  function transformCallback(fn, once = false) {
    const id = callbackCounter++;
    const key = '_' + id;
    window[key] = (value) => {
      if (once) delete window[key];
      fn(value);
    };
    return id;
  }

  window.__TAURI_INTERNALS__ = {
    invoke,
    transformCallback,
    metadata: { currentWebview: { label: 'main' }, currentWindow: { label: 'main' } },
    plugins: {},
    convertFileSrc: (src) => src,
    ipc: (_msg) => {},
  };
})();
`;

export async function installTauriMock(page: Page): Promise<TauriMockHandle> {
  await page.addInitScript(INIT_SCRIPT);

  // Host-side handler map. One dispatcher is exposed per page (Playwright
  // rejects duplicate `exposeFunction` names), and every registered or
  // replaced handler is routed through it.
  const handlers = new Map<string, InvokeHandler>();
  await page.exposeFunction(
    '__phase1_dispatch',
    async (cmd: string, args: Record<string, unknown>) => {
      const handler = handlers.get(cmd);
      if (!handler) throw new Error(`phase1 fixture: no handler for ${cmd}`);
      return await handler(args);
    },
  );

  return {
    onInvoke(cmd, handler) {
      handlers.set(cmd, handler as InvokeHandler);
    },
    async emit(eventName, payload) {
      await page.evaluate(
        ({ eventName, payload }) => {
          const slot = window.__PHASE1_FIXTURE__.events.get(eventName);
          if (!slot) return;
          for (const cb of slot.values()) cb(payload);
        },
        { eventName, payload },
      );
    },
    async calls() {
      return await page.evaluate(() => window.__PHASE1_FIXTURE__.calls.slice());
    },
    async reset() {
      handlers.clear();
      await page.evaluate(() => {
        window.__PHASE1_FIXTURE__.calls.length = 0;
      });
    },
  };
}

// Playwright fixture extension exposing `tauri` on each test.
export const test = base.extend<{ tauri: TauriMockHandle }>({
  tauri: async ({ page }, use) => {
    const handle = await installTauriMock(page);
    await use(handle);
  },
});

export { expect };
