import { defineConfig, devices } from '@playwright/test';

// Phase 1 UAT harness. Runs the Solid app against the Vite dev server with a
// mocked `window.__TAURI_INTERNALS__` layer (see tests/phase1/fixtures/tauri-mock.ts).
// Real-shell UATs (UAT-01a/01b/03/12B) that require the Tauri binary + a live
// `forged` daemon should be driven by a separate webdriverio + tauri-driver
// harness; those specs currently carry `test.skip` with a reference to
// `docs/testing/phase1-uat.md`.

export default defineConfig({
  testDir: './tests/phase1',
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: false,
  workers: 1,
  reporter: [['list'], ['html', { outputFolder: 'playwright-report', open: 'never' }]],
  use: {
    baseURL: 'http://127.0.0.1:5173',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
  webServer: {
    // Bind to 127.0.0.1 explicitly — on Linux + Vite 6, the default `localhost`
    // binding often resolves to ::1 (IPv6) only, which Playwright's IPv4 baseURL
    // can't reach.
    command: 'pnpm exec vite --host 127.0.0.1 --port 5173 --strictPort',
    url: 'http://127.0.0.1:5173',
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
