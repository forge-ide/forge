// F-050 / H9 — Restrictive CSP regression.
//
// Asserts that the bootstrap document carries a Content-Security-Policy meta
// tag whose content contains the anchor directives that define the policy's
// defense-in-depth posture against webview XSS:
//
//   - script-src 'self'       — blocks inline + remote scripts
//   - object-src 'none'       — blocks <object>/<embed> plugin injection
//   - frame-ancestors 'none'  — blocks embedding the app in a frame
//
// The Tauri webview reads CSP from tauri.conf.json; the Vite dev server and
// this Playwright harness read it from the <meta> tag in index.html. Both
// must carry the same policy — see the source-of-truth comment in index.html.
//
// This test runs against the Vite dev server (Playwright harness), so only
// the meta-tag path is exercised here. The tauri.conf.json path is covered
// by build verification (pnpm --filter app build) and manual smoke.

import { test, expect } from '@playwright/test';

test.describe('F-050 — restrictive CSP', () => {
  test('bootstrap document exposes CSP meta tag with anchor directives', async ({ page }) => {
    await page.goto('/');

    const meta = page.locator('meta[http-equiv="Content-Security-Policy"]');
    await expect(meta, 'CSP meta tag must be present in index.html').toHaveCount(1);

    const cspContent = await meta.getAttribute('content');
    expect(cspContent).not.toBeNull();
    expect(cspContent).toContain("script-src 'self'");
    expect(cspContent).toContain("object-src 'none'");
    expect(cspContent).toContain("frame-ancestors 'none'");
  });
});
