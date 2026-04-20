// F-142: minimal toast queue.
//
// A real toast system is out of scope — the context picker only needs a
// single "disallowed URL" user-visible notification today. This file exposes
// a signal-backed queue + `pushToast(kind, message)` helper. A `<ToastHost>`
// renderer is a follow-up; tests observe `toasts()` directly, and the
// ephemeral nature of the queue means nothing breaks while the renderer is
// absent (notifications just don't surface visually yet).

import { createSignal } from 'solid-js';

export type ToastKind = 'info' | 'warning' | 'error';

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
  createdAt: number;
}

const [toasts, setToasts] = createSignal<Toast[]>([]);
let nextId = 1;

/** Reactive accessor for mounted renderers and tests. */
export { toasts };

/** Append a toast and return its id. */
export function pushToast(kind: ToastKind, message: string): number {
  const id = nextId++;
  setToasts((prev) => [...prev, { id, kind, message, createdAt: Date.now() }]);
  return id;
}

/** Remove a toast by id. Used by the host component on dismiss/timeout. */
export function dismissToast(id: number): void {
  setToasts((prev) => prev.filter((t) => t.id !== id));
}

/** Test helper — clear the queue between tests. */
export function clearToastsForTesting(): void {
  setToasts([]);
  nextId = 1;
}
