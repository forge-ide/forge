/**
 * TerminalPane — xterm.js renderer on top of the `forge-term` byte stream
 * (F-125, builds on F-124). One pane owns one `TerminalSession` on the Rust
 * side, addressed by a client-generated `terminal_id`. The pane:
 *
 *   1. Spawns the PTY via `terminal_spawn` on mount.
 *   2. Forwards every xterm.js `onData` frame to `terminal_write`.
 *   3. Subscribes to the `terminal:bytes` event and pipes chunks back into
 *      xterm's parser unchanged (F-124's README "Two-layer VT state"
 *      discipline — bytes pass through, xterm.js owns the VT state).
 *   4. Uses `@xterm/addon-fit` + a `ResizeObserver` to keep the PTY window
 *      size in sync with the pane's CSS dimensions.
 *   5. Calls `terminal_kill` on cleanup so the child is reaped before the
 *      webview detaches (Rust-side `Drop` would also handle this if the
 *      webview crashes outright).
 *
 * Authz: every `terminal_*` command is gated on the calling webview's label
 * being `session-*`; the Rust registry additionally binds each `terminal_id`
 * to the spawning webview's label so cross-session writes are rejected.
 * See `forge-shell::ipc` §F-125.
 */
import { Component, createSignal, onCleanup, onMount, Show } from 'solid-js';
import { Terminal } from '@xterm/xterm';
import { FitAddon } from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  TerminalBytesEvent,
  TerminalExitEvent,
  TerminalId,
  TerminalSpawnArgs,
} from '@forge/ipc';
import { invoke } from '../lib/tauri';
import { PaneHeader } from '../routes/Session/PaneHeader';
import './TerminalPane.css';

export const TERMINAL_BYTES_EVENT = 'terminal:bytes';
export const TERMINAL_EXIT_EVENT = 'terminal:exit';

/**
 * Generate a fresh 16-hex-char terminal id matching the Rust `TerminalId`
 * wire shape. Uses `crypto.getRandomValues` so the entropy is cryptographic;
 * the id space is per-window so 64 bits is plenty.
 *
 * Kept inline (vs. a shared helper) because the only other id generator on
 * the web side is Rust-originated, and pulling in a dependency just for this
 * would be overkill.
 */
export function newTerminalId(): TerminalId {
  const bytes = new Uint8Array(8);
  crypto.getRandomValues(bytes);
  const hex = Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
  return hex as TerminalId;
}

export interface TerminalPaneProps {
  /** Working directory for the spawned shell. */
  cwd: string;
  /**
   * Optional shell program override (e.g. `/bin/zsh`). When omitted, the
   * Rust side resolves `$SHELL` → `/bin/sh` / `cmd.exe`.
   */
  shell?: string;
  /**
   * Display name for the shell in the pane header's subject slot. Defaults
   * to a trailing-path-component of `shell` or the literal string `shell`.
   */
  shellName?: string;
  /** Invoked when the user clicks the pane-header close button. */
  onClose: () => void;
}

/**
 * Derive a subject label from a shell path (`/bin/zsh` → `zsh`). Exported
 * for unit tests; the empty-string / undefined fallbacks keep the header
 * non-empty in pathological configurations.
 */
export function shellDisplayName(shell: string | undefined): string {
  if (!shell) return 'shell';
  const trimmed = shell.trim();
  if (!trimmed) return 'shell';
  const slash = trimmed.lastIndexOf('/');
  const base = slash >= 0 ? trimmed.slice(slash + 1) : trimmed;
  return base || 'shell';
}

export const TerminalPane: Component<TerminalPaneProps> = (props) => {
  const [terminalId] = createSignal<TerminalId>(newTerminalId());
  const [spawnError, setSpawnError] = createSignal<string | null>(null);

  let hostRef: HTMLDivElement | undefined;
  let term: Terminal | undefined;
  let fitAddon: FitAddon | undefined;
  let unlistenBytes: UnlistenFn | null = null;
  let unlistenExit: UnlistenFn | null = null;
  let resizeObserver: ResizeObserver | null = null;
  let termMounted = true;
  // Tracks whether spawn succeeded so cleanup doesn't attempt to kill a
  // terminal that was never registered on the Rust side.
  let spawnCompleted = false;
  // Flip before we call `terminal_kill` on our own initiative so the
  // `terminal:exit` event listener knows not to surface the exit as an
  // unexpected failure.
  let tearingDown = false;

  onMount(() => {
    if (!hostRef) return;

    // Theme + font are intentionally left at xterm.js defaults for Phase 1.
    // Forge's terminal-theme tokens (docs/design) will land in a follow-up;
    // doing it here would couple F-125 to an unshipped token surface.
    term = new Terminal({
      convertEol: true,
      cursorBlink: true,
      fontFamily:
        'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace',
      fontSize: 13,
    });
    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(hostRef);

    // Size the terminal to the host element *before* spawning so the PTY
    // starts out at the correct dimensions (otherwise early output can be
    // wrapped at the default 80x24 and reshaped on first resize).
    try {
      fitAddon.fit();
    } catch {
      // fit() throws on hosts with zero dimensions (e.g. during jsdom
      // unit-test mounts). Fall back to the xterm default; the real fit
      // will happen on the next ResizeObserver tick.
    }
    const initialCols = term.cols || 80;
    const initialRows = term.rows || 24;
    const id = terminalId();

    // Forward local keystrokes to the PTY. xterm.js already encodes them in
    // the terminal's input modes (including function keys / CSI sequences),
    // so we pass the string bytes through without interpretation.
    term.onData((data) => {
      const bytes = Array.from(new TextEncoder().encode(data));
      void invoke('terminal_write', { terminalId: id, data: bytes }).catch(
        (err) => {
          // Surface repeated write failures in the pane itself — the user
          // otherwise sees a silent dead terminal.
          setSpawnError(`terminal_write failed: ${String(err)}`);
        },
      );
    });

    void (async () => {
      // Subscribe before spawning so we cannot miss bytes emitted between
      // spawn return and subscription set-up.
      const bytesOff = await listen<TerminalBytesEvent>(
        TERMINAL_BYTES_EVENT,
        (event) => {
          if (event.payload.terminal_id !== id) return;
          if (!term) return;
          term.write(new Uint8Array(event.payload.data));
        },
      );
      if (!termMounted) { bytesOff(); return; }
      unlistenBytes = bytesOff;

      const exitOff = await listen<TerminalExitEvent>(
        TERMINAL_EXIT_EVENT,
        (event) => {
          if (event.payload.terminal_id !== id) return;
          // Don't overwrite an earlier error; just note the exit so the
          // renderer can show the user the child is gone.
          const code = event.payload.code;
          if (!tearingDown && !event.payload.killed_by_drop) {
            // Natural exit or a signal — show a faint trailer. We write
            // directly into xterm rather than raising a Solid notice because
            // the byte stream is the user's source of truth for what happened
            // (the trailer lives under the shell's goodbye line).
            const trailer =
              code === null || code === undefined
                ? '\r\n[terminal exited]'
                : `\r\n[terminal exited: code ${code}]`;
            term?.write(trailer);
          }
          // The terminal is gone on the Rust side now; mark cleanup so we
          // don't redundantly `terminal_kill`.
          spawnCompleted = false;
        },
      );
      if (!termMounted) { exitOff(); return; }
      unlistenExit = exitOff;

      const args: TerminalSpawnArgs = {
        terminal_id: id,
        shell: props.shell ?? null,
        cwd: props.cwd,
        cols: initialCols,
        rows: initialRows,
      };
      try {
        await invoke('terminal_spawn', { args });
        spawnCompleted = true;
      } catch (err) {
        setSpawnError(`terminal_spawn failed: ${String(err)}`);
        return;
      }
    })();

    // Resize observer — refit xterm, then push the new dimensions to the PTY
    // so SIGWINCH reaches the child. Throttle via rAF to coalesce bursts
    // during a drag-to-resize.
    let pendingResize = false;
    const handleResize = () => {
      if (!term || !fitAddon || !spawnCompleted) return;
      if (pendingResize) return;
      pendingResize = true;
      requestAnimationFrame(() => {
        pendingResize = false;
        if (!term || !fitAddon) return;
        try {
          fitAddon.fit();
        } catch {
          return;
        }
        const cols = term.cols;
        const rows = term.rows;
        void invoke('terminal_resize', {
          terminalId: id,
          cols,
          rows,
        }).catch((err) => {
          // Non-fatal — the next resize will re-try.
          console.warn('terminal_resize failed', err);
        });
      });
    };
    if (typeof ResizeObserver !== 'undefined') {
      resizeObserver = new ResizeObserver(handleResize);
      resizeObserver.observe(hostRef);
    }
  });

  onCleanup(() => {
    termMounted = false;
    tearingDown = true;
    const id = terminalId();
    if (resizeObserver) {
      resizeObserver.disconnect();
      resizeObserver = null;
    }
    if (unlistenBytes) {
      unlistenBytes();
      unlistenBytes = null;
    }
    if (unlistenExit) {
      unlistenExit();
      unlistenExit = null;
    }
    if (spawnCompleted) {
      // Fire-and-forget — the unmount already happened and any error is just
      // diagnostic now. Rust-side `Drop` on the `TerminalSession` is the
      // ultimate backstop (covers webview crashes).
      void invoke('terminal_kill', { terminalId: id }).catch((err) => {
        console.warn('terminal_kill failed', err);
      });
    }
    if (term) {
      term.dispose();
      term = undefined;
    }
  });

  const subject = () => props.shellName ?? shellDisplayName(props.shell);

  return (
    <section class="terminal-pane" data-testid="terminal-pane">
      <PaneHeader
        typeLabel="TERMINAL"
        subject={subject()}
        costLabel={props.cwd}
        closeLabel="CLOSE PANE"
        closeAriaLabel="Close pane"
        onClose={props.onClose}
      />
      <div class="terminal-pane__body">
        <Show when={spawnError()}>
          <div class="terminal-pane__error" role="alert">
            {spawnError()}
          </div>
        </Show>
        <div
          class="terminal-pane__host"
          data-testid="terminal-pane-host"
          ref={hostRef}
        />
      </div>
    </section>
  );
};
