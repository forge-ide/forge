# ARCHITECTURE.md

> **For humans and agents:** This document describes what Forge IDE is trying to accomplish, how it is structured technically, where things live, and how to make decisions that stay consistent with the architecture. Read this before modifying any platform-level code, registering a new service, or adding a new AI capability.

---

## Table of Contents

1. [What Forge Is](#1-what-forge-is)
2. [What Forge Is Not](#2-what-forge-is-not)
3. [Relationship to VS Code](#3-relationship-to-vs-code)
4. [High-Level Architecture](#4-high-level-architecture)
5. [Directory Structure](#5-directory-structure)
6. [Core Services](#6-core-services)
7. [The AI Layer](#7-the-ai-layer)
8. [The Canvas System](#8-the-canvas-system)
9. [MCP Integration](#9-mcp-integration)
10. [Agent System](#10-agent-system)
11. [Plugin System](#11-plugin-system)
12. [Configuration](#12-configuration)
13. [Data & State](#13-data--state)
14. [Build & Distribution](#14-build--distribution)
15. [Decision Log](#15-decision-log)

---

## 1. What Forge Is

Forge IDE is an AI-agnostic development environment. It is a fork of VS Code that replaces the assumption of a single AI backend (as in GitHub Copilot) with a first-class, configurable, multi-provider AI system built into the core of the editor.

### The core product thesis

Most AI-augmented editors are built as: **editor + AI sidebar**. The AI is a guest in the editor's house.

Forge is built as: **AI canvas + code canvas**, peers sharing the same space. Neither is primary. The user decides how to split the canvas between them.

### Primary user

A developer who uses AI heavily in their workflow, wants control over which model they use for which task, wants to run multiple models simultaneously and compare outputs, and does not want to be locked into a single provider's ecosystem or pricing.

### Goals for v0.1.0 beta

1. A working quad-split canvas where each pane can connect to an independent AI provider
2. A provider abstraction that makes switching between Anthropic, OpenAI, and local models trivial
3. MCP server management with tool call visibility
4. A basic sub-agent system for multi-step file tasks
5. An adaptive onboarding flow that detects what's already on the machine
6. Packaging for macOS (ARM + Intel) and Linux

### Non-goals for v0.1.0

- Windows packaging (post-beta)
- A plugin registry UI (plugins load from local path for beta)
- Agent memory / persistent context between sessions
- Cloud sync of workspaces
- A billing or account system (Forge is free and open source)

---

## 2. What Forge Is Not

Understanding what Forge explicitly rejects is as important as understanding what it is.

**Forge is not a VS Code extension.** Forge modifies the VS Code core. Extensions run in the extension host and cannot access the workbench internals needed for the quad canvas and AI service layer. If you find yourself thinking "we could build this as an extension," the answer is almost always no — extensions don't have the access we need.

**Forge is not vendor-aligned.** There is no preferred AI provider. Anthropic does not get a better experience than OpenAI. OpenAI does not get a better experience than a local Ollama instance. The UI never suggests one provider over another. Design and code must reflect this neutrality.

**Forge is not a chat interface with an IDE attached.** The code editing experience must be at least as good as vanilla VS Code. Forge inherits this for free by forking, but it must be preserved — never degrade the editing experience to make AI features easier to build.

**Forge is not opinionated about how you use AI.** It does not have a "recommended workflow." It provides the primitives (panes, providers, MCP, agents) and lets the user compose them.

---

## 3. Relationship to VS Code

### Fork strategy

Forge is a hard fork of `microsoft/vscode`. It is not a downstream extension, a soft fork that tracks every commit, or a wrapper. It is its own product that started from VS Code's codebase.

### Upstream sync

An `upstream-sync` branch tracks `microsoft/vscode:main`. It is merged into `main` monthly (not continuously) after reviewing the VS Code changelog for breaking changes. The goal is to receive bug fixes and platform improvements without being blocked by them.

**Merge frequency:** Monthly. Before each merge, review the VS Code changelog at github.com/microsoft/vscode/releases for changes to:

- `IEditorGroupsService` (quad canvas dependency)
- `IInstantiationService` (service registration)
- `workbench/browser/parts/` (layout system)
- Any editor input or editor group APIs

If a VS Code update breaks a Forge system, fix Forge — do not revert the upstream merge.

### Branching strategy

Forge uses a **forking model** for contributions. Nobody commits to `forge-ide/forge` directly except through a pull request from a personal fork.

#### Branches in the main repo

| Branch | Purpose | Who touches it |
| --- | --- | --- |
| `main` | Stable, releasable code. Protected — no direct pushes. | Merged via PR only |
| `upstream-sync` | Tracks `microsoft/vscode:main`. Reset monthly. | Maintainers only, during sync window |

`main` must always build and pass CI. No work-in-progress is merged here.

#### Contributor workflow (forking model)

```text
1. Fork forge-ide/forge to your personal GitHub account
2. Clone your fork locally
3. Create a branch off main:  git checkout -b fix/mcp-reconnect
4. Make changes, commit, push to your fork
5. Open a pull request from your fork's branch → forge-ide/forge:main
6. Address review feedback with additional commits (no force-push during review)
7. Maintainer squash-merges or merge-commits to main
```

#### Branch naming

| Prefix | When to use | Example |
| --- | --- | --- |
| `feature/` | New capability | `feature/gemini-provider` |
| `fix/` | Bug fix | `fix/mcp-reconnect-backoff` |
| `design/` | Visual / CSS changes only | `design/ember-token-update` |
| `docs/` | Documentation only | `docs/branching-strategy` |
| `upstream/` | Upstream merge prep | `upstream/2026-03` |

Keep branch names lowercase, hyphen-separated, and specific enough to understand from the PR list.

#### The upstream-sync process

The `upstream-sync` branch is managed by maintainers only and is not a development branch:

```text
1. At the start of each month, fetch microsoft/vscode:main
2. Merge (or rebase) into upstream-sync
3. Review the VS Code changelog for breaking changes to Forge-touched areas
4. Update "vscodeVersion" in product.json to match the new upstream VS Code version
5. Run the full build and CI suite on upstream-sync
6. Open a PR from upstream-sync → main with a summary of upstream changes
7. Merge after CI passes and at least one maintainer reviews
```

**Why step 4 matters:** Forge keeps its own semver in `package.json` (`0.x.y`), separate from the upstream VS Code version. Built-in extensions check `engines.vscode` against the product version — if these don't match, extensions fail to activate. The `vscodeVersion` field in `product.json` is used primarily for extension compatibility checks, marketplace communication, and extension host initialization, while `package.json` version remains Forge's own release version.

Never merge `upstream-sync` into your feature branch — rebase against `main` instead.

### What we keep from VS Code (unmodified)

- The Monaco editor core
- The language server protocol client
- The extension host and extension API
- The terminal emulator
- Git integration (SCM providers)
- The file system abstraction (`IFileService`)
- The settings system
- The keybinding system
- The debug adapter protocol

### What we modify

- The workbench layout (quad canvas, Forge-branded activity bar)
- The status bar (always ember background)
- The default theme (Forge dark theme as default)
- Product identity (name, icons, data directories)
- The editor group system (extended for Forge layout commands)
- The activity bar (Forge-specific view containers)

### What we add

These directories are the target state for v0.1.0. Most do not exist yet — they are created as part of Phase 2 implementation.

- The AI layer common interfaces (`src/vs/platform/ai/common/`) — **exists**
- The AI layer node implementations (`src/vs/platform/ai/node/`) — **exists** (Anthropic, OpenAI, Gemini, Mistral, Local)
- The AI layer browser implementation (`src/vs/platform/ai/browser/`) — **exists**
- The MCP integration (`src/vs/workbench/services/forge/mcp/`) — not yet
- The agent system (`src/vs/workbench/services/forge/agent/`) — not yet
- The Forge layout service (`src/vs/workbench/services/forge/common/` + `src/vs/workbench/services/forge/browser/`) — **exists**
- The Forge workspace service (`src/vs/workbench/services/forge/common/` + `src/vs/workbench/services/forge/browser/`) — **exists**
- The Forge AI activity bar viewlet (`src/vs/workbench/contrib/forgeAI/`) — **exists** (sidebar entry point, `Codicon.sparkle` temporary icon)
- The Forge chat editor input (`src/vs/workbench/browser/parts/editor/forgeChat/`) — not yet
- The onboarding flow (`src/vs/workbench/browser/forge/onboarding/`) — not yet
- The plugin loader (`src/vs/workbench/services/forge/plugins/`) — not yet

---

## 4. High-Level Architecture

```text
┌─────────────────────────────────────────────────────────────────┐
│                         Electron Shell                          │
│  ┌───────────────────────────────────────────────────────────┐  │
│  │                       Main Process                        │  │
│  │  VS Code main + ForgeAppService + product.json overrides  │  │
│  └───────────────────────┬───────────────────────────────────┘  │
│                          │ IPC                                   │
│  ┌───────────────────────▼───────────────────────────────────┐  │
│  │                    Renderer Process                        │  │
│  │  ┌─────────────────────────────────────────────────────┐  │  │
│  │  │                   VS Code Workbench                  │  │  │
│  │  │  ┌──────────┐ ┌──────────┐ ┌─────────────────────┐  │  │  │
│  │  │  │ Activity │ │ Sidebar  │ │    Main Canvas      │  │  │  │
│  │  │  │   Bar    │ │  Panel   │ │  ┌──────┬──────┐    │  │  │  │
│  │  │  │          │ │          │ │  │ TL   │ TR   │    │  │  │  │
│  │  │  │ Forge AI │ │ Explorer │ │  │ Pane │ Pane │    │  │  │  │
│  │  │  │ Workspaces│ │ MCP Srv │ │  ├──────┼──────┤    │  │  │  │
│  │  │  │ MCP Panel│ │ Providers│ │  │ BL   │ BR   │    │  │  │  │
│  │  │  │ Agents   │ │          │ │  │ Pane │ Pane │    │  │  │  │
│  │  │  └──────────┘ └──────────┘ │  └──────┴──────┘    │  │  │  │
│  │  │                            └─────────────────────┘  │  │  │
│  │  │  ┌───────────────────────────────────────────────┐  │  │  │
│  │  │  │  Status Bar (always ember)                    │  │  │  │
│  │  │  └───────────────────────────────────────────────┘  │  │  │
│  │  └─────────────────────────────────────────────────────┘  │  │
│  │                                                             │  │
│  │  Forge Services (registered via VS Code DI)                │  │
│  │  ┌──────────────┐ ┌──────────────┐ ┌──────────────────┐   │  │
│  │  │ IAIProvider  │ │ IMCPService  │ │ IForgeLayout     │   │  │
│  │  │ Service      │ │              │ │ Service          │   │  │
│  │  └──────────────┘ └──────────────┘ └──────────────────┘   │  │
│  │  ┌──────────────┐ ┌──────────────┐                         │  │
│  │  │ IForgeAgent  │ │ IForgePlugin │                         │  │
│  │  │ Service      │ │ Service      │                         │  │
│  │  └──────────────┘ └──────────────┘                         │  │
│  └───────────────────────────────────────────────────────────┘  │
│                                                                   │
│  ┌────────────────────────────┐  ┌───────────────────────────┐   │
│  │      Extension Host        │  │      Node.js Workers      │   │
│  │  (standard VS Code extns)  │  │  (MCP child processes,    │   │
│  │                            │  │   agent execution loops)  │   │
│  └────────────────────────────┘  └───────────────────────────┘   │
└─────────────────────────────────────────────────────────────────┘

External
┌─────────────────┐  ┌───────────────────┐  ┌──────────────────┐
│  AI Providers   │  │   MCP Servers     │  │  Local Models    │
│  Anthropic API  │  │  filesystem npx   │  │  Ollama :11434   │
│  OpenAI API     │  │  github npx       │  │  LM Studio       │
│  Gemini API     │  │  postgres npx     │  │                  │
│  Mistral API    │  │                   │  │                  │
└─────────────────┘  └───────────────────┘  └──────────────────┘
```

### Process model

Forge inherits VS Code's multi-process Electron architecture:

- **Main process:** App lifecycle, window management, OS integration
- **Renderer process:** The workbench UI — where all Forge services run
- **Extension host:** Isolated Node.js process for VS Code extensions — Forge does not run its AI services here
- **Node.js workers:** MCP server child processes and long-running agent loops run here to avoid blocking the renderer

---

## 5. Directory Structure

Only Forge-specific directories are documented here. The rest of the structure follows `microsoft/vscode` conventions — see their wiki for that.

The `src/vs/` subtree below reflects the target state for v0.1.0. The common interfaces under `src/vs/platform/ai/common/` exist; all other Forge-specific directories are created as part of Phase 2 implementation.

```text
forge/
├── src/
│   └── vs/
│       ├── platform/
│       │   └── ai/                          ← AI provider abstraction (platform layer)
│       │       ├── common/
│       │       │   ├── aiProvider.ts        ← IAIProvider interface
│       │       │   ├── aiProviderService.ts ← IAIProviderService interface
│       │       │   └── providerRegistry.ts  ← ProviderRegistry implementation
│       │       ├── browser/
│       │       │   └── aiProviderService.ts ← Browser implementation
│       │       └── node/
│       │           ├── anthropicProvider.ts ← Anthropic implementation
│       │           ├── openaiProvider.ts    ← OpenAI implementation
│       │           ├── geminiProvider.ts    ← Google Gemini implementation
│       │           ├── mistralProvider.ts   ← Mistral implementation
│       │           └── localProvider.ts     ← Local/Ollama implementation
│       └── workbench/
│           ├── contrib/
│           │   └── forgeAI/                 ← Activity bar viewlet (sidebar entry point)
│           │       ├── common/
│           │       │   └── forgeAI.ts       ← View container/view IDs, context keys
│           │       └── browser/
│           │           ├── forgeAI.contribution.ts  ← Registration wiring
│           │           ├── forgeAIViewlet.ts         ← ViewPaneContainer
│           │           ├── forgeAIWorkspaceView.ts   ← ViewPane (provider info, New Chat)
│           │           └── media/
│           │               ├── forgeAIViewlet.css
│           │               └── forgeLayoutButtons.css ← Layout preset button styles
│           ├── browser/
│           │   ├── parts/
│           │   │   └── editor/
│           │   │       └── forgeChat/       ← Chat editor input and UI
│           │   │           ├── forgeChatInput.ts
│           │   │           ├── forgeChatEditor.ts
│           │   │           └── forgeChatView.ts
│           │   └── forge/
│           │       └── onboarding/          ← First-run onboarding flow
│           │           ├── forgeOnboarding.ts
│           │           └── steps/
│           └── services/
│               └── forge/                   ← All Forge-specific services
│                   ├── common/
│                   │   ├── forgeConfigService.ts      ← IForgeConfigService interface + impl
│                   │   ├── forgeConfigTypes.ts        ← ForgeConfig, ForgeProviderConfig, ForgeModelConfig, resolveModelConfig()
│                   │   ├── forgeCredentialService.ts  ← IForgeCredentialService interface
│                   │   ├── forgeLayoutService.ts      ← IForgeLayoutService interface
│                   │   ├── forgeContextTypes.ts       ← Context types, priority enum, token budget
│                   │   ├── forgeContextService.ts     ← IForgeContextService interface
│                   │   ├── forgeGitDiffService.ts     ← IForgeGitDiffService interface
│                   │   ├── forgeWorkspaceService.ts   ← IForgeWorkspaceService interface
│                   │   └── forgeWorkspaceTypes.ts     ← ForgeWorkspaceConfig, SerializedConversation
│                   ├── browser/
│                   │   ├── forgeLayoutService.ts      ← ForgeLayoutService browser impl
│                   │   ├── forgeContextService.ts     ← ForgeContextService browser impl
│                   │   ├── forgeWorkspaceService.ts   ← ForgeWorkspaceService browser impl
│                   │   ├── contextProviders/           ← Context provider implementations (browser layer)
│                   │   │   ├── forgeFileContextProvider.ts          ← File context resolution with middle truncation
│                   │   │   ├── forgeActiveEditorContextProvider.ts  ← Auto-attach active editor (workbench contribution)
│                   │   │   └── forgePaneHistoryContextProvider.ts   ← Cross-pane conversation history context
│                   │   └── media/
│                   │       └── forgeContext.css        ← Context chip styles
│                   ├── electron-browser/
│                   │   ├── forgeCredentialService.ts   ← ForgeCredentialService impl (SecretStorage + process.env)
│                   │   └── forgeProviderBootstrap.ts   ← Workbench contribution: config → credentials → provider registration
│                   ├── node/
│                   │   └── contextProviders/           ← Context providers requiring Node.js
│                   │       └── forgeGitDiffContextProvider.ts  ← Git diff via child_process.execFile
│                   ├── test/
│                   │   ├── common/
│                   │   │   └── forgeConfigService.test.ts
│                   │   └── browser/
│                   │       └── forgeLayoutService.test.ts
│                   ├── mcp/
│                   │   ├── mcpService.ts          ← IMCPService implementation
│                   │   ├── mcpServer.ts           ← Single server connection
│                   │   └── mcpToolCallLog.ts      ← Call log state
│                   ├── agent/
│                   │   ├── forgeAgent.ts          ← Agent execution loop
│                   │   ├── forgeAgentService.ts   ← IForgeAgentService
│                   │   └── forgeAgentMonitor.ts   ← Monitor state
│                   └── plugins/
│                       └── forgePluginService.ts  ← Plugin loader
│
├── extensions/
│   └── forge-theme/                         ← Built-in Forge themes
│       ├── package.json
│       └── themes/
│           ├── forge-dark.json
│           └── forge-light.json
│
├── product.json                             ← Identity overrides (name, dirs)
├── docs/
│   ├── ARCHITECTURE.md                      ← This file
│   ├── DESIGN.md                            ← Design system reference
│   └── CONTRIBUTING.md                      ← Contributor guide
├── LATER.md                                 ← Deferred platform items (pre-launch blockers)
└── .github/
    └── workflows/
        ├── forge-ci.yml                     ← Compile, lint, hygiene, layer checks, unit tests
        └── forge-release.yml                ← Release packaging
```

---

## 6. Core Services

Forge registers all its services into VS Code's dependency injection container using the standard `registerSingleton` pattern. Services are defined by an interface in `common/` and implemented in `browser/` or `node/` depending on what they need.

### Service interfaces

All Forge service interfaces are prefixed with `IForge` or `IAI` and live in `common/` directories so they can be referenced from any layer.

```typescript
// Pattern: define interface in common/
export const IAIProviderService = createDecorator<IAIProviderService>('aiProviderService');
export interface IAIProviderService {
  readonly _serviceBrand: undefined;
  readonly onDidChangeProviders: Event<string[]>; // fires registered provider names when the set changes

  registerProvider(name: string, provider: IAIProvider): void;
  unregisterProvider(name: string): void;
  getProvider(name: string): IAIProvider | undefined;
  has(name: string): boolean;
  listProviders(): string[];

  getDefaultProviderName(): string | undefined;   // from config; used by new panes for initial selection
  setDefaultProviderName(name: string): void;
}

// Pattern: implement in browser/ or node/
class AIProviderService implements IAIProviderService { ... }

// Pattern: register in workbench contributions
registerSingleton(IAIProviderService, AIProviderService, InstantiationType.Delayed);
```

> **Note:** There is no global "active provider" concept. Each chat pane tracks its own provider independently. `getDefaultProviderName()` returns the config default used to initialize new panes.

### Service dependency order

Services initialize in this order. Do not create circular dependencies.

```text
IForgeConfigService          ← reads forge.json, no dependencies. InstantiationType: Eager.
  ↓
IForgeCredentialService      ← resolves API keys (SecretStorage → env var → undefined). electron-browser layer. InstantiationType: Delayed.
  ↓
IAIProviderService           ← provider registry. No active provider — panes choose their own. InstantiationType: Delayed.
  ↓
ForgeProviderBootstrap       ← workbench contribution (not a service). WorkbenchPhase.AfterRestored.
                                Reads config, resolves credentials, sets default provider name.
                                Re-bootstraps on config change and credential change.
  ↓
IMCPService                  ← reads config, launches MCP child processes
  ↓
IForgeLayoutService          ← depends on IEditorGroupsService, IStorageService, IForgeConfigService, ILogService
  ↓
IForgeContextService         ← depends on IFileService, IEditorService, IEditorGroupsService, IQuickInputService, IForgeLayoutService, ILogService, IInstantiationService, IForgeGitDiffService, IWorkspaceContextService
  ↓
IForgeWorkspaceService       ← depends on IStorageService, IForgeLayoutService, ILogService. InstantiationType: Delayed. Registered via self-registering module.
  ↓
IForgeGitDiffService         ← depends on ILogService. Node-only, registered in workbench.desktop.main.ts. InstantiationType: Delayed.
  ↓
ForgeActiveEditorContextProvider ← workbench contribution (not a service). Registered via registerWorkbenchContribution2, WorkbenchPhase.AfterRestored. Depends on IEditorService, IForgeLayoutService, IForgeContextService, IFileService, IConfigurationService, ILogService.
  ↓
IForgeAgentService           ← depends on IAIProviderService + IMCPService
  ↓
IForgePluginService          ← depends on all of the above
```

### Credential resolution chain

`IForgeCredentialService` resolves API keys for each provider using a two-level fallback:

```text
SecretStorage (`forge.provider.${providerName}`)
       ↓ not found
Environment variable (determined by provider's envKey, e.g., ANTHROPIC_API_KEY)
       ↓ not found
undefined  →  provider skipped during bootstrap
```

The `envKey` is resolved per-provider: explicit `envKey` field on the provider config, then `PROVIDER_ENV_VARS` lookup by provider name, then `${PROVIDER_NAME}_API_KEY` as a last-resort convention. The credential service fires `onDidChangeCredential` when a SecretStorage key changes, triggering re-bootstrap.

---

## 7. The AI Layer

### IAIProvider interface

Every AI provider implements this interface. No provider-specific code should ever appear outside its own file.

```typescript
export interface IAIProvider {
  readonly name: string;
  readonly availableModels: string[];

  // Single-shot completion
  complete(request: AICompletionRequest): Promise<AICompletionResponse>;

  // Streaming completion — yields tokens as they arrive
  stream(request: AICompletionRequest): AsyncIterable<AIStreamChunk>;

  // Validate that the configured credentials work
  validateCredentials(): Promise<AIValidationResult>;
}
```

> **Note:** `completeWithTools()` is planned for MCP integration but does not exist in the `IAIProvider` interface yet. It will be added when the MCP tool loop is implemented.

### Adding a new provider

1. Create `src/vs/platform/ai/node/[providerName]Provider.ts` — implement `IAIProvider`
2. Add the provider's default env var to `PROVIDER_ENV_VARS` in `forgeConfigTypes.ts`
3. Add to the provider list in the onboarding flow
4. Add to `DESIGN.md` with its accent color

Providers are bootstrapped from `forge.json` config at startup. `ForgeProviderBootstrap` reads the config, resolves credentials via `IForgeCredentialService`, and registers providers. Users declare providers in their `forge.json` `providers` array — no code changes needed beyond the implementation and env var mapping.

### Streaming architecture

All streaming responses use `AsyncIterable<AIStreamChunk>`. The chat view consumes this via `for await` and appends tokens to the message buffer. Token delivery to the UI goes through VS Code's event emitter system to keep it off the main thread where possible.

```typescript
// In the provider:
async *stream(request): AsyncIterable<AIStreamChunk> {
  for await (const chunk of sdkStream) {
    yield { type: 'token', content: chunk.delta.text };
  }
  yield { type: 'done' };
}

// In the chat view:
for await (const chunk of provider.stream(request)) {
  if (chunk.type === 'token') {
    this.appendToken(chunk.content);
  } else if (chunk.type === 'done') {
    this.finalizeMessage();
  }
}
```

### API key security

All API keys are stored in VS Code's `SecretStorage` API, which uses the system keychain on all platforms (Keychain on macOS, Secret Service on Linux, Credential Manager on Windows). Keys are never written to disk in plaintext. They are never logged. They are never included in workspace state or settings sync.

```typescript
// Store
await this.secretStorage.store(`forge.api.key.${providerName}`, apiKey);

// Retrieve
const key = await this.secretStorage.get(`forge.api.key.${providerName}`);
```

---

## 8. The Canvas System

### ForgeLayoutService

The `IForgeLayoutService` wraps VS Code's `IEditorGroupsService` and provides higher-level layout commands. It is the single source of truth for canvas layout state.

```typescript
export interface IForgeLayoutService {
  // Apply a named layout preset
  setLayout(layout: ForgeLayout): Promise<void>;

  // Current layout
  readonly activeLayout: ForgeLayout;

  // Open a new AI chat pane in a specific group position
  openChatPane(position: PanePosition, providerId?: string): Promise<void>;

  // Save the current layout to workspace state
  saveLayout(): void;

  // Restore the last saved layout
  restoreLayout(): Promise<void>;
}

type ForgeLayout = 'focus' | 'split' | 'quad' | 'code+ai';
type PanePosition = 'tl' | 'tr' | 'bl' | 'br';
```

### Layout commands

Layout presets are exposed as commands with keybindings:

| Command | Keybinding | Description |
| --- | --- | --- |
| `forge.layout.focus` | `Ctrl+Shift+1` | Single pane, maximized |
| `forge.layout.split` | `Ctrl+Shift+2` | Side-by-side split |
| `forge.layout.codeai` | `Ctrl+Shift+3` | Code editor left, AI pane right |
| `forge.layout.quad` | `Ctrl+Shift+4` | 2×2 grid — four independent panes |

Commands are registered in `forgeAI.contribution.ts` and delegate to `IForgeLayoutService.setLayout()`. Layout state is persisted to `IStorageService` (workspace scope) and restored on workspace reopen.

### ForgeChatInput

The `ForgeChatInput` is a custom `EditorInput` subclass. This is the correct VS Code pattern for custom editor types — it gives us tab management, split support, and serialization for session restore for free.

```typescript
export class ForgeChatInput extends EditorInput {
  static readonly ID = 'workbench.input.forgeChat';

  constructor(
    public readonly workspaceId: string,
    public readonly providerId: string,
    @IAIProviderService private readonly aiProviderService: IAIProviderService,
    @IMCPService private readonly mcpService: IMCPService,
  ) { super(); }

  override get typeId(): string { return ForgeChatInput.ID; }
  override getName(): string { return `AI: ${this.providerId}`; }
}
```

### Context system

The context system is managed by `IForgeContextService` (registered in `workbench.common.main.ts`, `InstantiationType.Delayed`). It gathers workspace context for AI requests and exposes it through the `@` chip UI in the chat input.

**Trigger:** Typing `@` in the chat input opens a quick pick (`IQuickInputService`) showing available context sources. Selected sources are added as `ForgeContextChip` objects that are serialized into the system prompt before the AI request.

Context types and their injection behavior:

| Type | How injected |
| --- | --- |
| ActiveEditor | Content of the focused editor tab in a `<file>` XML block |
| Selection | Selected text in a `<selection>` block |
| File | Full file content in a `<file>` XML block in the system prompt |
| GitDiff | Output of `git diff HEAD` in a `<diff>` block |
| Symbol | Symbol definition from LSP in a `<symbol>` block |
| PaneHistory | Previous pane's messages as `<context>` conversation history |

**Priority ordering:** Context is injected in priority order (highest first):

```text
ActiveEditor > Selection > File > GitDiff > Symbol > PaneHistory
```

**Token budget:** Default budget is 8000 tokens per request. Token count is estimated as `Math.ceil(content.length / 4)`. Context items are added in priority order until the budget is exhausted. Items that exceed the remaining budget are dropped and a warning chip is shown in the input.

### Context providers

Context providers implement the resolution logic for each context type. They are registered with `IForgeContextService` and resolve context items into content strings for injection into AI requests.

| Provider | Layer | Description |
| --- | --- | --- |
| `ForgeFileContextProvider` | browser | Reads files via `IFileService`. Applies middle truncation at 32,000 chars by default — keeps the first and last portions of large files with a `[...truncated...]` marker. |
| `IForgeGitDiffService` | node | Runs `git diff HEAD` via `child_process.execFile` with a 5-second timeout. Node-layer only because it requires child process spawning. Registered in `workbench.desktop.main.ts`. |
| `ForgeActiveEditorContextProvider` | browser | Workbench contribution that auto-attaches the active editor's content as context when `forge.autoAttachActiveEditor` is `true`. Listens for editor focus changes and routes context to the nearest AI pane via `IForgeLayoutService`. |
| `ForgePaneHistoryContextProvider` | browser | Exposes other panes' conversation history as context. Resolution is lazy — history is only serialized when the context item is resolved, not when it is attached. |

**Configuration:** `forge.autoAttachActiveEditor` (boolean, default: `false`) — when enabled, the active editor's content is automatically attached as context to the nearest AI pane on editor focus change.

### Workspace system

`IForgeWorkspaceService` manages named session collections. A workspace captures the current canvas layout, pane states, and provider assignments so users can save and restore different working configurations (e.g., "debugging with Claude" vs. "code review with GPT-4").

**Interface:**

```typescript
export interface IForgeWorkspaceService {
  readonly onDidChangeActiveWorkspace: Event<ForgeWorkspaceConfig | undefined>;
  readonly onDidChangeWorkspaces: Event<void>;
  getWorkspaces(): ForgeWorkspaceConfig[];
  getActiveWorkspace(): ForgeWorkspaceConfig | undefined;
  createWorkspace(name: string): Promise<ForgeWorkspaceConfig>;
  saveActiveWorkspace(): Promise<void>;
  switchWorkspace(id: string): Promise<void>;
  deleteWorkspace(id: string): Promise<void>;
  renameWorkspace(id: string, newName: string): Promise<void>;
}
```

**Data model:**

A `ForgeWorkspaceConfig` captures a snapshot of the canvas state:

```typescript
interface ForgeWorkspaceConfig {
  readonly id: string;           // UUID
  readonly name: string;         // User-chosen label
  readonly createdAt: number;    // Epoch ms
  readonly layout: ForgeLayout;  // 'focus' | 'split' | 'quad' | 'code+ai'
  readonly panes: ForgePaneState[];
  readonly conversations: SerializedConversation[];
}
```

**Storage strategy:**

| Data | Scope | Rationale |
| --- | --- | --- |
| Workspace list | `StorageScope.PROFILE` | Shared across VS Code workspaces so the same named sessions are available everywhere |
| Active workspace ID | `StorageScope.WORKSPACE` | Per VS Code workspace — different projects can have different active sessions |

**Switching workspaces:** `switchWorkspace(id)` restores the saved layout via `IForgeLayoutService.setLayout()` and reopens panes with their saved provider assignments. Conversation history is stored in the config but replay is not yet implemented (beta limitation).

**Known limitation:** `StorageScope.PROFILE` does not notify other windows when data changes. Cross-window sync of workspace lists is deferred to post-beta. Each window sees the workspace list as it was when that window loaded.

**Workspace commands:**

| Command | Description |
| --- | --- |
| `forge.workspace.create` | Create a new workspace from the current canvas state |
| `forge.workspace.save` | Save the current layout and pane state to the active workspace |
| `forge.workspace.switch` | Switch to a different workspace (restores its layout) |
| `forge.workspace.delete` | Delete a workspace |

---

## 9. MCP Integration

### Architecture

MCP servers run as child processes managed by `IMCPService`. Each server communicates via stdio using the `@modelcontextprotocol/sdk`'s `StdioClientTransport`. The service manages the lifecycle of all connections and exposes a unified tool registry to the AI layer.

```text
IMCPService
├── connects to servers on startup (from forge.config.ts)
├── exposes listTools() → MCPTool[] (union of all server tools)
├── exposes callTool(serverName, toolName, args) → MCPToolResult
├── maintains live call log (in-memory, last 500 calls)
└── handles crashes with exponential backoff reconnect
```

### Tool injection

When `IAIProvider.completeWithTools()` is called, the MCP tool registry provides the tool definitions. When the model returns a `tool_use` block, `IMCPService.callTool()` is invoked, the result is returned to the model, and the loop continues. This is the core agentic loop.

```typescript
// Simplified tool loop
async function toolLoop(provider, mcpService, request) {
  let response = await provider.completeWithTools(request, mcpService.listTools());

  while (response.stopReason === 'tool_use') {
    const toolResults = await Promise.all(
      response.toolCalls.map(tc =>
        mcpService.callTool(tc.server, tc.name, tc.args)
      )
    );
    request = appendToolResults(request, toolResults);
    response = await provider.completeWithTools(request, mcpService.listTools());
  }

  return response;
}
```

### MCP server configuration

Servers are configured in `forge.config.ts` and loaded at startup. Each server entry specifies the transport (stdio or http) and launch command.

```typescript
// forge.config.ts
mcp: [
  {
    name: 'filesystem',
    transport: 'stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-filesystem', '~/'],
  },
  {
    name: 'github',
    transport: 'stdio',
    command: 'npx',
    args: ['-y', '@modelcontextprotocol/server-github'],
    env: { GITHUB_TOKEN: process.env.GITHUB_TOKEN },
  },
]
```

---

## 10. Agent System

### What is an agent

A `ForgeAgent` is a self-contained AI execution loop. It has:

- A system prompt defining its role and constraints
- A task string (what it needs to accomplish)
- Access to a subset of MCP tools
- A provider (can be different from the orchestrating pane's provider)
- A `max_turns` limit (hard-coded to 20, not user-configurable)
- An event emitter for real-time progress reporting

Agents are stateless — they do not have memory between separate invocations. Within a single invocation, the full conversation history is maintained in memory.

### Execution model

```typescript
class ForgeAgent extends Disposable {
  constructor(
    private readonly config: ForgeAgentConfig,
    @IAIProviderService private readonly aiService: IAIProviderService,
    @IMCPService private readonly mcpService: IMCPService,
  ) { super(); }

  readonly onStep = this._register(new Emitter<AgentStep>());
  readonly onComplete = this._register(new Emitter<AgentResult>());
  readonly onError = this._register(new Emitter<Error>());

  async run(): Promise<AgentResult> {
    let turns = 0;
    const history: Message[] = [{ role: 'user', content: this.config.task }];

    while (turns < MAX_TURNS) {
      this.onStep.fire({ type: 'thinking', turn: turns });

      const response = await this.provider.completeWithTools(
        { systemPrompt: this.config.systemPrompt, messages: history },
        this.config.tools.map(t => this.mcpService.getTool(t))
      );

      if (response.stopReason === 'end_turn') {
        this.onComplete.fire({ result: response.content, turns });
        return { result: response.content, turns };
      }

      for (const toolCall of response.toolCalls) {
        this.onStep.fire({ type: 'tool_call', toolCall });
        const result = await this.mcpService.callTool(
          toolCall.server, toolCall.name, toolCall.args
        );
        this.onStep.fire({ type: 'tool_result', toolCall, result });
        history.push({ role: 'tool', content: result });
      }

      turns++;
    }

    throw new Error(`Agent exceeded max_turns (${MAX_TURNS})`);
  }
}
```

### Agent spawning

Agents are spawned by the orchestrating model via a special `forge_spawn_agent` tool that is always available in tool-enabled panes. The tool call includes the agent name, task, model, and tool list. `IForgeAgentService` handles the spawn, creates a `ForgeAgent`, runs it, and reports back.

This means the orchestrating model does not need special prompting to use agents — it uses them the same way it uses any other tool.

---

## 11. Plugin System

### Beta scope

For v0.1.0, plugins are loaded from `~/.forge/plugins/` at startup. There is no marketplace, no remote registry, and no install command. Users add plugin directories manually.

### Plugin manifest

Each plugin directory must contain a `forge-plugin.json`:

```json
{
  "name": "my-plugin",
  "version": "0.1.0",
  "displayName": "My Plugin",
  "description": "What this plugin does",
  "contributes": {
    "mcpServers": [
      {
        "name": "my-mcp-server",
        "transport": "stdio",
        "command": "node",
        "args": ["./server.js"]
      }
    ],
    "agentBehaviours": [],
    "uiPanels": []
  }
}
```

### Post-beta

The plugin registry UI, `forge install` CLI command, and remote registry are deferred to post-beta. See `LATER.md`.

---

## 12. Configuration

### forge.json

The primary configuration surface. Loaded from `forge.json` in the workspace root on open. Falls back to global config at `<userRoamingDataHome>/forge/forge.json` if no workspace config is found.

The config is a tree: top-level defaults, a `providers` array, and per-provider `models` arrays. Types are defined in `forgeConfigTypes.ts`.

```typescript
// forge.json — full type definition
interface ForgeConfig {
  defaultProvider: string;       // required — provider name for new panes
  defaultModel?: string;         // optional — overrides first model in provider's list
  stream?: boolean;              // default: true

  providers: ForgeProviderConfig[];  // required — at least one
}

interface ForgeProviderConfig {
  name: string;                  // required — provider ID (e.g., "anthropic")
  baseURL?: string;              // custom endpoint
  envKey?: string;               // override env var name for API key
  models: ForgeModelConfig[];    // required — at least one
}

interface ForgeModelConfig {
  id: string;                    // required — model ID (e.g., "claude-sonnet-4-6")
  maxTokens?: number;
  contextBudget?: number;
}
```

#### Setting resolution

`resolveModelConfig(config, providerName?, modelId?)` merges the config tree into a flat `ResolvedModelConfig` for runtime use. Resolution order: model-level override, then hardcoded defaults (maxTokens: 4096, contextBudget: 8000, stream: true).

#### Credential resolution per provider

The `envKey` for API key lookup is resolved as: explicit `envKey` on the provider config, then `PROVIDER_ENV_VARS[providerName]`, then `${PROVIDER_NAME}_API_KEY`. See section 6 for the full credential resolution chain.

### Settings

Standard VS Code settings are used for user preferences that don't belong in the config file (UI density, font size, etc.). Forge-specific settings are namespaced under `forge.*`.

```json
{
  "forge.showToolCallLog": true,
  "forge.defaultLayout": "quad",
  "forge.streaming": true,
  "forge.maxAgentTurns": 20
}
```

---

## 13. Data & State

### What is persisted

| Data | Storage | Location |
| --- | --- | --- |
| API keys | `ISecretStorage` (system keychain) | OS keychain |
| Workspace layout | `IStorageService` workspace scope | `.forge/workspace.json` |
| Conversation history | `IStorageService` workspace scope | `.forge/conversations/` |
| Named workspaces | `IStorageService` profile scope (`forge.workspaces`) | VS Code profile storage |
| Active workspace ID | `IStorageService` workspace scope (`forge.activeWorkspaceId`) | VS Code workspace storage |
| MCP server config | `forge.config.ts` | Workspace root |
| User preferences | VS Code settings | Standard VS Code locations |
| Plugin manifests | Filesystem | `~/.forge/plugins/` |

### What is never persisted

- AI responses (only conversation history, not embeddings or caches)
- Tool call arguments or results (call log is in-memory only)
- Agent intermediate state (agents are stateless between runs)
- Anything sent to external telemetry (Forge has no telemetry)

### Telemetry

Forge has no telemetry. VS Code's telemetry is disabled in `product.json`. No data is sent to any external service by Forge itself. AI providers receive only what the user explicitly sends (their messages and context). This is documented in the README and enforced in code review.

---

## 14. Build & Distribution

### Development build

```bash
# Install dependencies
npm install

# Start the TypeScript compiler in watch mode
npm run watch

# Launch Forge in a development window
./scripts/code.sh          # macOS/Linux
.\scripts\code.bat         # Windows
```

### Production build (minified, from source)

VS Code's gulp-based build system is used directly. Forge adds its own product configuration on top. The build must run in order — each step depends on the previous one.

```bash
# 1. Install dependencies (if not already done)
npm ci
cd build && npm ci && cd ..

# 2. Compile TypeScript with name mangling (production)
npm run compile-build

# 3. Compile extensions for production
npm run compile-extensions-build

# 4. Download built-in extensions
npm run download-builtin-extensions

# 5. Run core CI validation (hygiene, layer checks)
npm run gulp core-ci

# 6. Assemble the minified application for your platform
npm run gulp vscode-linux-x64-min-ci     # Linux x64
# npm run gulp vscode-linux-arm64-min-ci  # Linux arm64
```

Output: `../Forge-linux-x64/` (a self-contained application directory).

### Packaging (Linux RPM)

Requires `rpmbuild` installed on the system (`dnf install rpm-build` on Fedora).

```bash
# 6. Prepare RPM directory structure and spec file
npm run gulp vscode-linux-x64-prepare-rpm

# 7. Build the RPM
npm run gulp vscode-linux-x64-build-rpm
```

Output: `.build/linux/rpm/x86_64/forge-<version>-<timestamp>.el8.x86_64.rpm`

### Packaging (Linux DEB)

Requires `dpkg` and `fakeroot` installed on the system.

```bash
npm run gulp vscode-linux-x64-prepare-deb
npm run gulp vscode-linux-x64-build-deb
```

Output: `.build/linux/deb/amd64/deb/forge-<version>-<timestamp>_amd64.deb`

### Container build (local Linux, Podman)

`build/container/build.sh` wraps the full production build inside a Podman container so the host doesn't need system packages, npm dependencies, or a matching glibc version. Useful for local testing of release artifacts without polluting the host.

**Prerequisites:** Podman installed and rootless Podman configured (no other dependencies needed on the host).

```bash
# x64 tarball (default)
./build/container/build.sh

# arm64 tarball (emulated via QEMU — 3–5× slower than native)
./build/container/build.sh --arch arm64

# All Linux package formats
./build/container/build.sh --formats tarball,deb,rpm

# Build the container image only (no build run)
./build/container/build.sh --image-only

# Force a clean image rebuild, then produce a deb
./build/container/build.sh --no-cache --formats deb --output ./dist
```

Artifacts land in `./dist/` by default. The container image (`forge-build-x64` / `forge-build-arm64`) is cached by Podman — subsequent runs reuse npm install layers as long as package manifests haven't changed. First build takes ~30 min; incremental builds (source change only) are much faster.

**How it works:** Source is COPYed into the container (not bind-mounted) for hermetic, cacheable builds. `BUILD_SOURCEVERSION` is passed as an env var so the VS Code build system doesn't need a `.git` directory inside the container. No sysroots are used for cross-arch — `--platform linux/arm64` triggers QEMU emulation natively.

See `build/container/README.md` for the full reference and `build/container/Containerfile` for the image definition.

### Timing expectations

| Step | Approximate time |
| --- | --- |
| compile-build (with mangling) | ~17 min |
| compile-extensions-build | ~15 sec |
| download-builtin-extensions | ~30 sec |
| core-ci | ~25 sec |
| assemble min-ci | ~15 sec |
| RPM prepare + build | ~4 min |

### GitHub Actions CI

Every PR runs (via `forge-ci.yml`):

1. `npm ci` — dependency install
2. `npm run compile` — TypeScript typecheck
3. `npm run eslint` — ESLint
4. `npm run stylelint` — CSS/style lint
5. `npm run hygiene` — copyright headers and whitespace checks
6. `npm run valid-layers-check` — layer dependency validation
7. Unit tests (`npm run test-node`)

Releases run the full package pipeline for all platforms via matrix strategy (`forge-release.yml`).

### Platform notes

- **macOS:** Requires Developer ID certificate for distribution. ARM and Intel builds are separate — no universal binary for beta.
- **Linux:** `.deb` for Debian/Ubuntu, `.rpm` for Fedora/RHEL. Both are self-contained.
- **Windows:** Post-beta. Will require a code signing certificate.

---

## 15. Decision Log

A record of significant architectural decisions, why they were made, and what alternatives were considered. Add to this whenever a non-obvious decision is made.

---

### 2026-01 — Fork VS Code core rather than build an extension

**Decision:** Forge modifies the VS Code core rather than building on the extension API.

**Reason:** The extension API does not allow modification of the editor group layout, registration of custom editor input types as first-class tabs, or deep integration into the workbench service layer needed for the quad canvas. Extensions also run in a separate process (the extension host) which makes low-latency UI updates difficult.

**Alternative considered:** Building as a VS Code extension similar to GitHub Copilot. Rejected because the quad canvas — the defining Forge feature — is not achievable within extension API constraints.

---

### 2026-01 — Use the registry pattern for AI providers

**Decision:** All AI providers are registered via a `ProviderRegistry` (`Map<string, ProviderCtor>`) rather than a factory or conditional switch.

**Reason:** The registry pattern allows new providers to be added without modifying any core Forge code. A new provider file registers itself on import. This is critical for the plugin system where third-party providers need to register the same way as built-in ones.

**Alternative considered:** A factory function with a switch statement. Rejected because every new provider would require modifying `providerFactory.ts`, creating a merge conflict surface and a violation of the open/closed principle.

---

### 2026-01 — Chat panes as EditorInput, not a sidebar panel

**Decision:** Forge AI chat panes are implemented as `EditorInput` subclasses rather than as sidebar panel views or custom webview panels.

**Reason:** `EditorInput` gives us VS Code's tab management, split editor support, serialization for session restore, and the same keyboard navigation as code editors — all for free. Sidebar panels are constrained in size and cannot participate in the main canvas grid. Webview panels have a separate process and significant performance overhead.

**Alternative considered:** A custom sidebar panel (like GitHub Copilot). Rejected because it enforces the sidebar/secondary status that Forge explicitly rejects as a product philosophy. Chat and code must be peers in the same space.

---

### 2026-01 — MCP servers as child processes, not in-process

**Decision:** Each MCP server runs as a child process managed by `IMCPService`, communicating via stdio.

**Reason:** MCP servers can crash, hang, consume arbitrary memory, or make blocking I/O calls. Running them in-process would make those failures the Forge process's problem. Child processes provide isolation, can be killed and restarted without affecting the editor, and match how the MCP spec expects servers to be deployed.

**Alternative considered:** Running MCP servers in-process using the SDK's in-memory transport. Rejected due to isolation concerns.

---

### 2026-01 — max_turns hardcoded to 20, not user-configurable

**Decision:** The agent execution loop has a hard-coded `MAX_TURNS = 20` limit that is not exposed as a user setting.

**Reason:** Unbounded agent loops are a significant safety concern — a misconfigured agent could make thousands of file writes or API calls. 20 turns is sufficient for nearly all real coding tasks (refactoring a module, writing tests, updating docs). Making it configurable in beta introduces a support burden before we have evidence of what the right limit is.

**Revisit when:** Post-beta, based on real usage data. The setting exists internally (`forge.maxAgentTurns`) but is hidden from the settings UI.

---

*ARCHITECTURE.md — Forge IDE v0.1. Update the Decision Log whenever a significant architectural choice is made. Update section 5 whenever a new file or directory is added to the Forge-specific areas of the codebase.*
