import { type Component, Show, createSignal, onCleanup, onMount } from 'solid-js';
import { useParams } from '@solidjs/router';
import { getCurrentWindow } from '@tauri-apps/api/window';
import type { ProviderId, SessionId } from '@forge/ipc';
import {
  getPersistentApprovals,
  onSessionEvent,
  sessionHello,
  sessionSubscribe,
} from '../../ipc/session';
import {
  activeWorkspaceRoot,
  setActiveSessionId,
  setActiveWorkspaceRoot,
  setSessionEvents,
  setSessions,
} from '../../stores/session';
import { pushEvent } from '../../stores/messages';
import { fromRustEvent } from '../../ipc/events';
import { seedPersistentApprovals } from '../../stores/approvals';
import { PaneHeader } from './PaneHeader';
import { ChatPane } from './ChatPane';
import { EditorPane } from '../../panes/EditorPane';
import { usePaneWidth } from '../../layout/usePaneWidth';
import {
  createLayoutStore,
  type LayoutStore,
} from '../../layout/layoutStore';
import { ActivityBar, type ActivityId } from '../../shell/ActivityBar';
import { FilesSidebar } from '../../shell/FilesSidebar';
import './SessionWindow.css';

/**
 * F-126 test seam. Setting this before mounting SessionWindow makes the
 * component use the provided store instead of constructing one via
 * `createLayoutStore(workspaceRoot)`. Production callers never touch this;
 * tests inject a fake store so they can drive the Open -> EditorPane flow
 * without stubbing `read_layouts`/`write_layouts`. Reset to `null` in
 * `afterEach` so cross-test leakage is impossible.
 *
 * Kept as a module-level slot rather than a prop so SessionWindow's public
 * signature remains `Component<RouteSectionProps>` — the `@solidjs/router`
 * `component=` prop can't accept a widened prop type.
 */
let injectedLayoutStore: LayoutStore | null = null;
export function __setInjectedLayoutStoreForTesting(
  store: LayoutStore | null,
): void {
  injectedLayoutStore = store;
}

/**
 * Session window shell — default single-pane layout from
 * docs/ui-specs/layout-panes.md §3.4. Splitters and drag-to-dock are
 * deferred to Phase 2. Subscribes to the session event stream on mount
 * (via the F-020 IPC wrappers) and cleanly detaches on unmount.
 */
export const SessionWindow: Component = () => {
  const params = useParams<{ id: string }>();
  const sessionId = () => params.id as SessionId;

  let unlisten: (() => void) | null = null;
  let mounted = true;

  // F-126: layout store is lazily instantiated once the session's workspace
  // root is known (HelloAck). The FilesSidebar's onOpen handler and the
  // EditorPane slot both read/write through this store. Tests inject a
  // pre-built store via `__setInjectedLayoutStoreForTesting` to bypass
  // `read_layouts` and drive the Open -> EditorPane flow deterministically.
  const preInjected = injectedLayoutStore;
  const [store, setStore] = createSignal<LayoutStore | null>(preInjected);

  onMount(() => {
    const id = sessionId();
    setActiveSessionId(id);
    setSessions(id, { id, state: 'Active' });

    void (async () => {
      try {
        const ack = await sessionHello(id);
        // F-036: remember the workspace root from the HelloAck so the
        // ApprovalPrompt / WhitelistedPill code paths can pass it to the
        // persistent-approval commands without re-querying.
        setActiveWorkspaceRoot(ack.workspace);
        // F-126: spin up the layout store against the session's workspace
        // root (unless a test injected one). `store.load()` hydrates from
        // disk; a missing/corrupt layouts file silently defaults to the
        // single-pane layout so the UI doesn't stall on hello.
        if (preInjected === null) {
          const next = createLayoutStore(ack.workspace);
          try {
            await next.load();
          } catch (loadErr) {
            console.error('read_layouts failed; using default', loadErr);
          }
          if (mounted) setStore(next);
        }
        await sessionSubscribe(id);
        // F-036: seed the per-session whitelist with any persisted
        // workspace/user approvals for the active workspace. Failure here is
        // non-fatal — the session still runs, users just won't see auto-
        // approvals until they reload. Log and continue.
        try {
          const persisted = await getPersistentApprovals(ack.workspace);
          seedPersistentApprovals(id, persisted);
        } catch (seedErr) {
          console.error('get_persistent_approvals failed', seedErr);
        }
      } catch (err) {
        console.error('session_hello/subscribe failed', err);
      }
      const listener = await onSessionEvent((payload) => {
        if (payload.session_id !== id) return;
        setSessionEvents(payload.session_id, {
          lastSeq: payload.seq,
          lastEvent: payload.event,
        });
        // Route typed events to the messages store for ChatPane rendering.
        // `payload.event` is the Rust-serialized `forge_core::Event` shape
        // (`{"type":"user_message",...}`); fromRustEvent adapts it to the
        // store's discriminated union. Non-renderable variants return null.
        const storeEvent = fromRustEvent(payload.event);
        if (storeEvent) pushEvent(payload.session_id, storeEvent);
      });
      if (mounted) {
        unlisten = listener;
      } else {
        // Component unmounted before the async setup completed — detach immediately.
        listener();
      }
    })();
  });

  onCleanup(() => {
    mounted = false;
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
    // Flush any pending layout-store write before we drop the reference so
    // a rapid "open file then navigate away" doesn't lose the active_file
    // update. `flush()` is a no-op when no debounce is pending.
    const s = store();
    if (s) void s.flush();
    setActiveSessionId(null);
    setActiveWorkspaceRoot(null);
  });

  const handleClose = () => {
    try {
      void getCurrentWindow().close();
    } catch (err) {
      console.error('window close failed', err);
    }
  };

  const subject = () => `Session ${sessionId()}`;
  // Phase 1 ships only the Ollama provider; once the active session carries
  // its provider id over IPC (Phase 2), wire it through here so the pill
  // accent follows the live provider per ai-patterns.md §7 (F-091).
  const providerId = (): ProviderId => 'ollama' as ProviderId;
  const providerLabel = () => 'ollama \u00b7 pending';
  const costLabel = () => 'in 0 \u00b7 out 0 \u00b7 $0.00';

  // F-119: observe the pane section's width so the header collapses its
  // chrome below the 320/240 thresholds. The ref is populated synchronously
  // by Solid when the element mounts, which is the resolution order the
  // hook expects. In the jsdom test environment ResizeObserver is absent
  // and the hook degrades to `full` — the existing SessionWindow tests
  // still pass without a fixture.
  const [paneEl, setPaneEl] = createSignal<HTMLElement | null>(null);
  const { compactness } = usePaneWidth(paneEl);

  // F-126: activity-bar + files-sidebar chrome. `activeActivity` is `null`
  // when the sidebar is hidden (default) and an activity id when visible.
  // `Cmd/Ctrl+Shift+E` toggles the files sidebar without routing through
  // the activity bar click handler so the shortcut works even when the
  // activity bar is keyboard-focused elsewhere.
  const [activeActivity, setActiveActivity] = createSignal<ActivityId | null>(null);

  const toggleFiles = (): void => {
    setActiveActivity((prev) => (prev === 'files' ? null : 'files'));
  };

  const onActivitySelect = (id: ActivityId): void => {
    // Only 'files' is wired in F-126. Search/Git are placeholders.
    if (id !== 'files') return;
    toggleFiles();
  };

  const onShortcut = (e: KeyboardEvent): void => {
    // Cmd/Ctrl+Shift+E toggles the files sidebar. Match Mac's `Meta` and
    // Windows/Linux `Ctrl` on the same binding, matching the issue's
    // platform-agnostic spec.
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.shiftKey && (e.key === 'E' || e.key === 'e')) {
      e.preventDefault();
      toggleFiles();
    }
  };

  onMount(() => {
    window.addEventListener('keydown', onShortcut);
  });
  onCleanup(() => {
    window.removeEventListener('keydown', onShortcut);
  });

  // F-126 mandatory-fix: route FilesSidebar Open -> layoutStore -> EditorPane.
  // When `activeEditorFile()` resolves to a non-null path, SessionWindow
  // mounts an EditorPane pinned to that path in place of the chat pane.
  // Closing the editor clears the active file and drops back to chat.
  const onFileOpen = (path: string): void => {
    const s = store();
    if (s === null) {
      // Store not yet loaded (pre-hello). Extremely unlikely because the
      // FilesSidebar is gated on `activeWorkspaceRoot`, but guard anyway.
      console.warn('openFile dropped — layout store not ready', path);
      return;
    }
    s.openFile(path);
  };

  const onEditorClose = (): void => {
    const s = store();
    if (s) s.openFile(null);
  };

  const activeEditorFile = (): string | null => {
    const s = store();
    return s ? s.activeEditorFile() : null;
  };

  return (
    <main class="session-window">
      <div class="session-window__chrome">
        <ActivityBar
          active={activeActivity()}
          onSelect={onActivitySelect}
        />
        <Show when={activeActivity() === 'files' && activeWorkspaceRoot() !== null}>
          <FilesSidebar
            sessionId={sessionId()}
            workspaceRoot={activeWorkspaceRoot() as string}
            onOpen={onFileOpen}
          />
        </Show>
        <section
          class="session-window__pane"
          aria-label="Session pane"
          ref={setPaneEl}
        >
          <PaneHeader
            subject={subject()}
            providerId={providerId()}
            providerLabel={providerLabel()}
            costLabel={costLabel()}
            compactness={compactness()}
            onClose={handleClose}
          />
          <div class="session-window__pane-body">
            <Show
              when={activeEditorFile()}
              fallback={<ChatPane />}
            >
              {(path) => (
                <EditorPane
                  sessionId={sessionId()}
                  path={path()}
                  onClose={onEditorClose}
                />
              )}
            </Show>
          </div>
        </section>
      </div>
    </main>
  );
};
