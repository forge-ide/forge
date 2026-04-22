import { describe, expect, it } from 'vitest';
import { execFileSync, spawnSync } from 'node:child_process';
import { readFileSync, unlinkSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';

const repoRoot = resolve(__dirname, '../../../..');
const script = resolve(repoRoot, 'scripts/check-tokens.mjs');
const tokensCss = resolve(repoRoot, 'web/packages/design/src/tokens.css');

describe('scripts/check-tokens.mjs', () => {
  it('exits 0 when tokens.css matches docs/design/token-reference.md', () => {
    // Should not throw
    const out = execFileSync('node', [script], { cwd: repoRoot, encoding: 'utf-8' });
    expect(out).toContain('ok');
  });

  it('exits non-zero when tokens.css drifts from the reference doc', () => {
    const original = readFileSync(tokensCss, 'utf-8');
    const mutated = original.replace('--color-ember-400: #ff4a12;', '--color-ember-400: #000000;');
    writeFileSync(tokensCss, mutated);
    try {
      const result = spawnSync('node', [script], { cwd: repoRoot, encoding: 'utf-8' });
      expect(result.status).not.toBe(0);
      expect(result.stderr + result.stdout).toMatch(/drift|mismatch|--color-ember-400/i);
    } finally {
      writeFileSync(tokensCss, original);
    }
  });

  // F-389: the gate must also scan `.tsx` files for raw px/hex literals inside
  // inline `style={...}` blocks. Inline styling is the escape hatch that lets
  // raw values bypass tokens.css; the gate catches that class of drift.
  it('exits non-zero when a .tsx inline style contains a raw px value', () => {
    const fixture = resolve(
      repoRoot,
      'web/packages/app/src/__f389_rawpx_fixture__.tsx',
    );
    writeFileSync(
      fixture,
      [
        "import type { Component } from 'solid-js';",
        'export const Bad: Component = () => (',
        "  <div style={{ 'min-width': '360px' }} />",
        ');',
        '',
      ].join('\n'),
    );
    try {
      const result = spawnSync('node', [script], { cwd: repoRoot, encoding: 'utf-8' });
      expect(result.status).not.toBe(0);
      expect(result.stderr + result.stdout).toMatch(/__f389_rawpx_fixture__\.tsx/);
      expect(result.stderr + result.stdout).toMatch(/360px/);
    } finally {
      unlinkSync(fixture);
    }
  });

  it('exits non-zero when a .tsx inline style contains a raw #hex color', () => {
    const fixture = resolve(
      repoRoot,
      'web/packages/app/src/__f389_rawhex_fixture__.tsx',
    );
    writeFileSync(
      fixture,
      [
        "import type { Component } from 'solid-js';",
        'export const Bad: Component = () => (',
        "  <div style={{ color: '#ff00aa' }} />",
        ');',
        '',
      ].join('\n'),
    );
    try {
      const result = spawnSync('node', [script], { cwd: repoRoot, encoding: 'utf-8' });
      expect(result.status).not.toBe(0);
      expect(result.stderr + result.stdout).toMatch(/__f389_rawhex_fixture__\.tsx/);
      expect(result.stderr + result.stdout).toMatch(/#ff00aa/i);
    } finally {
      unlinkSync(fixture);
    }
  });
});
