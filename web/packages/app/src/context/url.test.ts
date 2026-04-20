import { describe, expect, it, vi } from 'vitest';
import { createUrlResolver, URL_RESOLVER_MAX_BYTES } from './url';

describe('createUrlResolver.list', () => {
  it('returns [] when the query is not a URL', async () => {
    const resolver = createUrlResolver({ allowedHosts: () => [] });
    expect(await resolver.list('not a url')).toEqual([]);
  });

  it('raises a toast and returns [] for a disallowed host', async () => {
    const pushToast = vi.fn();
    const resolver = createUrlResolver({
      allowedHosts: () => ['allowed.example.com'],
      pushToast,
    });
    const out = await resolver.list('https://blocked.example.com/page');
    expect(out).toEqual([]);
    expect(pushToast).toHaveBeenCalledWith(
      'warning',
      expect.stringContaining('not allowed'),
    );
  });

  it('returns one candidate for an allowed URL', async () => {
    const resolver = createUrlResolver({
      allowedHosts: () => ['docs.rs'],
    });
    const out = await resolver.list('https://docs.rs/tokio');
    expect(out).toEqual([
      { category: 'url', label: 'docs.rs/tokio', value: 'https://docs.rs/tokio' },
    ]);
  });
});

describe('createUrlResolver.resolve', () => {
  it('refuses disallowed URLs at send time and toasts the user', async () => {
    const pushToast = vi.fn();
    const fetchImpl = vi.fn();
    const resolver = createUrlResolver({
      fetchImpl: fetchImpl as unknown as typeof fetch,
      allowedHosts: () => ['only.example.com'],
      pushToast,
    });
    const block = await resolver.resolve('https://evil.example.com/x');
    expect(fetchImpl).not.toHaveBeenCalled();
    expect(pushToast).toHaveBeenCalledWith('error', expect.stringContaining('Refusing'));
    expect(block.content).toContain('refused');
    expect(block.meta).toEqual({ refused: true });
  });

  it('fetches an allowed URL and returns its body', async () => {
    const fetchImpl = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: () => Promise.resolve('<html>hi</html>'),
    });
    const resolver = createUrlResolver({
      fetchImpl: fetchImpl as unknown as typeof fetch,
      allowedHosts: () => ['example.com'],
    });
    const block = await resolver.resolve('https://example.com/page');
    expect(fetchImpl).toHaveBeenCalledWith('https://example.com/page');
    expect(block.content).toBe('<html>hi</html>');
    expect(block.type).toBe('url');
    expect(block.path).toBe('https://example.com/page');
  });

  it('truncates large response bodies', async () => {
    const big = 'x'.repeat(URL_RESOLVER_MAX_BYTES * 2);
    const fetchImpl = vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      text: () => Promise.resolve(big),
    });
    const resolver = createUrlResolver({
      fetchImpl: fetchImpl as unknown as typeof fetch,
      allowedHosts: () => ['example.com'],
    });
    const block = await resolver.resolve('https://example.com/big');
    expect(block.content).toMatch(/\(truncated at \d+ bytes\)$/);
  });

  it('surfaces non-2xx responses', async () => {
    const fetchImpl = vi.fn().mockResolvedValue({
      ok: false,
      status: 404,
      text: () => Promise.resolve(''),
    });
    const resolver = createUrlResolver({
      fetchImpl: fetchImpl as unknown as typeof fetch,
      allowedHosts: () => ['example.com'],
    });
    const block = await resolver.resolve('https://example.com/missing');
    expect(block.content).toBe('[fetch failed: HTTP 404]');
    expect(block.meta).toEqual({ status: 404 });
  });

  it('catches fetch errors gracefully', async () => {
    const fetchImpl = vi.fn().mockRejectedValue(new Error('offline'));
    const resolver = createUrlResolver({
      fetchImpl: fetchImpl as unknown as typeof fetch,
      allowedHosts: () => ['example.com'],
    });
    const block = await resolver.resolve('https://example.com/page');
    expect(block.content).toBe('[fetch failed: offline]');
    expect(block.meta).toEqual({ error: 'offline' });
  });
});
