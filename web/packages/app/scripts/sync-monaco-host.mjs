#!/usr/bin/env node
// F-122: copy the built monaco-host bundle into `app/public/monaco-host/`.
//
// Why: Vite serves `public/*` at root in both `pnpm dev` (dev server) and
// `pnpm build` (static bundle). Placing the iframe's static assets under
// `public/monaco-host/` makes `<iframe src="/monaco-host/index.html">`
// resolve identically in dev, production, and the Tauri bundle — without
// introducing a multi-page Vite config or a Tauri custom protocol.
//
// `monaco-host/vite.config.ts` sets `base: './'` so emitted asset URLs in
// `index.html` are relative (`./assets/…`). After we copy the tree to
// `public/monaco-host/`, requests land under `/monaco-host/assets/…` and
// resolve correctly.

import { cpSync, existsSync, rmSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const srcDist = resolve(here, '../../monaco-host/dist');
const destDir = resolve(here, '../public/monaco-host');

if (!existsSync(srcDist)) {
  console.error(
    `sync-monaco-host: expected build output at ${srcDist}; run ` +
      `\`pnpm --filter monaco-host build\` first (the predev/prebuild ` +
      `hooks in package.json do this automatically).`,
  );
  process.exit(1);
}

// Clean destination so removed upstream files don't linger.
if (existsSync(destDir)) {
  rmSync(destDir, { recursive: true, force: true });
}
cpSync(srcDist, destDir, { recursive: true });
console.log(`sync-monaco-host: copied ${srcDist} -> ${destDir}`);
