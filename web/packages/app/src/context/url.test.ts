import { describe, expect, it, vi } from 'vitest';
import { createUrlResolver, type FetchedUrl } from './url';

const SESSION_ID = 'session-abc';

function makeResolver(overrides: {
  invoke?: ReturnType<typeof vi.fn>;
  hosts?: string[];
  pushToast?: ReturnType<typeof vi.fn>;
}) {
  const invoke = overrides.invoke ?? vi.fn();
  const pushToast = overrides.pushToast ?? vi.fn();
  const typedInvoke = invoke as unknown as NonNullable<
    Parameters<typeof createUrlResolver>[0]['invoke']
  >;
  return {
    resolver: createUrlResolver({
      sessionId: SESSION_ID,
      invoke: typedInvoke,
      allowedHosts: () => overrides.hosts ?? [],
      pushToast,
    }),
    invoke,
    pushToast,
  };
}

describe('createUrlResolver.list', () => {
  it('returns [] when the query is not a URL', async () => {
    const { resolver } = makeResolver({});
    expect(await resolver.list('not a url')).toEqual([]);
  });

  it('raises a toast and returns [] for a disallowed host', async () => {
    const { resolver, pushToast } = makeResolver({
      hosts: ['allowed.example.com'],
    });
    const out = await resolver.list('https://blocked.example.com/page');
    expect(out).toEqual([]);
    expect(pushToast).toHaveBeenCalledWith(
      'warning',
      expect.stringContaining('not allowed'),
    );
  });

  it('returns one candidate for an allowed URL', async () => {
    const { resolver } = makeResolver({ hosts: ['docs.rs'] });
    const out = await resolver.list('https://docs.rs/tokio');
    expect(out).toEqual([
      { category: 'url', label: 'docs.rs/tokio', value: 'https://docs.rs/tokio' },
    ]);
  });
});

describe('createUrlResolver.resolve', () => {
  // F-359: SSRF-adjacent regression — a disallowed host must never reach
  // the Rust-side IPC. The client-side check is advisory but still fires
  // first so we avoid a round-trip (and so the UI toast is immediate).
  it('refuses disallowed URLs at send time and toasts the user', async () => {
    const invoke = vi.fn();
    const pushToast = vi.fn();
    const { resolver } = makeResolver({
      invoke,
      hosts: ['only.example.com'],
      pushToast,
    });
    const block = await resolver.resolve('https://evil.example.com/x');
    expect(invoke).not.toHaveBeenCalled();
    expect(pushToast).toHaveBeenCalledWith('error', expect.stringContaining('Refusing'));
    expect(block.content).toContain('refused');
    expect(block.meta).toEqual({ refused: true });
  });

  it('fetches an allowed URL via the context_fetch_url IPC command', async () => {
    const fetched: FetchedUrl = {
      body: '<<<BEGIN FETCHED URL body>>>\n<html>hi</html>\n<<<END FETCHED URL body>>>',
      status: 200,
      content_type: 'text/html',
      truncated: false,
    };
    const invoke = vi.fn().mockResolvedValue(fetched);
    const { resolver } = makeResolver({
      invoke,
      hosts: ['example.com'],
    });
    const block = await resolver.resolve('https://example.com/page');
    // F-359 critical regression: the resolver must call the Rust IPC
    // command by name with the session-scoped payload — NOT the browser
    // `fetch()`. A refactor that accidentally reintroduces `fetch()` would
    // restore the SSRF-adjacent surface the finding flagged.
    expect(invoke).toHaveBeenCalledWith('context_fetch_url', {
      sessionId: SESSION_ID,
      url: 'https://example.com/page',
    });
    expect(block.type).toBe('url');
    expect(block.path).toBe('https://example.com/page');
    // The body is already wrapped in the dual-LLM containment markers
    // by the Rust side — the webview must not strip or re-wrap them.
    expect(block.content).toContain('<<<BEGIN FETCHED URL body>>>');
    expect(block.content).toContain('<html>hi</html>');
    expect(block.content).toContain('<<<END FETCHED URL body>>>');
  });

  it('surfaces truncation in meta when the Rust side caps the body', async () => {
    const fetched: FetchedUrl = {
      body: '<<<BEGIN FETCHED URL body>>>\n…\n<<<END FETCHED URL body>>>',
      status: 200,
      content_type: null,
      truncated: true,
    };
    const invoke = vi.fn().mockResolvedValue(fetched);
    const { resolver } = makeResolver({ invoke, hosts: ['example.com'] });
    const block = await resolver.resolve('https://example.com/big');
    expect(block.meta).toEqual({ truncated: true });
  });

  it('surfaces IPC errors (e.g. host-not-allowed from Rust) gracefully', async () => {
    // Even though the webview pre-checks the host, the server-side list
    // is authoritative. If it drifts (webview cache stale, or a
    // compromised renderer tries to bypass the local check), the IPC
    // error propagates to the user as a `fetch failed` block — no
    // exception escapes into the composer.
    const invoke = vi.fn().mockRejectedValue(
      new Error('host not on allowed-hosts list: example.com'),
    );
    const { resolver } = makeResolver({ invoke, hosts: ['example.com'] });
    const block = await resolver.resolve('https://example.com/page');
    expect(block.content).toContain('[fetch failed:');
    expect(block.content).toContain('not on allowed-hosts list');
    expect(block.meta).toBeDefined();
  });

  it('surfaces transport errors from the Rust side', async () => {
    const invoke = vi.fn().mockRejectedValue(new Error('offline'));
    const { resolver } = makeResolver({ invoke, hosts: ['example.com'] });
    const block = await resolver.resolve('https://example.com/page');
    expect(block.content).toBe('[fetch failed: offline]');
    expect(block.meta).toEqual({ error: 'offline' });
  });

  // F-359 regression: confirm the resolver never reaches for the
  // browser `fetch()` global, even as a fallback. A direct fetch would
  // reintroduce the finding's surface regardless of the CSP state.
  it('does not invoke the browser fetch global on the resolve path', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockRejectedValue(
      new Error('fetch must not be called in the webview URL resolver'),
    );
    const fetched: FetchedUrl = {
      body: '<<<BEGIN FETCHED URL body>>>\nbody\n<<<END FETCHED URL body>>>',
      status: 200,
      content_type: null,
      truncated: false,
    };
    const invoke = vi.fn().mockResolvedValue(fetched);
    const { resolver } = makeResolver({ invoke, hosts: ['example.com'] });
    await resolver.resolve('https://example.com/page');
    expect(fetchSpy).not.toHaveBeenCalled();
    fetchSpy.mockRestore();
  });
});
