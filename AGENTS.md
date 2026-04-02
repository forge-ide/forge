# AGENTS.md

> Agent guide for the Forge IDE codebase. Read before making changes.

## Prerequisites

Read before any change: `docs/ARCHITECTURE.md`, `docs/DESIGN.md`, `LATER.md` (deferred — never implement unless asked), `product.json`.

Forge is a VS Code fork. Always find and extend existing VS Code patterns — never replace them. Working against VS Code's architecture compounds badly during upstream merges.

## Layered Architecture

Lower layers cannot import from higher layers. Violating this causes build errors.

```
common    ← Pure TypeScript, no runtime deps
base      ← Utilities, no VS Code concepts
platform  ← Core services (IAIProviderService lives here)
editor    ← Monaco text editor
workbench ← Full IDE shell (canvas, chat panes, MCP panel)
```

When creating a Forge service: UI-dependent → workbench. Pure logic → platform.

## Core Patterns

**DI everywhere.** Services are injected via `@IServiceName` decorators, never `new`'d directly. "Service not found" → check `registerSingleton` calls.

**Events not callbacks.** State changes use `Emitter<T>` / `Event<T>` pairs. Always wrap subscriptions in `this._register(...)` — unregistered listeners leak memory.

**Disposables.** Classes managing resources extend `Disposable`. All subscriptions go through `this._register()`.

**Extension host boundary.** Forge services run in the renderer process. Never use `vscode.workspace.*` / `vscode.window.*` extension APIs in Forge services — use VS Code's internal service APIs instead.

**Import interfaces from `common/`**, never the implementation from `browser/` or `node/`.

## Safe to Touch

Forge additions with no upstream dependencies:

```
src/vs/platform/ai/                     ← AI provider interface + implementations
src/vs/workbench/services/forge/        ← All Forge services
src/vs/workbench/contrib/forgeAI/       ← Activity bar viewlet
extensions/forge-theme/                 ← Default theme
resources/forge/                        ← Brand assets
docs/                                   ← Documentation
```

**Modifiable with care** (understand before changing): `product.json`, `src/vs/workbench/browser/workbench.ts`, `src/vs/workbench/workbench.web.main.ts`.

## Load-Bearing — Read ARCHITECTURE.md First

**Editor group system** (`editorGroupsService.ts`, `editorGroupView.ts`, `editorSplitView.ts`) — quad canvas depends on this. Test layout modes, pane state persistence, and session restore.

**Service registration order** (`workbench.web.main.ts`, `workbench.ts`) — services must register after their dependencies. Wrong order → cryptic runtime errors.

**AI provider interface** (`src/vs/platform/ai/common/aiProvider.ts`) — changes must be additive or update all implementations simultaneously.

**Credential resolution + bootstrap** (`forgeCredentialService.ts`, `forgeProviderBootstrap.ts`) — entry point for all provider initialization. Bootstrap runs at `AfterRestored`, re-runs on config/credential changes.

**MCP tool loop** (`mcpService.ts`) — the core agentic loop. Changes can cause infinite loops or malformed requests. Manual test required.

## Never Change

- `MAX_TURNS` constant (safety limit)
- SecretStorage for API keys (security — keys never written to disk)
- Telemetry disable in `product.json` (privacy commitment)
- Ember status bar color, `--color-text-primary` (#eae6de), font families (brand identity)
- `IAIProvider` interface without updating all implementations
- `upstream-sync` branch (monthly merges only)
- `main` branch directly (all changes via PRs)

## Never Introduce

- Raw hex values in CSS (use `var(--color-*)` tokens)
- New font families or grey values outside the iron scale
- Direct service instantiation (use DI)
- `console.log` (use `ILogService`), synchronous file I/O, `any` type
- Unregistered event listeners
- Decorative animations
- Personal fork URLs in code (use `forge-ide/forge`)
- Telemetry, analytics, or logging of user data/API keys

## Branching

Forking model: `fork → branch off main → PR to forge-ide/forge:main`. Prefixes: `feature/`, `fix/`, `design/`, `docs/`. Upstream org is `forge-ide` — always use upstream URLs in code.

## Code Conventions

**TypeScript:** `noImplicitAny`, `strictNullChecks`, `strictFunctionTypes` — all enforced. Never use `any`.

**Naming:**

| Thing | Convention | Example |
|---|---|---|
| Service interface | `I[Name]Service` | `IAIProviderService` |
| Forge files/classes | `forge[Name].ts` / `Forge[Name]` | `forgeChatInput.ts` |
| Events / Emitters | `on[Event]` / `_on[Event]` | `onDidChangeProviders` |
| Commands / Settings | `forge.[area].[action]` / `forge.[name]` | `forge.layout.quad` |
| CSS tokens | `--color-[category]-[name]` | `--color-surface-2` |

**Errors:** Never swallow silently — re-throw, log via `ILogService`, or handle meaningfully.

**Async:** Use `async/await`. Use `Promise.all()` for parallel ops. Never mix with `.then()` chains.

**Logging:** Use `ILogService` (trace/debug/info/warn/error). Never log secrets or user content.

## UI Changes

All visual decisions must follow `docs/DESIGN.md`. Key rules: design tokens only (no raw hex/px), three fonts only (Barlow Condensed for headings, Barlow for body, Fira Code for code), iron scale for greys, `ember-400` for active/error states, animations only for state changes.

## Testing

Tests live alongside source: `[filename].test.ts`. Runners:
```bash
./scripts/test.sh --run src/vs/.../test.ts   # common/ and browser/ tests
npm run test-node -- --run src/vs/.../test.ts # node/ tests
```

Verify TypeScript compiles cleanly: `npm run compile`. Never suppress errors with `@ts-ignore`.

## Failure Modes

- **Scope creep:** Change only what's necessary. Don't refactor unrelated code.
- **Implementing LATER.md:** Deferred items stay deferred unless explicitly asked.
- **Breaking DI:** Never `new` a service or import implementations directly.
- **Changing token values:** Flag disagreements, don't change unilaterally.
- **Ignoring upstream boundary:** Forge changes go in Forge-namespaced files, not core VS Code files.
- **Over-engineering:** Build for now, defer to `LATER.md`. No speculative abstractions.
