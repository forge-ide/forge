// F-157 / F-153 loop close: register the built-in commands that ship with
// the app. Today that's just "Open Agent Monitor" — F-153 (#303) deferred
// this registration to F-157 because the palette didn't exist yet.
//
// MUST be called from inside a Solid component that is itself inside the
// `<Router>` subtree, because `useNavigate()` reads the router context. See
// `App.tsx` for the call site.
//
// Route target is `/agents` (not the DoD's proposed `/agent-monitor`): the
// agent-monitor route registered by F-140 is `/agents` / `/agents/:id`, and
// F-153 landed its navigation paths against that route. Navigating to
// `/agent-monitor` would 404. The F-153 PR body documented the same
// precedent and the reviewer accepted it.

import { useNavigate } from '@solidjs/router';
import { registerCommand } from './registry';

/**
 * Register built-in commands. Returns a disposer that unregisters all of
 * them (useful in tests and HMR — not needed in production).
 */
export function registerBuiltins(): () => void {
  const navigate = useNavigate();

  const disposers: Array<() => void> = [];

  disposers.push(
    registerCommand({
      id: 'open-agent-monitor',
      title: 'Open Agent Monitor',
      run: () => navigate('/agents'),
    }),
  );

  return () => {
    for (const d of disposers) d();
  };
}
