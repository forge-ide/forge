import { describe, it, expect, vi, beforeEach } from 'vitest';

// We test the mounted-flag logic by exercising createMountedSubscription with
// manually-controlled onMount/onCleanup stubs so we can simulate fast unmount
// without a full Solid render environment.

let mountCbs: Array<() => void> = [];
let cleanupCbs: Array<() => void> = [];

vi.mock('solid-js', () => ({
  onMount: (cb: () => void) => {
    mountCbs.push(cb);
  },
  onCleanup: (cb: () => void) => {
    cleanupCbs.push(cb);
  },
}));

// Import after mock so the module picks up the stubbed solid-js.
const { createMountedSubscription } = await import('./useEventListener');

beforeEach(() => {
  mountCbs = [];
  cleanupCbs = [];
});

describe('createMountedSubscription', () => {
  it('stores the unlisten function when component stays mounted', async () => {
    const unlisten = vi.fn();
    let resolveSetup!: (fn: () => void) => void;
    const setup = () =>
      new Promise<() => void>((res) => {
        resolveSetup = res;
      });

    createMountedSubscription(setup);

    // Simulate mount
    mountCbs.forEach((cb) => cb());
    // Resolve after mount
    resolveSetup(unlisten);
    await Promise.resolve(); // flush microtask

    // Cleanup
    cleanupCbs.forEach((cb) => cb());
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it('calls unlisten immediately on fast unmount (before setup resolves)', async () => {
    const unlisten = vi.fn();
    let resolveSetup!: (fn: () => void) => void;
    const setup = () =>
      new Promise<() => void>((res) => {
        resolveSetup = res;
      });

    createMountedSubscription(setup);

    // Simulate mount then immediate cleanup before setup resolves
    mountCbs.forEach((cb) => cb());
    cleanupCbs.forEach((cb) => cb());

    // Now setup resolves — the listener should be detached immediately
    resolveSetup(unlisten);
    await Promise.resolve();

    expect(unlisten).toHaveBeenCalledOnce();
  });

  it('does not call unlisten twice when cleanup fires after normal detach', async () => {
    const unlisten = vi.fn();
    const setup = () => Promise.resolve(unlisten);

    createMountedSubscription(setup);
    mountCbs.forEach((cb) => cb());
    await Promise.resolve();

    // Normal cleanup
    cleanupCbs.forEach((cb) => cb());
    cleanupCbs.forEach((cb) => cb()); // second cleanup shouldn't double-call
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
