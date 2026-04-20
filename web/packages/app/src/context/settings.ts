// F-142: minimal settings surface for the URL resolver's allowed-hosts list.
//
// A full settings store is out of scope for this task. We expose a signal
// that defaults to an empty list (strictest — every URL is rejected until a
// host is added) and a setter for both tests and a future settings panel.
//
// The URL resolver imports `allowedHosts()` lazily so a host update is
// observed the next time a user types a URL in the picker.

import { createSignal } from 'solid-js';

/**
 * Allowed hosts for the URL context resolver. Exact hostname match — no
 * wildcard or suffix semantics. Keep this tiny; a real settings panel
 * (follow-up) will populate it from `~/.forge/config.toml`.
 */
const [allowedHosts, setAllowedHosts] = createSignal<string[]>([]);

export { allowedHosts, setAllowedHosts };

/**
 * Check whether a URL string is allowed. Returns `false` for malformed URLs
 * or hosts absent from `allowedHosts()`.
 */
export function isUrlAllowed(url: string, hosts: readonly string[] = allowedHosts()): boolean {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    return false;
  }
  // Only http(s) — file:// and data: URLs should never be fetched from the
  // webview on a user's behalf.
  if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') return false;
  return hosts.includes(parsed.hostname);
}

/** Test helper — reset to a clean empty list. */
export function resetAllowedHostsForTesting(): void {
  setAllowedHosts([]);
}
