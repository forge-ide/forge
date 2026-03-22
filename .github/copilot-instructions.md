# Forge IDE — Copilot / Agent Instructions

## Project Overview

Forge IDE is an AI-native fork of VS Code built with TypeScript, Electron, and web APIs. It extends VS Code's architecture with a multi-pane canvas layout, a provider-agnostic AI layer, MCP (Model Context Protocol) integration, and an agentic task system.

**Read [AGENT.md](../.claude/CLAUDE.md) before making any changes.** It is the authoritative source on architecture, safe areas to modify, load-bearing systems, and things that must never be touched.

---

## Repository Layout

- `src/vs/base/` — Foundation utilities, no runtime dependencies
- `src/vs/platform/` — Core services: DI, files, storage, config. AI provider interface lives here (`src/vs/platform/ai/`)
- `src/vs/editor/` — Monaco text editor
- `src/vs/workbench/` — Full IDE shell: canvas, chat panes, MCP panel, agent UI
  - `workbench/browser/forge/` — Forge-specific UI additions
  - `workbench/services/forge/` — Forge services (MCP, layout, agents)
  - `workbench/contrib/` — Feature contributions (git, debug, search, terminal, etc.)
- `src/vs/sessions/` — Agent sessions workbench layer (may import from `workbench/`, not vice versa)
- `extensions/` — Built-in extensions; `extensions/forge-theme/` is Forge's default theme
- `scripts/` — Development launch scripts
- `resources/` — Brand assets and icons
- `product.json` — Forge identity overrides (name, icons, URLs, data dirs)

---

## Architecture Principles

### Layered dependency rule

```
common → base → platform → editor → workbench
```

Lower layers cannot import from higher layers. A file in `platform/` cannot import from `workbench/`. Violating this causes build errors.

### Dependency injection everywhere

Services are never instantiated directly. They are declared as decorated constructor parameters:

```typescript
constructor(
  @IAIProviderService private readonly aiService: IAIProviderService,
  @IFileService private readonly fileService: IFileService,
) {}
```

If a service is missing, it isn't registered yet — check `registerSingleton` calls in `workbench.web.main.ts`.

### Events over callbacks

State changes use `Emitter<T>` / `Event<T>` pairs. Always wrap subscriptions in `this._register(...)` to prevent memory leaks. Never skip this.

### Disposables

Classes managing resources extend `Disposable` and use `this._register()` for all subscriptions, timers, and child disposables.

---

## Validating TypeScript Changes

**Always check for compilation errors before running tests or declaring work complete.**

- Check the `Forge - Build` watch task output for compilation errors (if task tooling is available).
- Otherwise run `npm run compile-check-ts-native` after changes to `src/`.
- For changes to `extensions/`, run `npm run gulp compile-extensions`.
- For changes to `build/`, run `npm run typecheck` in the `build/` folder.
- Run `npm run valid-layers-check` to check for layering violations.

Run tests with:
```bash
scripts/test.sh                          # unit tests (macOS/Linux)
scripts\test.bat                         # unit tests (Windows)
scripts/test-integration.sh              # integration tests
```

---

## Coding Guidelines

### TypeScript strictness

- `noImplicitAny: true` — never use `any`; find the actual type
- `strictNullChecks: true` — always handle `undefined` and `null`
- Never suppress errors with `@ts-ignore` or `@ts-expect-error` unless the existing file already uses this pattern

### Naming

| Thing | Convention | Example |
|---|---|---|
| Service interface | `I[Name]Service` | `IAIProviderService` |
| Forge files | `forge[Name].ts` | `forgeChatInput.ts` |
| Forge classes | `Forge[Name]` | `ForgeChatInput` |
| Events | `on[EventName]` | `onProviderChanged` |
| Emitters (private) | `_on[EventName]` | `_onProviderChanged` |
| Commands | `forge.[area].[action]` | `forge.layout.quad` |
| Settings | `forge.[name]` | `forge.streaming` |
| CSS tokens | `--color-[category]-[name]` | `--color-surface-2` |

### Indentation and style

- Tabs, not spaces
- Arrow functions over anonymous function expressions
- Always wrap loop and conditional bodies in curly braces
- `async/await` throughout — do not mix with `.then()/.catch()` chains
- `Promise.all()` for parallel async operations, not sequential awaits
- Prefer `export function x(…)` over `export const x = (…) =>` in top-level scopes

### Logging

Use `ILogService`, not `console.*`. Never log API keys, user message content, or file contents.

### Error handling

Never swallow errors silently. Every `catch` must re-throw, log via `ILogService`, or handle meaningfully. An empty `catch` block is always wrong.

---

## Design System Rules

All UI code must follow [DESIGN.md](../DESIGN.md).

- **Colors**: `var(--color-*)` tokens only — no raw hex values
- **Fonts**: Barlow Condensed (headings, uppercase), Barlow (body), Fira Code (code/identifiers) — no other fonts
- **Spacing**: `var(--sp-*)` tokens only — no raw pixel values
- **Border radius**: `var(--r-sm/md/lg)` — not hardcoded values
- **Active state**: `iron-750` background + `ember-400` indicator
- **Disabled state**: `iron-600` text — never `opacity` reduction
- **Animations**: state communication only — never decorative
- **Error state**: `ember-400` — never a different red

Provider accent colors: Anthropic = ember, OpenAI = amber, Local = steel.

---

## What Not to Do

- Do not change `MAX_TURNS` in the agent tool loop
- Do not add telemetry, analytics, or any calls to external services beyond the configured AI provider or MCP servers
- Do not store secrets via `IStorageService` — use `ISecretStorage`
- Do not import service implementations directly — always use the interface via DI
- Do not add raw hex values, new font families, or new grey values outside the iron scale
- Do not implement anything listed in `LATER.md`
- Do not use synchronous file I/O — always use `IFileService` async methods
