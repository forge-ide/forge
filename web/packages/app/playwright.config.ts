import { defineConfig, devices } from '@playwright/test';

// Phase 1 + Phase 2 UAT harness. Runs the Solid app against the Vite dev
// server with a mocked `window.__TAURI_INTERNALS__` layer (see
// tests/phase1/fixtures/tauri-mock.ts). Real-shell UATs that require the
// Tauri binary + a live `forged` daemon (or `tauri-driver`) should be driven
// by a separate webdriverio + tauri-driver harness; those specs currently
// carry `test.skip` with a reference to the corresponding plan section.
//
// Two Playwright projects are exposed:
//   - phase1 — runs `tests/phase1/` (Phase 1 mocked-IPC harness).
//   - phase2 — runs `tests/phase2/` (Phase 2 mocked-IPC + skipped stubs for
//     real-shell-only UATs).
// `pnpm run test:e2e` runs both. `pnpm run test:e2e:phase2` filters to phase2
// (the script invokes `playwright test tests/phase2`, which Playwright
// resolves as a project filter via the `testDir` of each project).

export default defineConfig({
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
      name: 'phase1',
      testDir: './tests/phase1',
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'phase2',
      testDir: './tests/phase2',
      use: { ...devices['Desktop Chrome'] },
    },
  ],
});
