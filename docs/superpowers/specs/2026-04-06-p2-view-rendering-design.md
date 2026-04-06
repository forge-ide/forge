# P2 View Rendering — Design Spec

**Date:** 2026-04-06  
**Scope:** Test Plan Phase 4 — Section 10 P2 (Agent Monitor View DOM, MCP Status View DOM)  
**Approach:** Extract render helpers from `ViewPane` subclasses into companion helper files

---

## Problem

`ForgeAgentMonitorView` and `ForgeMcpStatusView` extend VS Code's `ViewPane`, which requires the full DI container to instantiate. This blocks unit testing of their render logic (test plan items 5.1–5.12 and 6.1–6.7).

The render helpers (`renderDefinitionRow`, `renderAgentRow`, `renderServerRow`) are private methods on the view classes, tightly coupled via `this.agentService` / `this.forgeMcpService` closures.

---

## Approach: Companion Helper Files

Extract render logic into standalone exported functions in companion files. Functions take data objects and callbacks — no DI, no `this`. The view classes call through to these functions. Tests import helpers directly.

This follows the existing `createSvgIcon` pattern in `forgeAIWorkspaceView.ts`.

---

## New Files

```
src/vs/workbench/contrib/forgeAI/browser/
  forgeAgentMonitorViewHelpers.ts    ← new
  forgeMcpStatusViewHelpers.ts       ← new
```

No changes to test file locations — existing stubs are expanded.

---

## forgeAgentMonitorViewHelpers.ts

### Exports

```typescript
/** Maps ForgeAgentStatus enum → CSS class string for the status dot */
export function getAgentStatusClass(status: ForgeAgentStatus): string

/** Stable sort: Running → Queued → Completed → MaxTurnsReached → Error */
export function sortAgentsByStatus(agents: ForgeAgentTask[]): ForgeAgentTask[]

/** Returns the empty-state element shown when no definitions are loaded */
export function createEmptyDefinitionsState(): HTMLElement

/** Returns the empty-state element shown when no agents are running or recent */
export function createEmptyAgentsState(): HTMLElement

/** Creates a definition row element. onToggle fires on Enable/Disable button click. */
export function createDefinitionRow(
  def: AgentDefinition,
  disabled: boolean,
  onToggle: () => void
): HTMLElement

/** Creates an agent row element. onCancel fires on Cancel button click. */
export function createAgentRow(
  agent: ForgeAgentTask,
  onCancel: (id: string) => void
): HTMLElement
```

### View integration

`ForgeAgentMonitorView.renderAll()` calls `createEmptyDefinitionsState()` / `createEmptyAgentsState()` for empty sections. `renderDefinitionRow()` and `renderAgentRow()` delegate to the corresponding helper, passing service method references as callbacks.

---

## forgeMcpStatusViewHelpers.ts

### Exports

```typescript
/** Maps (status, disabled) → CSS class string for the status dot */
export function getServerStatusClass(status: ForgeMcpServerStatus, disabled: boolean): string

/** Returns "disabled" when disabled=true, otherwise "<n> tools" */
export function getToolCountText(toolCount: number, disabled: boolean): string

/** Returns the empty-state element shown when no servers are configured */
export function createEmptyServersState(): HTMLElement

/** Creates a server row element. onToggle fires on Enable/Disable button click. */
export function createServerRow(
  server: ForgeMcpServerStatusEntry,
  onToggle: () => void
): HTMLElement
```

### View integration

`ForgeMcpStatusView.renderServerList()` calls `createEmptyServersState()` for the empty case. `renderServerRow()` delegates to `createServerRow`, passing the service toggle as callback.

---

## Test Coverage

### forgeAgentMonitorView.test.ts — new cases

| Test plan | Scenario | Tested via |
|-----------|----------|------------|
| 5.1 | No definitions → empty state | `createEmptyDefinitionsState()`, assert element present |
| 5.2 | 3 definitions → 3 rows with name + description | call helper 3×, query text content |
| 5.3 | Disabled definition → button text "Enable" | `createDefinitionRow(def, true, ...)` |
| 5.4 | Enabled definition → button text "Disable" | `createDefinitionRow(def, false, ...)` |
| 5.5 | Toggle clicked → callback called | spy on `onToggle`, simulate click |
| 5.6 | No agents → running section empty state | `createEmptyAgentsState()`, assert element present |
| 5.7 | Running agent → blue dot, turn counter, Cancel button | `createAgentRow` with Running task |
| 5.8 | Completed agent → green dot, no Cancel button | `createAgentRow` with Completed task |
| 5.9 | MaxTurnsReached → orange dot | `getAgentStatusClass(MaxTurnsReached)` + row |
| 5.10 | Error agent → red dot | `getAgentStatusClass(Error)` + row |
| 5.11 | Queued agent → yellow dot | `getAgentStatusClass(Queued)` + row |
| 5.12 | Cancel clicked → `onCancel(id)` called | spy on `onCancel`, simulate click |

Plus pure-function unit tests for `getAgentStatusClass` (all 5 enum values) and `sortAgentsByStatus`.

### forgeMcpStatusView.test.ts — new cases

| Test plan | Scenario | Tested via |
|-----------|----------|------------|
| 6.1 | No servers → empty state message | `createEmptyServersState()`, assert element present |
| 6.2 | 2 connected servers → green dot, tool count | `createServerRow` with Connected entries |
| 6.3 | Connecting server → yellow dot | `getServerStatusClass(Connecting, false)` + row |
| 6.4 | Error server → red dot | `getServerStatusClass(Error, false)` + row |
| 6.5 | Disconnected server → gray dot | `getServerStatusClass(Disconnected, false)` + row |
| 6.6 | Disabled server → "disabled" text | `createServerRow` with `disabled: true` |
| 6.7 | Toggle clicked → callback called | spy on `onToggle`, simulate click |

Plus pure-function unit tests for `getServerStatusClass` (all status × disabled combinations) and `getToolCountText`.

### Accepted gaps

- 5.13 (`onDidChangeAgent` → re-render) — requires ViewPane instantiation
- 6.8 (`onDidChangeServerStatus` → re-render) — requires ViewPane instantiation
- 6.9 (`onDidChangeTools` → re-render) — requires ViewPane instantiation

---

## Modified Files

| File | Change |
|------|--------|
| `browser/forgeAgentMonitorView.ts` | `renderDefinitionRow` and `renderAgentRow` delegate to helpers |
| `browser/forgeMcpStatusView.ts` | `renderServerRow` delegates to helper |
| `browser/forgeAgentMonitorViewHelpers.ts` | **new** — extracted helpers |
| `browser/forgeMcpStatusViewHelpers.ts` | **new** — extracted helpers |
| `test/browser/forgeAgentMonitorView.test.ts` | expanded with 12+ new tests |
| `test/browser/forgeMcpStatusView.test.ts` | expanded with 7+ new tests |

---

## Constraints

- Helper files import only from `common/` layers (`forgeAgentTypes.ts`, `forgeMcpTypes.ts`, `forgeConfigResolutionTypes.ts`) — no VS Code platform imports
- No `any` types
- CSS class names used in tests must match what the view currently renders (read view files before writing tests)
- Empty-state rendering for 5.1 and 6.1 may require a small helper function or inline logic in the test — check what the view currently renders for empty state
