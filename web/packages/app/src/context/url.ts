// F-142: URL resolver.
//
// A URL is accepted as a candidate only if its hostname is on the
// `allowedHosts` list (see `./settings.ts`). At resolve time we fetch the
// URL with `fetch()` from the webview — no new IPC command was added for
// this task. Disallowed URLs raise a user-visible toast and return an empty
// string candidate list so the picker shows "No url results".
//
// Truncation mirrors the file resolver's 32 KiB budget: a fetched page's
// body beyond that is dropped with a visible marker.

import type { Candidate, ContextBlock, Resolver } from './types';
import { allowedHosts as defaultAllowedHosts, isUrlAllowed } from './settings';
import { pushToast as defaultPushToast } from '../components/toast';
import { truncateToBytes } from './file';

export const URL_RESOLVER_MAX_BYTES = 32 * 1024;

export interface UrlResolverDeps {
  /** Injection seam — defaults to the browser `fetch`. Tests pass stubs. */
  fetchImpl?: typeof fetch;
  /** Override the allowed-hosts source (tests). Defaults to the live signal. */
  allowedHosts?: () => readonly string[];
  /** Override the toast sink (tests). Defaults to the global queue. */
  pushToast?: (kind: 'info' | 'warning' | 'error', message: string) => void;
}

export function createUrlResolver(deps: UrlResolverDeps = {}): Resolver<string> {
  const fetchFn = deps.fetchImpl ?? fetch.bind(globalThis);
  const hostsFn = deps.allowedHosts ?? (() => defaultAllowedHosts());
  const toast = deps.pushToast ?? defaultPushToast;

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
      // have changed between pick and send.
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
        const res = await fetchFn(url);
        if (!res.ok) {
          return {
            type: 'url',
            path: url,
            content: `[fetch failed: HTTP ${res.status}]`,
            meta: { status: res.status },
          };
        }
        const body = await res.text();
        return {
          type: 'url',
          path: url,
          content: truncateToBytes(body, URL_RESOLVER_MAX_BYTES),
        };
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
