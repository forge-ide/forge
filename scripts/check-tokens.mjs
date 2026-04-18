#!/usr/bin/env node
// Design token drift check.
// Enforces that web/packages/design/src/tokens.css matches the CSS custom
// properties declared inside the ```css fenced block in
// docs/design/token-reference.md (the authoritative source).
//
// Pragmatic deviation from docs/frontend/token-pipeline.md, which names a
// bash version; this Node version is functionally equivalent and avoids a
// shell dependency in CI. Invoked via `pnpm --filter forge-web run check-tokens`
// or directly: `node scripts/check-tokens.mjs`.

import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..');
const refPath = resolve(repoRoot, 'docs/design/token-reference.md');
const cssPath = resolve(repoRoot, 'web/packages/design/src/tokens.css');

/**
 * Extract `--name: value;` declarations from a CSS string, preserving
 * original order and normalising whitespace inside values.
 * @param {string} source
 * @returns {Map<string, string>}
 */
function parseTokens(source) {
  const tokens = new Map();
  const re = /(--[a-z0-9-]+)\s*:\s*([^;]+);/gi;
  let m;
  while ((m = re.exec(source)) !== null) {
    const name = m[1];
    const value = m[2].trim().replace(/\s+/g, ' ');
    tokens.set(name, value);
  }
  return tokens;
}

/** Extract the first ```css fenced block from a markdown document. */
function extractCssBlock(markdown) {
  const match = markdown.match(/```css\n([\s\S]*?)```/);
  if (!match) {
    throw new Error(`No \`\`\`css block found in ${refPath}`);
  }
  return match[1];
}

const refMarkdown = readFileSync(refPath, 'utf-8');
const cssSource = readFileSync(cssPath, 'utf-8');

const referenceTokens = parseTokens(extractCssBlock(refMarkdown));
const cssTokens = parseTokens(cssSource);

const errors = [];

for (const [name, value] of referenceTokens) {
  if (!cssTokens.has(name)) {
    errors.push(`missing in tokens.css: ${name}`);
  } else if (cssTokens.get(name) !== value) {
    errors.push(`value drift: ${name} — reference="${value}" css="${cssTokens.get(name)}"`);
  }
}

for (const name of cssTokens.keys()) {
  if (!referenceTokens.has(name)) {
    errors.push(`extra in tokens.css (not in reference): ${name}`);
  }
}

if (errors.length > 0) {
  console.error('Token drift detected:');
  for (const e of errors) console.error(`  - ${e}`);
  console.error(`\nReference: ${refPath}`);
  console.error(`CSS:       ${cssPath}`);
  process.exit(1);
}

console.log(`ok: ${cssTokens.size} tokens match ${refPath}`);
