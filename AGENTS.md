# AGENT.md

> This file is written for AI agents working on the Forge IDE codebase. It tells you what you need to know before making changes, how to navigate the codebase safely, how to handle common tasks, and what to avoid. Read it fully before making any modifications.

---

## Table of Contents

1. [Before You Start](#1-before-you-start)
2. [Mental Model of the Codebase](#2-mental-model-of-the-codebase)
3. [What Is Safe to Touch](#3-what-is-safe-to-touch)
4. [What Is Load-Bearing — Proceed with Caution](#4-what-is-load-bearing--proceed-with-caution)
5. [What to Never Touch](#5-what-to-never-touch)
6. [Navigating VS Code Internals](#6-navigating-vs-code-internals)
7. [Common Tasks — Step by Step](#7-common-tasks--step-by-step)
8. [Code Patterns and Conventions](#8-code-patterns-and-conventions)
9. [Design System Rules for Agents](#9-design-system-rules-for-agents)
10. [Testing Your Changes](#10-testing-your-changes)
11. [How to Ask for Clarification](#11-how-to-ask-for-clarification)
12. [Failure Modes to Avoid](#12-failure-modes-to-avoid)

---

## 1. Before You Start

### Read these files first

Before making any change to this repository, you should have read:

| File | Why it matters |
|---|---|
| `ARCHITECTURE.md` | Explains what Forge is, how it's structured, and why decisions were made. Prevents you from undoing intentional choices. |
| `DESIGN.md` | Defines every visual rule. Any UI change that violates these rules will be rejected in review. |
| `LATER.md` | Lists ideas that were deliberately deferred. Do not implement anything in this file unless explicitly asked. |
| `product.json` | The identity overrides on top of VS Code. Understand what it controls before touching it. |

### Understand the task scope

Before writing a single line of code, establish:

1. **Which layer does this change live in?** Platform (`src/vs/platform/`), workbench (`src/vs/workbench/`), or extension (`extensions/`)?
2. **Does it touch VS Code internals or only Forge additions?** If it touches VS Code internals, re-read section 4 of this document.
3. **Does it change the AI layer, the canvas system, MCP, or agents?** These have interdependencies documented in `ARCHITECTURE.md` section 6.
4. **Does it have a UI component?** If yes, all visual decisions must follow `DESIGN.md`.

### The fundamental constraint

Forge is a fork of VS Code. The VS Code codebase is large, complex, and has its own strong conventions. When something seems hard to do, the answer is almost always to find the existing VS Code pattern and extend it — not to work around it or replace it. Working against VS Code's architecture creates technical debt that compounds badly when upstream merges happen.

---

## 2. Mental Model of the Codebase

### The layered architecture

VS Code (and therefore Forge) uses a strict layered architecture. Lower layers cannot depend on higher layers.

```
common       ← No runtime dependencies. Platform-agnostic TypeScript.
    ↑
base         ← Basic utilities. No VS Code-specific concepts.
    ↑
platform     ← Core services (files, storage, keybindings, config).
             ← THIS IS WHERE IAIProviderService LIVES.
    ↑
editor       ← The Monaco text editor. No workbench concepts.
    ↑
workbench    ← The full IDE shell. Everything the user sees.
             ← THIS IS WHERE THE CANVAS, CHAT PANES, MCP PANEL LIVE.
```

**The rule:** If you're in `platform/`, you cannot import from `workbench/`. If you're in `common/`, you cannot import from `platform/`. Violating this causes build errors and is always wrong.

When creating a new Forge service, ask: does this need the UI (workbench) or is it a pure data/logic service (platform)? Put it in the right place.

### Dependency injection everywhere

VS Code uses a service locator / DI pattern throughout. Services are never `new`'d directly — they are declared as constructor parameters with a decorator:

```typescript
// ✓ Correct — injected by the DI container
constructor(
  @IAIProviderService private readonly aiService: IAIProviderService,
  @IFileService private readonly fileService: IFileService,
) {}

// ✕ Wrong — never instantiate services directly
constructor() {
  this.aiService = new AIProviderService(); // Never do this
}
```

If you need a service in a new class, add it as a decorated constructor parameter. The container will provide it. If you get a "service not found" error, the service isn't registered yet — check `registerSingleton` calls in the workbench contributions.

### Events not callbacks

VS Code is event-driven. State changes are communicated via `Emitter<T>` / `Event<T>` pairs, not callbacks or promises. When adding state that others need to react to:

```typescript
// Define in the service
private readonly _onProviderChanged = this._register(new Emitter<ProviderChangedEvent>());
readonly onProviderChanged: Event<ProviderChangedEvent> = this._onProviderChanged.event;

// Fire when state changes
this._onProviderChanged.fire({ provider: newProvider });

// Subscribe elsewhere
this._register(aiService.onProviderChanged(e => this.handleProviderChange(e)));
```

Always wrap subscriptions in `this._register(...)` to prevent memory leaks. This is not optional.

### Disposables

Everything that allocates resources (event listeners, timers, child processes, async loops) must be properly disposed. Classes that manage resources should extend `Disposable` and use `this._register()` for all subscriptions and child disposables.

```typescript
class ForgeChatPane extends Disposable {
  constructor(...) {
    super();
    // ✓ Registered — cleaned up when pane is disposed
    this._register(this.aiService.onStreamChunk(chunk => this.handleChunk(chunk)));

    // ✕ Leaked — survives pane disposal, fires into a dead object
    this.aiService.onStreamChunk(chunk => this.handleChunk(chunk));
  }
}
```

Memory leaks from unregistered disposables are one of the most common bugs in VS Code extensions and workbench contributions. Never skip `_register`.

---

## 3. What Is Safe to Touch

These areas are Forge additions with no VS Code internals dependencies. Changes here are unlikely to break upstream merges.

### Freely modifiable

```
src/vs/platform/ai/              ← AI provider interface and implementations (common/ + node/ exist)
src/vs/workbench/services/forge/ ← All Forge services (MCP, layout, agents, plugins) [Phase 2 — does not exist yet]
src/vs/workbench/browser/forge/  ← Forge-specific UI (onboarding, etc.) [Phase 2 — does not exist yet]
src/vs/workbench/browser/parts/editor/forgeChat/ ← Chat pane UI [Phase 2 — does not exist yet]
extensions/forge-theme/          ← Default theme
resources/forge/                 ← Brand assets
product.json                     ← Identity overrides
forge.config.ts                  ← Example workspace config
DESIGN.md                        ← Update when design decisions are made
ARCHITECTURE.md                  ← Update when architecture decisions are made
AGENT.md                         ← Update this file when new patterns emerge
LATER.md                         ← Add deferred ideas here, never implement from here
```

### Modifiable with care

These are VS Code files that Forge has intentionally modified. Changes are fine but you should understand what each controls:

```
product.json                ← Name, icons, data dirs. Understand each field before changing.
src/vs/workbench/browser/workbench.ts ← Workbench bootstrap. Forge registers services here.
src/vs/workbench/workbench.web.main.ts ← Web workbench contributions.
```

---

## 4. What Is Load-Bearing — Proceed with Caution

These areas have Forge modifications that are load-bearing. Changes here can break the quad canvas, provider system, or MCP integration. Read the relevant `ARCHITECTURE.md` section before touching them.

### Editor group system — the quad canvas depends on this

```
src/vs/workbench/services/editor/browser/editorGroupsService.ts
src/vs/workbench/browser/parts/editor/editorGroupView.ts
src/vs/workbench/browser/parts/editor/editorSplitView.ts
```

Forge's `IForgeLayoutService` wraps `IEditorGroupsService`. If you modify how editor groups work, test that:
- `forge.layout.quad` still creates a 2×2 grid correctly
- Pane state (provider, conversation history) survives a layout change
- Session restore correctly reopens panes after restart

### Service registration order — the DI bootstrap

```
src/vs/workbench/workbench.web.main.ts
src/vs/workbench/browser/workbench.ts
```

Forge services are registered here in dependency order. If you add a new service, it must be registered after all the services it depends on. Getting this wrong produces cryptic "service not found" errors at runtime that are hard to diagnose. Refer to `ARCHITECTURE.md` section 6 for the correct order.

### The AI provider interface

```
src/vs/platform/ai/common/aiProvider.ts
```

`IAIProvider` is implemented by every provider. Changing this interface requires updating every provider implementation (`anthropic`, `openai`, `local`) and any code that calls it. Changes should be additive (new optional method) not breaking (changing an existing method signature). If you must make a breaking change, update all implementations in the same commit.

### MCP tool loop

```
src/vs/workbench/services/forge/mcp/mcpService.ts
```

The tool loop (`completeWithTools` → `callTool` → feed result back) is the core agentic loop. Changes here can cause agents to loop infinitely, miss tool results, or send malformed requests to providers. The `MAX_TURNS` constant must remain at 20 unless explicitly asked to change it. Any change to this file requires manual testing of a multi-step agent task.

---

## 5. What to Never Touch

These things must not be changed without an explicit instruction from the project maintainer and a clear understanding of the consequences.

### Never change

```
MAX_TURNS constant in agent.ts          ← Safety limit. See ARCHITECTURE.md decision log.
SecretStorage usage for API keys        ← Security. Keys must never be written to disk.
The telemetry disable in product.json   ← Privacy commitment. Forge has no telemetry.
The ember status bar color              ← Core brand identity. See DESIGN.md.
--color-text-primary (#eae6de)          ← Intentionally warm white. See DESIGN.md section 3.
The font families (Barlow, Fira Code)   ← Locked. See DESIGN.md section 4.
The IAIProvider interface contract      ← Without updating all implementations simultaneously.
upstream-sync branch                    ← Only touched during scheduled monthly merges.
main branch directly                    ← All changes go through PRs from forks. No direct commits.
```

---

## Branching Model

Forge uses a **forking model**. All work — including work by maintainers — goes through a pull request from a personal fork.

### Branch rules

- **`main`** is always releasable. Never push directly to it. Never merge a branch that doesn't build and pass CI.
- **`upstream-sync`** tracks `microsoft/vscode:main`. Only touched during the monthly upstream merge window by maintainers. Do not rebase your feature branch against it — rebase against `main`.

### For any code change

```
fork → branch off main → make changes → PR to forge-ide/forge:main
```

Branch naming: `feature/`, `fix/`, `design/`, `docs/` prefixes, lowercase, hyphen-separated.

The full branching strategy is documented in `ARCHITECTURE.md` section 3.

### Upstream URLs

The upstream organization is `forge-ide`. When referencing Forge in code (URLs, branding, issue links, OAuth client URIs, license URLs, etc.), always use the **upstream** URL `https://github.com/forge-ide/forge` — never a personal fork URL. Personal forks are for development only and must not appear in shipped code.

### Never introduce

- **Raw hex values in component CSS.** Always use design tokens (`var(--color-*)`).
- **New font families.** Three fonts only. This is locked.
- **New grey values outside the iron scale.** If you need a grey, find the closest iron token.
- **Direct instantiation of services.** Always use DI (`@IServiceName` decorator).
- **`console.log` in production code.** Use `ILogService` for logging.
- **Synchronous file I/O.** Always use `IFileService` async methods.
- **`any` type.** TypeScript is strict. If you don't know the type, find it — don't use `any`.
- **Unregistered event listeners.** Always wrap in `this._register(...)`.
- **Animations for decoration.** Animations only communicate state changes. See DESIGN.md section 9.
- **Personal fork URLs in code.** All URLs referencing Forge must use the upstream `forge-ide/forge` organization, not any personal fork.

---

## 6. Navigating VS Code Internals

Working in the VS Code codebase as a first-timer is disorienting. These pointers will save you hours.

### Finding where something lives

**I need to read/write files:**
→ `IFileService` in `src/vs/platform/files/common/files.ts`

**I need to store persistent data:**
→ `IStorageService` in `src/vs/platform/storage/common/storage.ts`
→ Use `StorageScope.WORKSPACE` for per-project data, `StorageScope.PROFILE` for global

**I need to show a notification:**
→ `INotificationService` in `src/vs/platform/notification/common/notification.ts`
→ For toasts specifically: `INotificationService.notify()` with severity

**I need to open a quick pick / input box:**
→ `IQuickInputService` in `src/vs/platform/quickinput/common/quickInput.ts`

**I need to register a keyboard shortcut:**
→ `IKeybindingService` + contribution point in package.json `keybindings` array

**I need to register a command:**
→ `CommandsRegistry.registerCommand()` in `src/vs/platform/commands/common/commands.ts`

**I need to access the active text editor:**
→ `IEditorService.activeTextEditorControl` in `src/vs/workbench/services/editor/common/editorService.ts`

**I need to read a setting:**
→ `IConfigurationService.getValue<T>('forge.settingName')`

**I need to store a secret (API key):**
→ `ISecretStorage.store/get` — never use `IStorageService` for secrets

**I need to run a child process:**
→ `INodeProcessService` or Node's `child_process` module in `node/` layer files only

**I need to show something in the status bar:**
→ `IStatusbarService` in `src/vs/workbench/services/statusbar/browser/statusbar.ts`

### The extension host boundary

VS Code has two JavaScript contexts: the renderer process (where the UI runs) and the extension host process (where extensions run). Forge services run in the renderer. VS Code extensions run in the extension host.

**Key implication:** Forge services cannot directly call VS Code extension APIs and vice versa. If you find yourself trying to use a VS Code extension API (`vscode.workspace.*`, `vscode.window.*`) inside a Forge service, you're in the wrong layer. Forge services use VS Code's internal service APIs, not the public extension API.

### Finding the right service interface vs. implementation

VS Code uses a pattern where the interface lives in `common/` and implementations live in `browser/` or `node/`. When you need to use a service, import the interface from `common/` — never the implementation.

```typescript
// ✓ Import the interface from common/
import { IFileService } from 'vs/platform/files/common/files';

// ✕ Never import the implementation directly
import { FileService } from 'vs/platform/files/browser/fileService';
```

### How to find an existing service

If you need a capability and suspect VS Code already has a service for it:

1. Search `src/vs/platform/` for a directory with a relevant name
2. Look for a file ending in `Service.ts` in that directory
3. Find the `I[Name]Service` interface — this is what you inject
4. Check if it's already used in a nearby file to understand the usage pattern

---

## 7. Common Tasks — Step by Step

### Task: Add a new AI provider

1. Create `src/vs/platform/ai/node/[providerName]Provider.ts`
2. Implement `IAIProvider` — all four methods: `complete`, `stream`, `completeWithTools`, `validateCredentials`
3. Add the provider accent color to `DESIGN.md` section 7 and to `:root` in the CSS token reference as `--color-provider-[name]`
4. Register in `src/vs/platform/ai/common/providerRegistry.ts`:
   ```typescript
   registry.register('[providerName]', ProviderNameProvider);
   ```
5. Add to the `forge.config.ts` schema in `src/vs/workbench/services/forge/config/forgeConfigService.ts`
6. Add to the provider list in the onboarding flow (`src/vs/workbench/browser/forge/onboarding/steps/providerStep.ts`)
7. Write a unit test in `src/vs/platform/ai/test/node/[providerName]Provider.test.ts` that mocks the HTTP layer and tests streaming, errors, and tool use

**Do not** modify any other file. The registry pattern is specifically designed so this is the complete set of changes needed.

---

### Task: Add a new MCP tool capability

MCP tools are exposed by MCP servers — Forge doesn't define them. If you want to support a new tool type, the work is in how Forge handles tool results, not in defining tools.

If you need to add a new **built-in MCP server** (one that ships with Forge):

1. Do not write a new MCP server from scratch — use an existing one from `github.com/modelcontextprotocol/servers`
2. Add it to the default `mcp` array in the forge.config.ts schema with `enabled: false` (opt-in, not opt-out)
3. Add an entry in the MCP panel's "Available" section in `src/vs/workbench/browser/parts/editor/forgeMCPPanel.ts`
4. Add it to the onboarding MCP step

---

### Task: Add a new Forge service

1. Create the interface in `src/vs/platform/forge/common/[serviceName].ts` or `src/vs/workbench/services/forge/[area]/[serviceName].ts` depending on the layer (see section 2)
2. Define the `createDecorator` and export the interface:
   ```typescript
   export const IMyForgeService = createDecorator<IMyForgeService>('myForgeService');
   export interface IMyForgeService {
     readonly _serviceBrand: undefined;
     // ... methods
   }
   ```
3. Create the implementation in the same directory
4. Register in `src/vs/workbench/workbench.web.main.ts` after its dependencies:
   ```typescript
   registerSingleton(IMyForgeService, MyForgeService, InstantiationType.Delayed);
   ```
5. Use `InstantiationType.Delayed` unless the service must start immediately on launch (most services should be delayed)
6. Add to `ARCHITECTURE.md` section 6 with its dependency order

---

### Task: Add a new UI component

1. Check `DESIGN.md` section 6 first — does an existing component cover this use case?
2. If creating new, follow the existing patterns in `src/vs/workbench/browser/parts/editor/forgeChat/`
3. Color: use `var(--color-*)` tokens only. No raw hex.
4. Typography: Barlow Condensed for headings (uppercase), Barlow for body, Fira Code for code/identifiers
5. Spacing: use `var(--sp-*)` tokens only. No raw pixel values.
6. Border radius: `var(--r-sm)` for most components, `var(--r-md)` for panels/toasts, `var(--r-lg)` for cards/modals
7. Active state: `iron-750` background + `ember-400` left border (2px) or bottom border (2px for tabs)
8. If the component has a loading state, use the shimmer skeleton pattern from DESIGN.md, not a spinner (spinners are for async operations in progress, skeletons are for initial content load)

---

### Task: Modify the canvas layout

The canvas layout is controlled by `IForgeLayoutService`. Before modifying:

1. Read `ARCHITECTURE.md` section 8 fully
2. Understand that `IForgeLayoutService` wraps `IEditorGroupsService` — you are extending, not replacing
3. New layout commands should be added as methods on `IForgeLayoutService`, not as direct calls to `IEditorGroupsService` from other code
4. After any layout change, verify: layout switch doesn't lose pane state, session restore works, the tab bar layout controls still work

---

### Task: Add a new agent capability

1. Read `ARCHITECTURE.md` section 10 fully
2. Agents must remain stateless between runs — no persistent memory is added in this phase (it's in LATER.md)
3. New capabilities should be exposed as new MCP tools, not as changes to `ForgeAgent` itself
4. `MAX_TURNS` must remain 20
5. All agent steps must fire events via `this.onStep.fire(...)` — the UI relies on these for the live trace

---

### Task: Update design tokens

If a new token is needed:

1. Add to `DESIGN.md` section 10 token reference table
2. Add to the `:root` CSS block in the same section
3. Add to the VS Code theme file (`extensions/forge-theme/themes/forge-dark.json`) if it needs a VS Code theme variable mapping
4. Do not add tokens that are one-off values for a single component — tokens are for values used in 3+ places

---

## 8. Code Patterns and Conventions

### TypeScript strictness

Forge inherits VS Code's TypeScript configuration which is very strict. The following are enforced:

- `noImplicitAny: true` — never use `any`
- `strictNullChecks: true` — always handle `undefined` and `null`
- `strictFunctionTypes: true` — function type compatibility is strict

When you encounter a type you don't know, find it — don't cast to `any`. Start from the service interface and follow the types.

### Naming conventions

| Thing | Convention | Example |
|---|---|---|
| Service interface | `I[Name]Service` | `IAIProviderService` |
| Service decorator | Same as interface | `IAIProviderService` (both the interface and its decorator token) |
| Forge-specific files | `forge[Name].ts` | `forgeChatInput.ts` |
| Forge-specific classes | `Forge[Name]` | `ForgeChatInput` |
| Events | `on[EventName]` (noun form) | `onProviderChanged`, `onStreamChunk` |
| Emitters | `_on[EventName]` (private) | `_onProviderChanged` |
| Commands | `forge.[area].[action]` | `forge.layout.quad`, `forge.provider.switch` |
| Settings | `forge.[name]` | `forge.streaming`, `forge.maxAgentTurns` |
| CSS tokens | `--color-[category]-[name]` | `--color-surface-2`, `--color-provider-anthropic` |

### Error handling

Never swallow errors silently. Every `try/catch` must either:
- Re-throw the error (let it propagate up)
- Log it via `ILogService` and update UI state to reflect failure
- Handle it meaningfully (e.g., retry logic with backoff)

```typescript
// ✓ Correct — error is surfaced
try {
  await this.mcpService.callTool(name, args);
} catch (error) {
  this.logService.error('MCP tool call failed', error);
  this.onToolCallFailed.fire({ name, error });
}

// ✕ Wrong — error is lost
try {
  await this.mcpService.callTool(name, args);
} catch {
  // silent failure
}
```

### Async patterns

Use `async/await` throughout. Never mix `.then()/.catch()` chains with `async/await` in the same function. Use `Promise.all()` for parallel operations, not sequential awaits.

```typescript
// ✓ Parallel — faster
const [fileContent, gitDiff] = await Promise.all([
  this.fileService.readFile(uri),
  this.gitService.getDiff(),
]);

// ✕ Sequential — unnecessarily slow
const fileContent = await this.fileService.readFile(uri);
const gitDiff = await this.gitService.getDiff();
```

### Logging

Use `ILogService`, not `console.*`. Log levels:

- `logService.trace(...)` — verbose internals, disabled in production
- `logService.debug(...)` — debugging info
- `logService.info(...)` — significant events (provider connected, MCP server started)
- `logService.warn(...)` — recoverable issues (rate limit approaching, reconnecting)
- `logService.error(...)` — failures with an error object

Never log API keys, user message content, or file contents.

---

## 9. Design System Rules for Agents

When writing or modifying any UI code, check every item in this list:

### Colors

- [ ] All colors use `var(--color-*)` tokens — no raw hex values
- [ ] Surface elevation follows the iron scale order (darker = deeper)
- [ ] Active/selected state uses `iron-750` background + `ember-400` indicator
- [ ] Disabled state uses `iron-600` text, no `opacity` reduction
- [ ] Error states use `ember-400` — never a different red
- [ ] Status bar is always `var(--color-brand)` background — never changed
- [ ] Provider accents match: Anthropic = ember, OpenAI = amber, Local = steel

### Typography

- [ ] Headings and labels use Barlow Condensed, uppercase — no exceptions
- [ ] Body text uses Barlow, sentence case
- [ ] Code, paths, shortcuts, identifiers use Fira Code
- [ ] No font family outside these three has been introduced
- [ ] Minimum sizes respected: display 14px, body 12px, mono 9px

### Spacing and layout

- [ ] Spacing values use `var(--sp-*)` tokens — no raw pixel values
- [ ] Border radii use `var(--r-sm/md/lg)` — not hardcoded values
- [ ] No border radius larger than `var(--r-lg)` (8px) has been introduced

### Behaviour

- [ ] Animations communicate state change — they are not decorative
- [ ] Streaming state shows the blinking cursor (5px wide, ember color, 1s blink)
- [ ] Tool calls are visible as inline collapsible cards in the message bubble
- [ ] Error toasts persist until actioned — never auto-dismiss errors

### Writing

- [ ] All UI copy follows the voice guide in `DESIGN.md` section 8
- [ ] Correct terminology is used (provider not model, pane not window, etc.)
- [ ] Error messages state what happened, the cause, and the action — in that order

---

## 10. Testing Your Changes

### What to test manually for each area

**AI provider changes:**
- Connect and send a message → response arrives and streams correctly
- Disconnect network mid-stream → graceful error, pane shows error state
- Invalid API key → clear error toast with actionable message
- Switch providers between panes → each pane uses its own provider independently

**Canvas / layout changes:**
- Switch between all four layout modes (focus, split, quad, code+ai)
- Open a folder, trigger quad layout, close and reopen → panes restore correctly
- Resize a pane → no layout corruption

**MCP changes:**
- Start an MCP server → appears in panel with "connected" state and glow
- Kill the MCP server process externally → auto-reconnect with amber status, then green on reconnect
- Use an MCP tool from a chat pane → tool call appears inline, result feeds back to AI correctly

**Agent changes:**
- Run a multi-step task (e.g., "read registry.ts and write a test for it") → steps appear in thread view, files are actually written
- Hit MAX_TURNS → agent stops and reports gracefully, no silent failure

**UI component changes:**
- Verify at all three sizes: compact (800px wide), standard (1200px), wide (1600px+)
- Verify that hover states, focus states, active states, and disabled states all look correct
- Verify that no design token has been violated (spot-check against DESIGN.md section 10)

### Unit tests

Unit tests live alongside the files they test: `[filename].test.ts` in the same directory. Use the correct runner for each test type:

```bash
# common/ and browser/ tests — Electron renderer
./scripts/test.sh --run src/vs/platform/ai/test/common/providerRegistry.test.ts

# node/ tests — Node.js runtime (required for tests importing npm packages)
npm run test-node -- --run src/vs/platform/ai/test/node/anthropicProvider.test.ts
```

All new public methods on service implementations should have unit tests. Mocking pattern for AI providers:

```typescript
// Mock the HTTP layer, not the service
const mockProvider = new AnthropicProvider(
  { apiKey: 'test-key' },
  new MockFetchService(responses)
);
```

### TypeScript verification

Before submitting any change, verify that TypeScript compiles cleanly:

```bash
npm run compile
```

There should be zero new errors. Never suppress TypeScript errors with `// @ts-ignore` or `// @ts-expect-error` unless the original codebase already used this pattern in the same file.

---

## 11. How to Ask for Clarification

When a task is ambiguous, do not make assumptions that could result in breaking changes. Stop and ask when:

- The task requires modifying a file listed in section 4 (load-bearing) and the scope is unclear
- The task requires a design decision not covered by `DESIGN.md`
- The task conflicts with something in the `ARCHITECTURE.md` decision log
- The task would require adding something that is explicitly in `LATER.md`
- You find yourself about to use `any`, hardcode a color, or bypass the DI system

**How to phrase the clarification:**

Be specific. State what you know, what's ambiguous, and what the two or three plausible interpretations are. Don't ask open-ended questions when you can present concrete options.

```
// ✓ Good clarification request
"I need to add error handling for MCP server crashes. I see two approaches:
(a) Auto-restart with exponential backoff (3 attempts, then show error in panel)
(b) Show error immediately and require manual reconnect
ARCHITECTURE.md says 'auto-restart with backoff' but doesn't specify the attempt count.
Which do you want, and should the reconnect button always be visible or only after
all retries are exhausted?"

// ✕ Bad clarification request
"How should I handle errors?"
```

---

## 12. Failure Modes to Avoid

These are the mistakes most likely to cause problems. They are listed because AI agents have a pattern of making them.

### Making changes too broadly

When asked to fix a bug or add a feature, change only what is necessary. Do not refactor surrounding code, rename variables, or "clean up" unrelated files. Broad changes make diffs hard to review and increase the risk of unintended regressions.

If you notice something that should be cleaned up, add it to a comment or note — do not fix it in the same commit unless explicitly asked.

### Implementing things from LATER.md

`LATER.md` contains ideas that were deliberately deferred. Some of them look like obvious improvements (agent memory, the plugin registry UI, etc.). Do not implement them. They were deferred for reasons — sometimes scope, sometimes because the right architecture isn't clear yet. If you believe something in `LATER.md` should be implemented now, flag it and wait for instruction.

### Breaking the DI contract

Never import a service implementation directly. Never `new` a service. If you find yourself doing this, stop — there is always a way to get the service through the DI container. The error you're trying to avoid by going direct will simply appear later in a harder-to-debug form.

### Silently changing design tokens

Never change the value of an existing design token. Token values are referenced in multiple places and a change to the token changes every usage. If you believe a token value is wrong, flag it — do not change it unilaterally.

### Adding telemetry or logging of user data

Forge has no telemetry. Do not add:
- Analytics calls
- Error reporting that sends data to an external service
- Logging of user message content, file contents, or API keys
- Any call to a URL that is not the configured AI provider or MCP server

This is a trust commitment to users. It is non-negotiable.

### Ignoring the upstream boundary

The files in `upstream-sync` track the original VS Code codebase. Do not commit Forge-specific changes to files that live in the core VS Code directories without namespacing them carefully. The rule of thumb: if the change is in a file that would exist in an unmodified VS Code checkout, think carefully about whether it belongs in a Forge-namespaced file instead.

### Over-engineering for future flexibility

Forge is in beta. Build what is needed now, document what might be needed later in `LATER.md`. Do not create abstract base classes, generic registries, or plugin points for capabilities that don't exist yet. The codebase will be clearer and easier to change when the actual requirements are known.

---

*AGENT.md — Forge IDE v0.1. Update this file when new patterns emerge, when common mistakes are identified, or when the codebase adds new load-bearing systems that agents need to know about. This document should be treated as living documentation — it gets more useful over time, not less.*
