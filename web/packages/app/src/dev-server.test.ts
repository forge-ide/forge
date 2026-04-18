import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const appRoot = resolve(__dirname, '..');

describe('pnpm --filter app dev', () => {
  it('package.json defines a dev script that runs vite', () => {
    const pkg = JSON.parse(readFileSync(resolve(appRoot, 'package.json'), 'utf-8'));
    expect(pkg.scripts?.dev).toBe('vite');
  });

  it('vite.config.ts declares the solid plugin and dist outDir', () => {
    const src = readFileSync(resolve(appRoot, 'vite.config.ts'), 'utf-8');
    expect(src).toMatch(/from ['"]vite-plugin-solid['"]/);
    expect(src).toMatch(/outDir:\s*['"]dist['"]/);
  });
});
