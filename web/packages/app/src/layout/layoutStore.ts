import { createStore, reconcile } from 'solid-js/store';
import type { Layout, LayoutTree, Layouts, PaneState, PaneType } from '@forge/ipc';
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
 * F-150: openFile / closeLeaf now operate on the real GridContainer tree
 * rather than a synthetic singleton slot. `openFile(path)` either reuses
 * the first editor leaf (DFS order) or splits the root right with a newly
 * minted `editor-<n>` leaf. `closeLeaf(id)` removes a leaf, promotes its
 * sibling, and garbage-collects the matching `pane_state` entry; closing
 * the sole remaining leaf snaps back to the default single-chat layout so
 * the tree is never empty.
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
  /**
   * Injection seam for leaf-id generation. Production uses a module-local
   * monotonic counter; tests can substitute a deterministic generator to
   * keep assertions legible. Called once per freshly-split editor leaf.
   */
  nextEditorId?: () => string;
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
  /** Replace just the tree of a named layout. Schedules a debounced write. */
  setLayoutTree: (name: string, tree: LayoutTree) => void;
  /** Upsert pane state for a leaf. Schedules a debounced write. */
  setPaneState: (layoutName: string, leafId: string, state: PaneState) => void;
  /** Change the active named layout. Schedules a debounced write. */
  setActive: (name: string) => void;
  /**
   * F-150: open `path` in an editor leaf. If the active layout's tree
   * contains an editor leaf, its `pane_state[<id>].active_file` is updated.
   * Otherwise the root is v-split (existing subtree left, new editor right)
   * and the new editor leaf's `pane_state` is populated. Passing `null`
   * is a no-op — use `closeLeaf` to remove an editor.
   */
  openFile: (path: string) => void;
  /**
   * F-150: remove a leaf from the active layout's tree. The leaf's
   * `pane_state` entry is garbage-collected. If the tree would become
   * empty, it resets to the default single-chat layout instead.
   */
  closeLeaf: (leafId: string) => void;
  /** Cancel any pending debounced write without persisting. */
  cancelPendingWrite: () => void;
  /** Force the pending debounced write to run immediately. No-op if none pending. */
  flush: () => Promise<void>;
}

// Module-level monotonic counter for fresh editor-leaf ids. Shared across
// stores because leaf ids live entirely in the persisted tree — if a newer
// session happens to reuse `editor-3` after an old one was persisted, the
// store that loads it still sees a deterministic, collision-free id space
// going forward. The counter never rolls back on closeLeaf so a rapid open/
// close sequence can't alias ids.
let editorIdCounter = 0;
function defaultNextEditorId(): string {
  editorIdCounter += 1;
  return `editor-${editorIdCounter}`;
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
  const nextEditorId = options.nextEditorId ?? defaultNextEditorId;
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
    setLayoutTree(name, tree) {
      // `reconcile` on the tree node preserves identity for unchanged
      // subtrees — critical for Solid's downstream `<Show>` memoization of
      // individual leaf renderers after a drag-to-dock mutation.
      setState('named', name, 'tree', reconcile(tree));
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
    openFile(path) {
      const layoutName = state.active;
      const layout = state.named[layoutName];
      if (!layout) return;
      const existing = findFirstLeafByPaneType(layout.tree, 'editor');
      if (existing !== null) {
        const current = layout.pane_state[existing.id];
        if (current?.active_file === path) return;
        const next: PaneState = { ...(current ?? {}), active_file: path };
        setState('named', layoutName, 'pane_state', existing.id, next);
        schedule();
        return;
      }
      // No editor leaf — v-split the root, existing tree on the left, a
      // fresh editor leaf on the right. `nextEditorId()` never collides with
      // ids in the current tree because the counter is monotonic and tree
      // ids are bounded by the active persistence window.
      //
      // `cloneTree(layout.tree)` breaks the Solid store proxy: Solid exposes
      // nested objects as live proxies that delegate back to the root, so
      // re-inserting `layout.tree` as a child of the new split would create
      // a self-referential subtree under the proxy — infinite recursion on
      // any subsequent traversal.
      const newId = nextEditorId();
      const newTree: LayoutTree = {
        kind: 'split',
        id: `split-${newId}`,
        direction: 'v',
        ratio: 0.5,
        a: cloneTree(layout.tree),
        b: { kind: 'leaf', id: newId, pane_type: 'editor' },
      };
      setState('named', layoutName, 'tree', reconcile(newTree));
      setState('named', layoutName, 'pane_state', newId, {
        active_file: path,
      });
      schedule();
    },
    closeLeaf(leafId) {
      const layoutName = state.active;
      const layout = state.named[layoutName];
      if (!layout) return;
      const next = removeLeaf(layout.tree, leafId);
      if (next === null || next === layout.tree) {
        // Sole-leaf removal returns null; reset to the default single-chat
        // tree so the UI always has something to render. If the leaf wasn't
        // in the tree at all, `removeLeaf` returns the original reference —
        // bail out without touching persistence.
        if (next === null) {
          const fresh = defaultLayouts().named.default;
          if (fresh !== undefined) {
            setState('named', layoutName, 'tree', reconcile(fresh.tree));
            setState('named', layoutName, 'pane_state', reconcile({}));
            schedule();
          }
        }
        return;
      }
      setState('named', layoutName, 'tree', reconcile(next));
      // Garbage-collect pane_state for the removed leaf. Leaves that are
      // still present keep their state by identity.
      if (layoutName in state.named) {
        const current = state.named[layoutName]?.pane_state;
        if (current && leafId in current) {
          const pruned: { [k: string]: PaneState } = {};
          for (const [k, v] of Object.entries(current)) {
            if (k !== leafId) pruned[k] = v;
          }
          setState('named', layoutName, 'pane_state', reconcile(pruned));
        }
      }
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

/**
 * DFS-traverse a tree, returning the first leaf whose `pane_type` matches.
 * Left-first traversal gives a predictable reuse rule — the leaf closest to
 * the top-left of the tree wins, matching the user's mental model of "the
 * editor I see when I open the session".
 */
export function findFirstLeafByPaneType(
  tree: LayoutTree,
  paneType: PaneType,
): (LayoutTree & { kind: 'leaf' }) | null {
  if (tree.kind === 'leaf') {
    return tree.pane_type === paneType ? tree : null;
  }
  return (
    findFirstLeafByPaneType(tree.a, paneType) ??
    findFirstLeafByPaneType(tree.b, paneType)
  );
}

/** DFS-collect every leaf id that appears in `tree` (stable iteration order). */
export function collectLeafIds(tree: LayoutTree): string[] {
  if (tree.kind === 'leaf') return [tree.id];
  return [...collectLeafIds(tree.a), ...collectLeafIds(tree.b)];
}

/**
 * Remove a leaf from the tree by id. Returns:
 *   - the unchanged tree reference if the leaf is not present,
 *   - `null` if the tree itself was the sole leaf being removed,
 *   - a new tree with the sibling promoted otherwise.
 * Split nodes along the path are structurally rebuilt; untouched subtrees
 * are shared by reference so Solid's `reconcile` can preserve identity.
 */
export function removeLeaf(tree: LayoutTree, id: string): LayoutTree | null {
  if (tree.kind === 'leaf') return tree.id === id ? null : tree;
  if (tree.a.kind === 'leaf' && tree.a.id === id) return tree.b;
  if (tree.b.kind === 'leaf' && tree.b.id === id) return tree.a;
  const containsInA = leafExists(tree.a, id);
  const containsInB = leafExists(tree.b, id);
  if (containsInA) {
    const nextA = removeLeaf(tree.a, id);
    return nextA === null ? tree.b : { ...tree, a: nextA };
  }
  if (containsInB) {
    const nextB = removeLeaf(tree.b, id);
    return nextB === null ? tree.a : { ...tree, b: nextB };
  }
  return tree;
}

function leafExists(tree: LayoutTree, id: string): boolean {
  if (tree.kind === 'leaf') return tree.id === id;
  return leafExists(tree.a, id) || leafExists(tree.b, id);
}

/**
 * Structurally clone a layout tree into plain JS objects. Used when reading
 * a subtree out of the reactive Solid store to splice it back in as a child
 * of a new parent — the Solid proxy makes nested reads live, so naively
 * re-inserting a child reference produces a self-referential tree. The
 * tree is a small, bounded structure (typically a handful of leaves), so
 * a recursive clone is both obvious and cheap.
 */
function cloneTree(tree: LayoutTree): LayoutTree {
  if (tree.kind === 'leaf') {
    return { kind: 'leaf', id: tree.id, pane_type: tree.pane_type };
  }
  return {
    kind: 'split',
    id: tree.id,
    direction: tree.direction,
    ratio: tree.ratio,
    a: cloneTree(tree.a),
    b: cloneTree(tree.b),
  };
}
