#!/usr/bin/env node
// Raw-`<button>` drift check — DRAFTED, DISABLED (F-398).
//
// Purpose: once `@forge/design` ships `Button` / `IconButton` / `Tab+Tabs` /
// `MenuItem` primitives (Phase 3), forbid raw `<button>` inside
// `web/packages/app/src/` outside the allowlisted row/card-as-button sites.
//
// State today: DRAFTED and NOT WIRED into CI. The primitives don't exist yet,
// so activating this rule would fail the build against the existing 44 raw
// sites. This file exists so the lint is committed and reviewable alongside
// the migration plan; Phase-3 migration PR 4 (cleanup) flips the switch.
//
// To activate in Phase 3:
//   1. Delete or shrink the `ALLOWLIST` below to only true row/card-as-button
//      files (rows 1, 14, 18, 19, 27 from the F-398 catalog).
//   2. Add `node scripts/check-raw-buttons.mjs` to `justfile`'s `check-web`
//      recipe, after `pnpm check-tokens`.
//   3. Add a `pnpm check-raw-buttons` script to `web/package.json`.
//
// Migration plan: `docs/frontend/button-primitives-migration.md`.
// Tests:         `scripts/check-raw-buttons.test.mjs` (unit-tested on fixtures).

import { readdirSync, readFileSync } from 'node:fs';
import { relative, resolve } from 'node:path';

// Allowlist of files that legitimately render a raw `<button>` because the
// content shape (row, card, tree item) isn't cleanly expressible through a
// primitive. Sourced from the F-398 catalog's row/card-as-button callout.
//
// Paths are relative to the repo root, using forward slashes.
export const ALLOWLIST = [
  'web/packages/app/src/shell/StatusBar.tsx',               // row 1 — bg-agent badge
  'web/packages/app/src/components/BranchMetadataPopover.tsx', // row 14 — variant rows
  'web/packages/app/src/routes/AgentMonitor.tsx',           // rows 18, 19 — agent row, trace step
  'web/packages/app/src/routes/Dashboard/SessionsPanel.tsx', // row 27 — session card
];

/** Recursively yield absolute paths of `.tsx` files under `dir`. */
function* walkTsx(dir) {
  let entries;
  try {
    entries = readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (entry.name === 'node_modules' || entry.name.startsWith('.')) continue;
    const full = resolve(dir, entry.name);
    if (entry.isDirectory()) {
      yield* walkTsx(full);
    } else if (entry.isFile() && entry.name.endsWith('.tsx')) {
      yield full;
    }
  }
}

/**
 * Match opening JSX `<button` tags. The trailing boundary is a whitespace
 * char or `>` so we don't false-positive on identifiers like `Button`,
 * `ButtonGroup`, etc. (JSX tag names are case-sensitive; capital `B`
 * components are primitives by convention.)
 */
const rawButton = /<button(?=[\s>])/g;

/**
 * Scan every `.tsx` file under `root`, returning `{ file, line }` for each
 * raw `<button` opening tag in a file not covered by `allowlist`.
 *
 * Exported so `check-raw-buttons.test.mjs` can drive it with tmpdir
 * fixtures without hitting the real repo tree.
 *
 * @param {{ root: string, allowlist?: string[] }} opts
 * @returns {Array<{ file: string, line: number }>}
 */
export function scanTsxSources({ root, allowlist = [] }) {
  const suppressed = new Set(allowlist);
  const findings = [];
  for (const file of walkTsx(root)) {
    const rel = relative(root, file).split('\\').join('/');
    if (suppressed.has(rel)) continue;
    const source = readFileSync(file, 'utf-8');
    rawButton.lastIndex = 0;
    let m;
    while ((m = rawButton.exec(source)) !== null) {
      const upto = source.slice(0, m.index);
      const line = upto.split('\n').length;
      findings.push({ file: rel, line });
    }
  }
  return findings;
}

/** CLI entry point — invoked only when this file is run directly. */
function main() {
  const repoRoot = resolve(new URL('..', import.meta.url).pathname);
  const scanRoot = resolve(repoRoot, 'web/packages/app/src');
  const findings = scanTsxSources({ root: scanRoot, allowlist: ALLOWLIST.map((p) => relative('web/packages/app/src', p).split('\\').join('/')) });
  if (findings.length > 0) {
    console.error(`Raw <button> detected at ${findings.length} site(s):`);
    for (const f of findings) console.error(`  - ${f.file}:${f.line}`);
    console.error('\nUse a @forge/design primitive (Button / IconButton / Tab / MenuItem)');
    console.error('or add the file to ALLOWLIST in scripts/check-raw-buttons.mjs if it');
    console.error('is a row/card-as-button site.');
    process.exit(1);
  }
  console.log('ok: no raw <button> outside allowlisted sites');
}

// Only run the CLI when this file is invoked directly (not when imported by tests).
if (import.meta.url === `file://${process.argv[1]}`) {
  main();
}
