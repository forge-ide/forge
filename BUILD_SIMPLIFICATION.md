# Build System Simplification Plan

> Plan for reducing complexity in Forge's gulp-based build system and completing the migration to esbuild.

## Current State

- **310 gulp tasks** across 12 gulpfile modules, inherited from VS Code upstream
- **esbuild migration is ~90% complete** for core builds via `build/next/index.ts`
- Dev workflow already uses esbuild (`useEsbuildTranspile = true` in `build/buildConfig.ts`)
- CI builds use `core-ci` (esbuild) — the old `core-ci-old` gulp+tsc path has been removed
- Extensions compilation has **not** been migrated to esbuild

## Target Platforms

Forge targets 6 platform/arch combinations:
- macOS: x64, arm64
- Linux: x64, arm64
- Windows: x64, arm64

All platform-specific gulpfiles are load-bearing and must be retained.

---

## Phase 1: Remove Dead Code

Safe to remove with no impact on any build target.

### Files to Delete

| File | Lines | Reason |
| --- | --- | --- |
| `build/gulpfile.scan.ts` | 131 | Debug symbol extraction for Microsoft crash telemetry. Forge has no telemetry. |
| `build/lib/standalone.ts` | ~100 | Only used by Monaco standalone distribution tasks. |
| `build/monaco/` | directory | ~~Monaco standalone distribution config.~~ **Cannot delete** — `build/lib/monaco-api.ts` reads `monaco.d.ts.recipe` during every compile via `MonacoGenerator` in `compilation.ts`. |
| `test/monaco/` | directory | Tests for the Monaco standalone distribution. |

### Partial Removals

| File | What to Remove | What to Keep |
|------|---------------|--------------|
| `build/gulpfile.editor.ts` | `editor-distro`, `extract-editor-src`, `compile-editor-esm`, `monacodts`, `final-editor-resources` tasks | `monacoTypecheckTask` — used in the main compile pipeline via `gulpfile.ts` |
| `package.json` scripts | `download-builtin-extensions-cg` (if unused) | Everything else |

### Must Keep (initially appeared removable but isn't)

| File | Reason |
|------|--------|
| `build/gulpfile.reh.ts` | Powers dev containers, Remote-SSH, Remote-WSL, VS Code Server, tunnels. Core infrastructure, not optional. |
| `remote/` | Runtime dependencies for REH server. |
| `build/lib/monaco-api.ts` | `MonacoGenerator` is wired into `compilation.ts` and runs during every compile/watch. |
| `build/lib/formatter.ts` | Used by hygiene checks in `build/hygiene.ts`. |
| `build/lib/builtInExtensionsCG.ts` | Has a dedicated npm script. |

---

## Phase 2: Complete the esbuild Migration

### Current esbuild Coverage

| Feature | Status | Location |
|---------|--------|----------|
| TypeScript transpilation | Done | `build/next/index.ts` |
| Watch mode | Done | `build/next/index.ts` (--watch) |
| Bundling (desktop, server, server-web, web) | Done | `build/next/index.ts` (--target) |
| NLS extraction/replacement | Done | `build/next/nls-plugin.ts` |
| Private field mangling (`#foo` to `$a`) | Done | `build/next/private-to-property.ts` |
| Minification | Done | esbuild native |
| Source maps | Done | esbuild native, with CDN URL rewriting |
| Resource copying | Done | Per-target pattern matching |

### What Still Uses Gulp

| Feature | File | Blocker |
|---------|------|---------|
| TypeScript-to-TypeScript mangling (rename private members) | `build/lib/mangle/index.ts` | Requires TS language service; **cannot** be done in esbuild |
| Monaco `.d.ts` extraction | `build/lib/monaco-api.ts` | Requires TS compiler API |
| `core-ci-pr` task | `gulpfile.vscode.ts` | Still uses old gulp+tsc path |

### Migration Steps

#### 2a. Migrate `core-ci-pr` to the esbuild path

`core-ci` already uses esbuild. `core-ci-pr` still uses the old gulp+tsc pipeline. Align them so there's a single CI build path.

- In `gulpfile.vscode.ts`, redefine `core-ci-pr` to use `runEsbuildTranspile()` and `runEsbuildBundle()` (same as `core-ci` but without minification or with `--minify=false`)
- Verify PR builds still pass

#### 2b. Migrate extension transpilation to esbuild ✅ Complete (2026-03-25)

`gulpfile.extensions.ts` generates 3 gulp tasks per extension (transpile, compile, watch). The compile and watch tasks already used tsgo; only the transpile tasks still went through `tsb`.

**What changed:**

- `build/next/extensions.ts` (new) — standalone script that transpiles a single extension `src/` to `out/` using `esbuild.transform()`, matching the pattern in `build/next/index.ts`
- `build/lib/esbuild.ts` — added `runEsbuildExtensionTranspile(srcDir, outDir)` that spawns `extensions.ts` as a child process
- `build/gulpfile.extensions.ts` — removed `tsb` import, `createReporter` import, `plumber`/`sourcemaps` imports, `overrideOptions` variable, and `createPipeline()` function (~50 lines); transpile task body replaced with `runEsbuildExtensionTranspile()`

#### 2c. Extract duplicate esbuild wrappers

`runEsbuildTranspile()` and `runEsbuildBundle()` are duplicated between:
- `build/gulpfile.vscode.ts` (lines 165-219)
- `build/gulpfile.vscode.web.ts` (lines 36-65)

Extract to a shared module (e.g., `build/lib/esbuild.ts`) and import from both gulpfiles.

#### 2d. Remove old bundling pipeline ✅ Complete (2026-03-25)

**Deleted:**

| File | Lines | Reason |
| --- | --- | --- |
| ~~`build/lib/treeshaking.ts`~~ | 927 | Zero imports anywhere. Superseded by esbuild's native tree-shaking. |
| ~~`build/lib/optimize.ts`~~ | 295 | Replaced by `runEsbuildBundle()` in `gulpfile.vscode.ts` and `gulpfile.reh.ts`. |
| ~~`build/lib/bundle.ts`~~ | 66 | Only used by `optimize.ts` and a type-only import in `buildfile.ts`. Both removed. |
| ~~`build/lib/nls.ts`~~ | 273 | Gulp NLS stream removed from `compilation.ts`. NLS is now handled entirely by `build/next/nls-plugin.ts` + `finalizeNLS()` at bundle time, which also writes to `out-build/` for backwards compatibility. |

**Additional cleanup in this phase:**

- `gulpfile.vscode.ts`: removed `bundleVSCodeTask`, `minifyVSCodeTask`, `core-ci-old`, dead `else` branch in the per-platform packaging loop, and the `buildfile`/`optimize` imports.
- `gulpfile.reh.ts`: removed all entry-point and resource arrays that fed the old `optimize.bundleTask`; bundle/minify tasks now call `runEsbuildBundle()` directly.
- `buildfile.ts`: removed `IEntryPoint` import from `bundle.ts`; return type is now an inline `{ name: string }`.
- `compilation.ts`: removed `nls.ts` import and the `.pipe(nls.nls(...))` stream step.

#### 2e. TypeScript mangling — accept the boundary

`build/lib/mangle/index.ts` performs cross-file private member renaming using the TypeScript language service. This **cannot** be replicated in esbuild (esbuild doesn't expose the TS AST). This is the one piece that will remain outside the esbuild path.

Options:
1. **Keep it as a pre-pass**: Run mangling on TS source before esbuild transpiles. This is roughly what happens today.
2. **Drop it**: Accept slightly larger bundles. Mangling saves ~5-10% on bundle size. Evaluate whether the complexity is worth the savings for Forge's use case.

---

## Phase 3: Reduce Gulp to Orchestration Only

After Phase 2, gulp's role shrinks to:
- **Orchestration**: `task.series()` / `task.parallel()` to sequence build steps
- **Platform packaging**: Copying files, running platform-specific packagers (InnoSetup, dpkg, rpmbuild)
- **TS mangling pre-pass** (if retained)

At this point, gulp could optionally be replaced with a simple Node script or `npm-run-all`, but the ROI is low — gulp as a task runner is fine once the heavy lifting moves to esbuild.

### What Stays in Gulp Long-Term

| Gulpfile | Reason |
|----------|--------|
| `gulpfile.ts` | Entry point and task orchestration |
| `gulpfile.compile.ts` | Thin wrapper for compile tasks |
| `gulpfile.vscode.ts` | Desktop packaging (all 6 targets) |
| `gulpfile.vscode.web.ts` | Web build orchestration |
| `gulpfile.reh.ts` | Remote extension host packaging |
| `gulpfile.vscode.linux.ts` | Linux deb/rpm/snap packaging |
| `gulpfile.vscode.win32.ts` | Windows installer packaging |
| `gulpfile.extensions.ts` | Extension build orchestration (simplified) |
| `gulpfile.cli.ts` | Rust CLI compilation |
| `gulpfile.hygiene.ts` | Code quality checks |

---

## Summary

| Phase | Effort | Impact |
|-------|--------|--------|
| Phase 1: Remove dead code | Low | ~430 lines + directories removed |
| Phase 2: Complete esbuild migration | Medium-High | ~1,500 lines of old pipeline become removable; 108 extension tasks simplified |
| Phase 3: Gulp as orchestration only | Low | Conceptual clarity; no further code removal needed |
