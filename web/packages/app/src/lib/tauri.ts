// Thin shim around Tauri's `invoke`. Hot-swappable for tests without pulling
// in @tauri-apps/api at build time.
//
// In a Tauri webview, `window.__TAURI_INTERNALS__.invoke(cmd, args?)` is the
// supported runtime entry. Outside Tauri (vitest, storybook, plain browser),
// the stub rejects so missing wiring surfaces loudly.

export type Invoke = <T = unknown>(command: string, args?: Record<string, unknown>) => Promise<T>;

type TauriInternals = {
  invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
};

const defaultInvoke: Invoke = async (command, args) => {
  const internals = (globalThis as unknown as { __TAURI_INTERNALS__?: TauriInternals })
    .__TAURI_INTERNALS__;
  if (!internals) {
    throw new Error(`tauri invoke unavailable (command=${command})`);
  }
  return internals.invoke(command, args) as Promise<never>;
};

let current: Invoke = defaultInvoke;

export const invoke: Invoke = (command, args) => current(command, args);

// Test seam: swap the underlying invoker. Pass `null` to restore the default.
export function setInvokeForTesting(fn: Invoke | null): void {
  current = fn ?? defaultInvoke;
}
