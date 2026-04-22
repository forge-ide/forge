#!/usr/bin/env node
// Design token drift check.
// Enforces that web/packages/design/src/tokens.css matches the CSS custom
// properties declared inside the ```css fenced block in
// docs/design/token-reference.md (the authoritative source).
//
// Documented in docs/frontend/generation-pipelines.md (§1, "Design tokens").
// Invoked via `pnpm --filter forge-web run check-tokens` or directly:
// `node scripts/check-tokens.mjs`.

import { readdirSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, relative, resolve } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(here, '..');
const refPath = resolve(repoRoot, 'docs/design/token-reference.md');
const cssPath = resolve(repoRoot, 'web/packages/design/src/tokens.css');
// Scope for the inline-style scan (F-389): the webview's own TSX sources.
// Any raw px/hex value inside a JSX `style={...}` block here should live in
// tokens.css or a `.css` class instead.
const tsxScanRoots = [
  resolve(repoRoot, 'web/packages/app/src'),
  resolve(repoRoot, 'web/packages/design/src'),
];

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

// ---------------------------------------------------------------------------
// F-389: scan .tsx files for raw px/hex literals inside JSX `style={...}`
// blocks. Inline styling is the escape hatch that lets raw values bypass
// tokens.css, so the gate must cover it.
//
// Heuristic: `\d+px` / `\d*\.\d+px` matches *adjacent* digit+unit pairs only,
// so template-literal interpolations like `${expr}px` (runtime-computed
// positions) don't trip it. Hex: `#[0-9a-fA-F]{3,8}` catches static colors.
// ---------------------------------------------------------------------------

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
 * Yield every JSX `style={...}` block body (the text between the outer
 * braces) as `{ body, offset }` where `offset` is the start index in the
 * source. Handles nested braces inside object literals / template strings.
 */
function* extractStyleBlocks(source) {
  const re = /style\s*=\s*\{/g;
  let m;
  while ((m = re.exec(source)) !== null) {
    const start = m.index + m[0].length; // position just after the opening `{`
    let depth = 1;
    let i = start;
    while (i < source.length && depth > 0) {
      const ch = source[i];
      if (ch === '{') depth += 1;
      else if (ch === '}') depth -= 1;
      i += 1;
    }
    if (depth === 0) yield { body: source.slice(start, i - 1), offset: start };
  }
}

/** 1-based (line, column) for a character offset in `source`. */
function locate(source, offset) {
  let line = 1;
  let col = 1;
  for (let i = 0; i < offset; i += 1) {
    if (source[i] === '\n') {
      line += 1;
      col = 1;
    } else {
      col += 1;
    }
  }
  return { line, col };
}

const rawPx = /\d+(?:\.\d+)?px/;
const rawHex = /#[0-9a-fA-F]{3,8}\b/;

for (const root of tsxScanRoots) {
  for (const file of walkTsx(root)) {
    const source = readFileSync(file, 'utf-8');
    for (const { body, offset } of extractStyleBlocks(source)) {
      const pxMatch = body.match(rawPx);
      const hexMatch = body.match(rawHex);
      const rel = relative(repoRoot, file);
      if (pxMatch) {
        const { line } = locate(source, offset + pxMatch.index);
        errors.push(
          `raw px in inline style (use tokens.css or a CSS class): ${rel}:${line} — ${pxMatch[0]}`,
        );
      }
      if (hexMatch) {
        const { line } = locate(source, offset + hexMatch.index);
        errors.push(
          `raw hex in inline style (use tokens.css or a CSS class): ${rel}:${line} — ${hexMatch[0]}`,
        );
      }
    }
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
