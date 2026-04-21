import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { LayoutTree, Layouts } from '@forge/ipc';
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

  // F-150: openFile now writes into a real editor leaf in the GridContainer
  // tree. The reducer walks the tree DFS and either updates an existing
  // editor leaf's `pane_state[<id>].active_file`, or splits the root right
  // with a freshly-minted editor leaf when no editor is present.
  it('openFile splits the root right with a fresh editor leaf when no editor exists', async () => {
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.openFile('/ws/src/main.ts');

    const tree = store.layouts.named.default?.tree;
    // Root is now a v-split (chat left, editor right).
    expect(tree?.kind).toBe('split');
    if (tree?.kind !== 'split') throw new Error('expected split');
    expect(tree.direction).toBe('v');
    expect(tree.ratio).toBeCloseTo(0.5);
    expect(tree.a.kind).toBe('leaf');
    if (tree.a.kind !== 'leaf') throw new Error('expected leaf');
    expect(tree.a.pane_type).toBe('chat');
    expect(tree.b.kind).toBe('leaf');
    if (tree.b.kind !== 'leaf') throw new Error('expected leaf');
    expect(tree.b.pane_type).toBe('editor');

    // The editor leaf's active_file is wired via pane_state keyed on the
    // leaf's own id.
    const editorId = tree.b.id;
    expect(
      store.layouts.named.default?.pane_state[editorId]?.active_file,
    ).toBe('/ws/src/main.ts');

    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();
    expect(writes[0]?.named.default?.pane_state[editorId]?.active_file).toBe(
      '/ws/src/main.ts',
    );
  });

  it('openFile reuses the first editor leaf in DFS order when one exists', async () => {
    // Pre-load a layout that already has an editor leaf mid-tree.
    const seed: LayoutTree = {
      kind: 'split',
      id: 'root',
      direction: 'v',
      ratio: 0.5,
      a: { kind: 'leaf', id: 'chat-1', pane_type: 'chat' },
      b: { kind: 'leaf', id: 'editor-1', pane_type: 'editor' },
    };
    read.mockResolvedValueOnce({
      active: 'default',
      named: {
        default: {
          tree: seed,
          pane_state: { 'editor-1': { active_file: '/ws/old.ts' } },
        },
      },
    });
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.openFile('/ws/new.ts');

    // Tree structure is unchanged (same leaf count, same ids).
    const tree = store.layouts.named.default?.tree;
    expect(tree).toEqual(seed);
    // active_file on the existing leaf swapped to the new path.
    expect(
      store.layouts.named.default?.pane_state['editor-1']?.active_file,
    ).toBe('/ws/new.ts');

    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();
    expect(
      writes[0]?.named.default?.pane_state['editor-1']?.active_file,
    ).toBe('/ws/new.ts');
  });

  it('openFile assigns a fresh id for each new editor leaf so multiple editors can coexist', async () => {
    // Start from the singleton chat layout; openFile once to create editor-1.
    // Then closeLeaf(editor-1) to restore the singleton, then openFile again —
    // the new editor must not collide with the first id even after removal.
    const { scheduler } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.openFile('/ws/a.ts');
    const firstTree = store.layouts.named.default?.tree;
    if (firstTree?.kind !== 'split') throw new Error('expected split');
    if (firstTree.b.kind !== 'leaf') throw new Error('expected leaf');
    const firstEditorId = firstTree.b.id;

    store.closeLeaf(firstEditorId);
    store.openFile('/ws/b.ts');
    const secondTree = store.layouts.named.default?.tree;
    if (secondTree?.kind !== 'split') throw new Error('expected split');
    if (secondTree.b.kind !== 'leaf') throw new Error('expected leaf');
    const secondEditorId = secondTree.b.id;

    expect(secondEditorId).not.toBe(firstEditorId);
  });

  it('closeLeaf removes a leaf, promotes its sibling, and reclaims pane_state', async () => {
    const seed: LayoutTree = {
      kind: 'split',
      id: 'root',
      direction: 'v',
      ratio: 0.5,
      a: { kind: 'leaf', id: 'chat-1', pane_type: 'chat' },
      b: { kind: 'leaf', id: 'editor-1', pane_type: 'editor' },
    };
    read.mockResolvedValueOnce({
      active: 'default',
      named: {
        default: {
          tree: seed,
          pane_state: {
            'editor-1': { active_file: '/ws/gone.ts' },
            'chat-1': {},
          },
        },
      },
    });
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    store.closeLeaf('editor-1');

    const tree = store.layouts.named.default?.tree;
    // Sibling promoted — root becomes the chat leaf.
    expect(tree?.kind).toBe('leaf');
    if (tree?.kind !== 'leaf') throw new Error('expected leaf');
    expect(tree.id).toBe('chat-1');
    expect(tree.pane_type).toBe('chat');

    // pane_state for the removed leaf is gone; the surviving leaf's state
    // is preserved.
    const paneState = store.layouts.named.default?.pane_state ?? {};
    expect('editor-1' in paneState).toBe(false);
    expect('chat-1' in paneState).toBe(true);

    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();
    expect('editor-1' in (writes[0]?.named.default?.pane_state ?? {})).toBe(
      false,
    );
  });

  it('closeLeaf on the sole remaining leaf resets to the default chat layout', async () => {
    const { scheduler } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    // Default seed is a single `root`/`chat` leaf.
    store.closeLeaf('root');

    const tree = store.layouts.named.default?.tree;
    expect(tree?.kind).toBe('leaf');
    if (tree?.kind !== 'leaf') throw new Error('expected leaf');
    expect(tree.pane_type).toBe('chat');
  });

  it('setLayoutTree replaces the active layout\'s tree and schedules a write', async () => {
    const { scheduler, tick } = makeFakeScheduler();
    const store = createLayoutStore(WORKSPACE, { read, write, scheduler });
    await store.load();

    const next: LayoutTree = {
      kind: 'split',
      id: 'root',
      direction: 'h',
      ratio: 0.3,
      a: { kind: 'leaf', id: 'chat-1', pane_type: 'chat' },
      b: { kind: 'leaf', id: 'terminal-1', pane_type: 'terminal' },
    };
    store.setLayoutTree('default', next);
    expect(store.layouts.named.default?.tree).toEqual(next);

    tick(DEFAULT_DEBOUNCE_MS);
    await store.flush();
    expect(writes[0]?.named.default?.tree).toEqual(next);
  });
});
