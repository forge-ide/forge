import { describe, expect, it, beforeEach } from 'vitest';
import {
  allowedHosts,
  setAllowedHosts,
  isUrlAllowed,
  resetAllowedHostsForTesting,
} from './settings';

describe('allowedHosts signal', () => {
  beforeEach(() => {
    resetAllowedHostsForTesting();
  });

  it('starts empty', () => {
    expect(allowedHosts()).toEqual([]);
  });

  it('setAllowedHosts updates the signal', () => {
    setAllowedHosts(['example.com']);
    expect(allowedHosts()).toEqual(['example.com']);
  });
});

describe('isUrlAllowed', () => {
  it('rejects non-http(s) schemes', () => {
    expect(isUrlAllowed('file:///etc/passwd', ['passwd'])).toBe(false);
    expect(isUrlAllowed('data:text/plain,hi', [])).toBe(false);
  });

  it('rejects malformed URLs', () => {
    expect(isUrlAllowed('not a url', ['example.com'])).toBe(false);
  });

  it('rejects hosts not on the list', () => {
    expect(isUrlAllowed('https://blocked.example.com', ['allowed.example.com'])).toBe(false);
  });

  it('accepts exact hostname matches', () => {
    expect(isUrlAllowed('https://docs.rs/tokio', ['docs.rs'])).toBe(true);
    expect(isUrlAllowed('http://example.com:8080/x', ['example.com'])).toBe(true);
  });

  it('does not match subdomains implicitly', () => {
    expect(isUrlAllowed('https://api.example.com', ['example.com'])).toBe(false);
  });
});
