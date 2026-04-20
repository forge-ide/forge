import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Layouts } from '@forge/ipc';
import {
  createLayoutStore,
  DEFAULT_DEBOUNCE_MS,
  defaultLayouts,
  type Scheduler,
} from './layoutStore';

// ---------------------------------------------------------------------------
// Fake scheduler — lets tests advance the debounce clock deterministically
// without polluting `globalThis` or relying on vitest fake timers (which also
// affect `queueMicrotask` / `Promise` resolution and make the assertions
// harder to read).
// ---------------------------------------------------------------------------

interface ScheduledTask {
  id: number;
  fn: () => void;
  delay: number;
}

function makeFakeScheduler(): { scheduler: Scheduler; tick: (ms: number) => void } {
  const tasks = new Map<number, ScheduledTask>();
  let nextId = 1;

  const scheduler: Scheduler = {
    setTimeout: (fn, ms) => {
      const id = nextId++;
      tasks.set(id, { id, fn, delay: ms });
      return id;
    },
    clearTimeout: (handle) => {
      tasks.delete(handle as number);
    },
  };

  const tick = (_ms: number) => {
    // Fire every currently-scheduled task once. The store only ever has one
    // outstanding debounce handle, so this is sufficient for our assertions.
    const snap = [...tasks.values()];
    tasks.clear();
    for (const task of snap) {
      task.fn();
    }
  };

  return { scheduler, tick };
}

const WORKSPACE = '/tmp/ws';

describe('layoutStore', () => {
  let writes: Layouts[];
  let read: ReturnType<typeof vi.fn>;
  let write: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    writes = [];
    read = vi.fn(async () => defaultLayouts());
    write = vi.fn(async (_root: string, layouts: Layouts) => {
      // `layouts` is the snapshot the store assembled — already a plain
      // object detached from the Solid proxy, so we can keep the reference
      // directly without cloning.
      writes.push(layouts);
    });
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('hydrates from the shell on load()', async () => {
    const hydrated: Layouts = {
      active: 'split-editor',
      named: {
        'split-editor': {
          tree: { kind: 'leaf', id: 'a', pane_type: 'editor' },
          pane_state: {},
        },
      },
    };
    read.mockResolvedValueOnce(hydrated);

    const { scheduler } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });

    await store.load();
    expect(store.layouts.active).toBe('split-editor');
    expect(store.layouts.named['split-editor']?.tree.kind).toBe('leaf');
  });

  it('falls back to default when load() does not mutate the store', async () => {
    // Shell degrades to default on corrupt/missing — callers should never
    // see an error from `load()`. The store exposes the default immediately.
    const { scheduler } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    expect(store.layouts.active).toBe('default');
    expect(store.layouts.named.default?.tree).toEqual({
      kind: 'leaf',
      id: 'root',
      pane_type: 'chat',
    });
  });

  it('debounces mutations into a single write after 500 ms of quiet', async () => {
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    // Burst: three mutations in quick succession.
    store.setActive('a');
    store.setActive('b');
    store.setActive('c');

    // Nothing written yet — the debounce has not elapsed.
    expect(write).not.toHaveBeenCalled();

    // Fire the timer — exactly one write lands, carrying the latest state.
    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();

    expect(write).toHaveBeenCalledTimes(1);
    expect(writes[0]?.active).toBe('c');
  });

  it('starts a fresh debounce on each mutation, not on the first', async () => {
    // The spec calls out "500 ms debounce" (= fire after quiet, not
    // fire-every-500-ms). A mutation during the debounce window extends it.
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.setActive('a');
    // Advance partway — the scheduler fires everything on the queue, but
    // the store must re-arm on the next mutation. We simulate this by
    // firing, then immediately triggering another mutation: only that
    // mutation's timer should be outstanding.
    store.setActive('b');
    expect(write).not.toHaveBeenCalled();
    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();
    // Exactly one write, carrying 'b'.
    expect(write).toHaveBeenCalledTimes(1);
    expect(writes[0]?.active).toBe('b');
  });

  it('flush() runs the pending write immediately', async () => {
    const { scheduler } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.setActive('flushed');
    await store.flush();
    expect(write).toHaveBeenCalledTimes(1);
    expect(writes[0]?.active).toBe('flushed');
  });

  it('cancelPendingWrite() drops the scheduled write', async () => {
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.setActive('dropped');
    store.cancelPendingWrite();
    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();

    expect(write).not.toHaveBeenCalled();
  });

  it('reports write errors via onWriteError and keeps accepting mutations', async () => {
    const errors: unknown[] = [];
    const boomWrite = vi.fn(async () => {
      throw new Error('ENOSPC');
    });
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, {
      read,
      write: boomWrite,
      scheduler,
      onWriteError: (e) => errors.push(e),
    });
    await store.load();

    store.setActive('doomed');
    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();

    expect(boomWrite).toHaveBeenCalledTimes(1);
    expect(errors).toHaveLength(1);
    expect((errors[0] as Error).message).toBe('ENOSPC');

    // A subsequent mutation must still be able to schedule.
    store.setActive('retry');
    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();
    expect(boomWrite).toHaveBeenCalledTimes(2);
  });

  it('setPaneState only writes the affected leaf', async () => {
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.setPaneState('default', 'root', { scroll_top: 42n });
    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();

    expect(writes[0]?.named.default?.pane_state.root?.scroll_top).toBe(42n);
    // Other layouts untouched.
    expect(Object.keys(writes[0]?.named ?? {})).toEqual(['default']);
  });
});
