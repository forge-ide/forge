/// <reference types="vitest" />
import { defineConfig } from 'vite';
import solid from 'vite-plugin-solid';

export default defineConfig({
  plugins: [solid({ ssr: false })],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
  },
  server: {
    port: 5173,
    strictPort: false,
  },
  resolve: {
    conditions: ['development', 'browser'],
  },
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/test-setup.ts'],
    // Playwright specs live in tests/phase1/ and use @playwright/test —
    // keep vitest off them; they run via `pnpm run test:e2e`.
    exclude: ['node_modules', 'dist', 'tests/**'],
    server: {
      deps: {
        inline: [/solid-js/, /@solidjs\/router/, /@solidjs\/testing-library/],
      },
    },
  },
});
