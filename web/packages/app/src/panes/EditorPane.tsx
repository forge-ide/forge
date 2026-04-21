// F-122: EditorPane — hosts the monaco-host iframe (F-121) and bridges its
// lifecycle messages to the session-scoped filesystem commands (`read_file`,
// `write_file`). The iframe runs in isolation so Monaco's workers and AMD
// globals never touch the Solid tree; we talk to it exclusively via
// `window.postMessage`, mirroring the protocol documented in
// `web/packages/monaco-host/src/protocol.ts`.
//
// Header (breadcrumb, dirty-dot, CLOSE button) follows
// `docs/ui-specs/pane-header.md` §PH.1–PH.6 adapted for the EDITOR pane
// type label. The spec's narrow-width / overflow-menu slots are placeholders
// for now — this PR is focused on the IPC bridge + dirty-state signal.
//
// The component is intentionally test-friendly: every side-channel (iframe
// URL, postMessage dispatch, IPC invoke) is injectable through
// `EditorPaneProps`. Vitest runs the component with a stub `src`
// (`about:blank`) and stubbed helpers — Monaco itself is never mounted in
// jsdom (see monaco-host/README for the rationale).

import {
  type Component,
  type JSX,
  createEffect,
  createMemo,
  createSignal,
  onCleanup,
  onMount,
} from 'solid-js';
import type { SessionId } from '@forge/ipc';
import {
  readFile as defaultReadFile,
  writeFile as defaultWriteFile,
} from '../ipc/fs';
import './EditorPane.css';

/** Default URL the iframe loads. Relative so it resolves under both the
 *  Tauri bundle (`tauri://localhost/monaco-host/index.html`) and the Vite
 *  dev server (`/monaco-host/index.html`). The monaco-host package builds
 *  into `web/packages/monaco-host/dist/` and the parent app's
 *  `predev`/`prebuild` hooks copy that tree into
 *  `web/packages/app/public/monaco-host/` so Vite serves it at `/` in dev
 *  and bundles it into `app/dist/monaco-host/` in production. */
export const DEFAULT_MONACO_HOST_SRC = '/monaco-host/index.html';

/** Opaque shape we accept on the wire from the iframe. We do no structural
 *  validation beyond the `kind` tag — the source of truth is the
 *  monaco-host `EditorOutboundMessage` union. Keeping it `unknown` here
 *  avoids a runtime dependency on the monaco-host package (it has no
 *  export surface the parent app can import type-only). */
type IframeMessage =
  | { kind: 'ready' }
  | { kind: 'opened'; uri: string }
  | { kind: 'closed'; uri: string }
  | { kind: 'save'; uri: string; value: string }
  | { kind: 'change'; uri: string; value: string }
  | { kind: 'client.message'; payload: unknown }
  | { kind: 'client.notification'; payload: unknown };

export interface EditorPaneProps {
  sessionId: SessionId;
  /** Absolute path of the file to edit. Required — the pane is one-file-per-
   *  instance today; the tab bar (F-126) will mount multiple EditorPanes. */
  path: string;
  /** Close the pane (parent owns window/tab lifecycle). */
  onClose: () => void;
  /** Override the iframe's `src`. Tests pass `about:blank`; dev / prod use
   *  the default. */
  src?: string;
  /** Injection seam: replace `readFile`/`writeFile` for tests. Defaults to
   *  the real Tauri-invoking helpers. The seam also allows the eventual
   *  F-126 tab bar to cache/mock per-file content without threading through
   *  the module. */
  readFile?: typeof defaultReadFile;
  writeFile?: typeof defaultWriteFile;
  /** Injection seam: called every time we want to post a message to the
   *  iframe. Tests capture these; prod resolves to
   *  `iframe.contentWindow.postMessage`. */
  postToIframe?: (msg: unknown) => void;
  /** F-150: pointer-down handler wired onto the breadcrumb header so the
   *  pane participates in the F-118 drag-to-dock gesture. Callers thread
   *  `dockApi.startDrag(leafId)` through here; the header is the only
   *  non-content surface on the pane and is the same drag-initiation
   *  affordance every other grid leaf uses. */
  onHeaderPointerDown?: (e: PointerEvent) => void;
}

/** Monaco-style URI. Keeps the round-trip through `readFile` → iframe →
 *  `save` event deterministic: the `uri` we pass into `open` is exactly
 *  the `uri` the iframe echoes back in `save`. */
function pathToUri(path: string): string {
  return `file://${path}`;
}

/** Derive the breadcrumb segments from an absolute path. Truncates on the
 *  left if the path is long; the last two segments stay visible. */
export function breadcrumbFromPath(path: string): string[] {
  // Leading slash produces an empty first segment; drop it so the breadcrumb
  // starts at the first real directory.
  return path.split('/').filter((seg) => seg.length > 0);
}

/** Shorten the leaf segment's containing path to the final two segments.
 *  The whole breadcrumb stays available via the title attribute so a user
 *  who wants the full path can hover. */
export function trimmedBreadcrumb(path: string): { prefix: string; leaf: string } {
  const segs = breadcrumbFromPath(path);
  if (segs.length === 0) return { prefix: '', leaf: path };
  const leaf = segs[segs.length - 1] ?? path;
  const tailStart = Math.max(0, segs.length - 3);
  const middle = segs.slice(tailStart, segs.length - 1);
  const prefix =
    tailStart > 0 ? `…/${middle.join('/')}` : middle.join('/');
  return { prefix, leaf };
}

/**
 * EditorPane — monaco-host iframe + chrome. Save is triggered both by the
 * iframe's own save button (if any) and by Cmd/Ctrl+S anywhere inside the
 * pane. Dirty state tracks the diff between the last-saved buffer and the
 * current buffer reported by the iframe's `change` events.
 */
export const EditorPane: Component<EditorPaneProps> = (props) => {
  const readFile = props.readFile ?? defaultReadFile;
  const writeFile = props.writeFile ?? defaultWriteFile;

  let iframeRef: HTMLIFrameElement | undefined;

  const [isDirty, setIsDirty] = createSignal(false);
  const [errorMessage, setErrorMessage] = createSignal<string | null>(null);
  const [, setIsReady] = createSignal(false);
  // Track last-saved contents separately from current to decide dirty.
  let lastSavedValue: string | null = null;
  let currentValue: string | null = null;
  const currentUri = createMemo(() => pathToUri(props.path));

  const postToIframe = (msg: unknown): void => {
    if (props.postToIframe) {
      props.postToIframe(msg);
      return;
    }
    const win = iframeRef?.contentWindow;
    if (win !== null && win !== undefined) {
      win.postMessage(msg, '*');
    }
  };

  const sendOpen = async (): Promise<void> => {
    try {
      const file = await readFile(props.sessionId, props.path);
      lastSavedValue = file.content;
      currentValue = file.content;
      setIsDirty(false);
      setErrorMessage(null);
      postToIframe({
        kind: 'open',
        uri: currentUri(),
        languageId: languageFromPath(props.path),
        value: file.content,
      });
    } catch (err) {
      setErrorMessage(errorToString(err));
    }
  };

  const persist = async (value: string): Promise<void> => {
    try {
      await writeFile(props.sessionId, props.path, value);
      lastSavedValue = value;
      currentValue = value;
      setIsDirty(false);
      setErrorMessage(null);
    } catch (err) {
      setErrorMessage(errorToString(err));
    }
  };

  const requestSave = (): void => {
    // Route through the iframe — it is the source of truth for the buffer
    // contents. The iframe replies with a `save` outbound event carrying
    // the current value, which we then persist.
    postToIframe({ kind: 'save', uri: currentUri() });
  };

  const handleMessage = (event: MessageEvent): void => {
    // Reject messages from windows other than the hosted iframe.
    if (iframeRef && event.source !== iframeRef.contentWindow) return;
    const data = event.data as IframeMessage | undefined;
    if (!data || typeof data !== 'object' || typeof data.kind !== 'string') return;

    switch (data.kind) {
      case 'ready':
        setIsReady(true);
        void sendOpen();
        break;
      case 'change':
        if (data.uri !== currentUri()) return;
        currentValue = data.value;
        setIsDirty(lastSavedValue !== data.value);
        break;
      case 'save':
        if (data.uri !== currentUri()) return;
        void persist(data.value);
        break;
      // `opened` / `closed` are acknowledgements we don't need to act on
      // today; `client.message` / `client.notification` are LSP traffic
      // that F-123 will route to forge-lsp (out of scope here).
      default:
        break;
    }
  };

  const handleKeyDown = (e: KeyboardEvent): void => {
    const isSave = (e.metaKey || e.ctrlKey) && e.key === 's';
    if (!isSave) return;
    e.preventDefault();
    requestSave();
  };

  onMount(() => {
    window.addEventListener('message', handleMessage);
  });

  // Re-open when the path changes mid-life.
  createEffect(() => {
    // Tracked: props.path triggers a re-open. Use `void` to discard the
    // promise — errors are already surfaced through the error signal.
    void props.path;
    if (currentValue !== null) {
      void sendOpen();
    }
  });

  onCleanup(() => {
    window.removeEventListener('message', handleMessage);
    postToIframe({ kind: 'close', uri: currentUri() });
  });

  const { prefix, leaf } = trimmedBreadcrumb(props.path);
  const breadcrumbTitle = (): string => props.path;
  const iframeSrc = (): string => props.src ?? DEFAULT_MONACO_HOST_SRC;

  return (
    <section
      class="editor-pane"
      data-testid="editor-pane"
      onKeyDown={handleKeyDown}
    >
      <header
        class="editor-pane__header"
        role="banner"
        onPointerDown={props.onHeaderPointerDown}
      >
        <span class="editor-pane__type-label">EDITOR</span>
        <span
          class="editor-pane__breadcrumb"
          data-testid="editor-pane-breadcrumb"
          title={breadcrumbTitle()}
        >
          {prefix && <span class="editor-pane__breadcrumb-prefix">{prefix}/</span>}
          <span class="editor-pane__breadcrumb-leaf">{leaf}</span>
          {isDirty() && (
            <span
              class="editor-pane__dirty-dot"
              data-testid="editor-pane-dirty"
              aria-label="unsaved changes"
              role="status"
            />
          )}
        </span>
        <button
          type="button"
          class="editor-pane__close"
          aria-label="Close editor pane"
          onClick={props.onClose}
        >
          CLOSE PANE
        </button>
      </header>
      {errorMessage() !== null && (
        <div
          class="editor-pane__error"
          role="alert"
          data-testid="editor-pane-error"
        >
          {errorMessage()}
        </div>
      )}
      <iframe
        ref={iframeRef}
        class="editor-pane__iframe"
        data-testid="editor-pane-iframe"
        src={iframeSrc()}
        title="Monaco editor host"
        // Sandboxing intentionally allows scripts + same-origin — the
        // iframe is first-party and runs the trusted monaco-host bundle.
        sandbox="allow-scripts allow-same-origin"
        style={iframeStyle()}
      />
    </section>
  );
};

function iframeStyle(): JSX.CSSProperties {
  // Fill the remaining pane space under the header. Inline rather than in
  // CSS because the header height is defined in the same CSS module and
  // the iframe style depends on flex layout there.
  return { flex: '1 1 auto', border: '0', width: '100%' };
}

function languageFromPath(path: string): string {
  const dot = path.lastIndexOf('.');
  if (dot < 0) return 'plaintext';
  const ext = path.slice(dot + 1).toLowerCase();
  switch (ext) {
    case 'ts':
    case 'tsx':
      return 'typescript';
    case 'js':
    case 'jsx':
      return 'javascript';
    case 'rs':
      return 'rust';
    case 'toml':
      return 'toml';
    case 'json':
      return 'json';
    case 'md':
      return 'markdown';
    case 'css':
      return 'css';
    case 'html':
      return 'html';
    case 'py':
      return 'python';
    default:
      return 'plaintext';
  }
}

function errorToString(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (typeof err === 'string') return err;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}
