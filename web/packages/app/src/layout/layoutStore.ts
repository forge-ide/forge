import { createStore, reconcile } from 'solid-js/store';
import type { Layout, Layouts, PaneState } from '@forge/ipc';
import { readLayouts, writeLayouts } from '../ipc/layouts';

/**
 * F-120: per-workspace pane layout persistence.
 *
 * Owns the `Layouts` record for the active workspace. On mount it calls
 * `read_layouts` and seeds the store; on every subsequent mutation it
 * schedules a single debounced `write_layouts` (500 ms) so a drag-resize
 * burst collapses into one disk write. A missing or corrupt file produces
 * the default single-pane layout at the shell layer — this store never has
 * to branch on "did the file exist".
 *
 * Testability. The debounce and the write transport are parameterizable: a
 * test can pass a synchronous write hook and a `setTimeout`-like scheduler
 * to drive time deterministically. Production callers use the module-level
 * factory which wires the Tauri IPC and real `window.setTimeout`.
 */

export const DEFAULT_DEBOUNCE_MS = 500;

/**
 * The canonical empty layout the UI falls back to when nothing else resolves.
 * Kept in sync with the Rust `Layouts::default()` so the shell and frontend
 * agree on the shape a fresh workspace produces.
 */
export function defaultLayouts(): Layouts {
  return {
    active: 'default',
    named: {
      default: {
        tree: { kind: 'leaf', id: 'root', pane_type: 'chat' },
        pane_state: {},
      },
    },
  };
}

/**
 * Injection seam for the debounce scheduler. Matches the subset of
 * `window.setTimeout` / `clearTimeout` we actually use so tests can substitute
 * a fake-timer implementation without pulling a global shim.
 */
export interface Scheduler {
  setTimeout: (fn: () => void, ms: number) => unknown;
  clearTimeout: (handle: unknown) => void;
}

const realScheduler: Scheduler = {
  setTimeout: (fn, ms) => globalThis.setTimeout(fn, ms),
  clearTimeout: (handle) => globalThis.clearTimeout(handle as number),
};

export interface LayoutStoreOptions {
  /** Override the debounce window. Defaults to 500 ms per the spec. */
  debounceMs?: number;
  /** Alternate write transport — the integration test seam. Defaults to `writeLayouts`. */
  write?: (workspaceRoot: string, layouts: Layouts) => Promise<void>;
  /** Alternate read transport. Defaults to `readLayouts`. */
  read?: (workspaceRoot: string) => Promise<Layouts>;
  /** Alternate scheduler — defaults to the real `setTimeout`/`clearTimeout`. */
  scheduler?: Scheduler;
  /** Error hook for failed writes. Defaults to `console.error`. */
  onWriteError?: (err: unknown) => void;
}

export interface LayoutStore {
  /** The reactive snapshot. Wrap reads in effects to track changes. */
  readonly layouts: Layouts;
  /** Hydrate from disk. Idempotent — calling twice reseeds from the shell. */
  load: () => Promise<void>;
  /** Replace the entire `Layouts` record. Schedules a debounced write. */
  setLayouts: (next: Layouts) => void;
  /** Replace one named layout's tree. Schedules a debounced write. */
  setLayout: (name: string, layout: Layout) => void;
  /** Upsert pane state for a leaf. Schedules a debounced write. */
  setPaneState: (layoutName: string, leafId: string, state: PaneState) => void;
  /** Change the active named layout. Schedules a debounced write. */
  setActive: (name: string) => void;
  /** Cancel any pending debounced write without persisting. */
  cancelPendingWrite: () => void;
  /** Force the pending debounced write to run immediately. No-op if none pending. */
  flush: () => Promise<void>;
}

/**
 * Construct a layout store for `workspaceRoot`. The store is lazily hydrated —
 * call `store.load()` once on session mount before reading `store.layouts`.
 *
 * The debouncer coalesces a rapid sequence of mutations into a single
 * `write_layouts` call after `debounceMs` of quiet. If a write fails
 * (disk full, read-only mount), the error is reported via `onWriteError`
 * and the next mutation starts a fresh debounce — there is no retry loop
 * because the next layout change will re-persist the same state anyway.
 */
export function createLayoutStore(
  workspaceRoot: string,
  options: LayoutStoreOptions = {},
): LayoutStore {
  const debounceMs = options.debounceMs ?? DEFAULT_DEBOUNCE_MS;
  const write = options.write ?? writeLayouts;
  const read = options.read ?? readLayouts;
  const scheduler = options.scheduler ?? realScheduler;
  const onWriteError =
    options.onWriteError ??
    ((err: unknown) => {
      // Surface to devtools without throwing; the next mutation retries.
      // eslint-disable-next-line no-console
      console.error('[layoutStore] write_layouts failed:', err);
    });

  const [state, setState] = createStore<Layouts>(defaultLayouts());

  let pendingHandle: unknown = null;
  let inflight: Promise<void> | null = null;

  const doWrite = async () => {
    pendingHandle = null;
    // Snapshot the current state at the moment the debounce fires, not when
    // the promise settles — a mutation arriving during the write will enqueue
    // a fresh one rather than stomping this snapshot.
    const snapshot: Layouts = {
      active: state.active,
      named: Object.fromEntries(
        Object.entries(state.named).map(([k, v]) => [
          k,
          {
            tree: v.tree,
            pane_state: { ...v.pane_state },
          },
        ]),
      ),
    };
    inflight = write(workspaceRoot, snapshot).catch(onWriteError);
    try {
      await inflight;
    } finally {
      inflight = null;
    }
  };

  const schedule = () => {
    if (pendingHandle !== null) {
      scheduler.clearTimeout(pendingHandle);
    }
    pendingHandle = scheduler.setTimeout(() => {
      void doWrite();
    }, debounceMs);
  };

  return {
    get layouts() {
      return state;
    },
    async load() {
      const next = await read(workspaceRoot);
      // `reconcile` updates the existing store in place, preserving identity
      // for unchanged subtrees — downstream `createEffect`s that track a
      // specific leaf don't re-fire on an unrelated change.
      setState(reconcile(next));
    },
    setLayouts(next) {
      setState(reconcile(next));
      schedule();
    },
    setLayout(name, layout) {
      setState('named', name, reconcile(layout));
      schedule();
    },
    setPaneState(layoutName, leafId, paneState) {
      setState('named', layoutName, 'pane_state', leafId, paneState);
      schedule();
    },
    setActive(name) {
      setState('active', name);
      schedule();
    },
    cancelPendingWrite() {
      if (pendingHandle !== null) {
        scheduler.clearTimeout(pendingHandle);
        pendingHandle = null;
      }
    },
    async flush() {
      if (pendingHandle !== null) {
        scheduler.clearTimeout(pendingHandle);
        await doWrite();
      } else if (inflight) {
        await inflight;
      }
    },
  };
}
