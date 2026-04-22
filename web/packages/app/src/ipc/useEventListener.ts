import { onCleanup, onMount } from 'solid-js';
import type { UnlistenFn } from '@tauri-apps/api/event';

/**
 * Safely registers an async subscription inside a Solid component.
 *
 * Accepts a `setup` thunk returning Promise<UnlistenFn>. Handles the
 * fast-unmount race: if the component unmounts before the promise resolves,
 * the unlisten function is called immediately rather than leaking.
 *
 * Call during component setup (not inside onMount) — it installs its own
 * onMount/onCleanup guards.
 */
export function createMountedSubscription(setup: () => Promise<UnlistenFn>): void {
  let mounted = true;
  let unlisten: UnlistenFn | null = null;

  onMount(() => {
    void (async () => {
      const fn = await setup();
      if (mounted) {
        unlisten = fn;
      } else {
        fn();
      }
    })();
  });

  onCleanup(() => {
    mounted = false;
    unlisten?.();
    unlisten = null;
  });
}
