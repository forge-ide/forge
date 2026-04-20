/// <reference types="vitest" />
import { defineConfig } from 'vite';

// F-121: isolated iframe hosting Monaco + monaco-languageclient. Runs in its
// own Vite build so Monaco's AMD loader, web workers, and global scope do not
// pollute the parent `app` bundle.
export default defineConfig({
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    target: 'es2022',
  },
  server: {
    port: 5174,
    strictPort: false,
  },
  test: {
    environment: 'jsdom',
    globals: true,
    // Integration/e2e that needs real Monaco goes elsewhere; unit tests here
    // must not import `monaco-editor` or `monaco-languageclient` directly —
    // they exercise `src/protocol.ts` with a stub editor. See README.
    exclude: ['node_modules', 'dist'],
  },
});
