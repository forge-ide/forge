// F-365: typed wrappers around the `terminal_*` Tauri commands (F-125).
// Every terminal command is gated on the calling webview's label being
// `session-*`; the Rust registry additionally binds each `terminal_id` to
// the spawning webview so cross-session writes are rejected.
// See `forge-shell::ipc` §F-125 for the authority model.

import { invoke } from '../lib/tauri';
import type { TerminalId, TerminalSpawnArgs } from '@forge/ipc';

export type { TerminalId, TerminalSpawnArgs };

/** Spawn a PTY for `args.terminal_id` in the specified working directory. */
export async function terminalSpawn(args: TerminalSpawnArgs): Promise<void> {
  await invoke('terminal_spawn', { args });
}

/**
 * Forward raw bytes to the PTY's stdin. `data` is a UTF-8 encoded byte
 * array produced by `TextEncoder` — xterm.js already encodes keystrokes and
 * CSI sequences, so callers pass `Array.from(new TextEncoder().encode(str))`.
 */
export async function terminalWrite(
  terminalId: TerminalId,
  data: number[],
): Promise<void> {
  await invoke('terminal_write', { terminalId, data });
}

/**
 * Notify the PTY of a window size change (SIGWINCH). Call this after every
 * `FitAddon.fit()` so the child process sees the updated geometry.
 * Non-fatal on failure — the next resize will re-try.
 */
export async function terminalResize(
  terminalId: TerminalId,
  cols: number,
  rows: number,
): Promise<void> {
  await invoke('terminal_resize', { terminalId, cols, rows });
}

/**
 * Terminate the PTY and reap the child process. The Rust-side `Drop` on
 * `TerminalSession` is the ultimate backstop (covers webview crashes), but
 * calling this on unmount ensures deterministic cleanup.
 */
export async function terminalKill(terminalId: TerminalId): Promise<void> {
  await invoke('terminal_kill', { terminalId });
}
