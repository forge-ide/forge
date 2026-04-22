// F-142 + F-359: URL resolver.
//
// A URL is accepted as a candidate only if its hostname is on the
// `allowedHosts` list (see `./settings.ts`). At resolve time we fetch the
// URL via the Rust-side `context_fetch_url` IPC command, **not** the
// browser `fetch()` API. The Rust path enforces the allowlist server-side,
// rejects non-http(s) schemes, blocks private / loopback / link-local IP
// literals, re-validates every redirect hop, caps the body at 32 KiB, and
// wraps the returned body in the dual-LLM containment markers.
//
// Why replace `fetch()`. F-359 filed the webview's direct `fetch()` as an
// SSRF-adjacent surface: the moment the page CSP's `connect-src` is
// widened to match the user-configured allowed hosts, a compromised or
// prompt-injected renderer could redirect a fetch to a private-range IP,
// pull IAM creds from AWS IMDS or LAN services, and smuggle the response
// body verbatim into the next LLM turn. Routing through an IPC command
// keeps the allowlist on the Rust side (the webview cannot lie about the
// target host) and is a prerequisite to widening the CSP safely.
//
// Disallowed URLs still raise a user-visible toast and return an empty
// string candidate list so the picker shows "No url results".

import { invoke as defaultInvoke } from '../lib/tauri';
import type { FetchedUrl } from '@forge/ipc';
import type { Candidate, ContextBlock, Resolver } from './types';
import { allowedHosts as defaultAllowedHosts, isUrlAllowed } from './settings';
import { pushToast as defaultPushToast } from '../components/toast';

// Re-export so test files can `import type { FetchedUrl }` from this module
// (the local shape is the ts-rs-generated wire type, not a separate spec).
export type { FetchedUrl };

export interface UrlResolverDeps {
  /** Session id — required for the Rust-side authz check. */
  sessionId: string;
  /** Injection seam — defaults to the real Tauri invoker. Tests pass stubs. */
  invoke?: typeof defaultInvoke;
  /** Override the allowed-hosts source (tests). Defaults to the live signal. */
  allowedHosts?: () => readonly string[];
  /** Override the toast sink (tests). Defaults to the global queue. */
  pushToast?: (kind: 'info' | 'warning' | 'error', message: string) => void;
}

export function createUrlResolver(deps: UrlResolverDeps): Resolver<string> {
  const invokeFn = deps.invoke ?? defaultInvoke;
  const hostsFn = deps.allowedHosts ?? (() => defaultAllowedHosts());
  const toast = deps.pushToast ?? defaultPushToast;
  const { sessionId } = deps;

  return {
    async list(query: string): Promise<Candidate[]> {
      // The picker treats a `url` candidate as present only when the query
      // parses as an http(s) URL. Otherwise the tab is empty.
      const trimmed = query.trim();
      if (trimmed.length === 0) return [];
      if (!/^https?:\/\//i.test(trimmed)) return [];
      const hosts = hostsFn();
      if (!isUrlAllowed(trimmed, hosts)) {
        toast(
          'warning',
          `URL host not allowed — add it to settings → allowed hosts before @-referencing it.`,
        );
        return [];
      }
      let parsed: URL;
      try {
        parsed = new URL(trimmed);
      } catch {
        return [];
      }
      return [
        {
          category: 'url' as const,
          label: parsed.hostname + parsed.pathname,
          value: trimmed,
        },
      ];
    },

    async resolve(url: string): Promise<ContextBlock> {
      // Re-check the allowed-hosts list at send time — the setting could
      // have changed between pick and send. The webview-side check is
      // advisory; the Rust IPC command is the authoritative gate.
      if (!isUrlAllowed(url, hostsFn())) {
        toast(
          'error',
          `Refusing to fetch ${url}: host not on allowed-hosts list.`,
        );
        return {
          type: 'url',
          path: url,
          content: `[refused: host not allowed]`,
          meta: { refused: true },
        };
      }
      try {
        const fetched = await invokeFn<FetchedUrl>('context_fetch_url', {
          sessionId,
          url,
        });
        const block: ContextBlock = {
          type: 'url',
          path: url,
          content: fetched.body,
        };
        if (fetched.truncated) block.meta = { truncated: true };
        return block;
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err);
        return {
          type: 'url',
          path: url,
          content: `[fetch failed: ${message}]`,
          meta: { error: message },
        };
      }
    },
  };
}
