# IMPLEMENTATION_PLAN.md

<!-- **AGENTS CAN EXCLUDE THIS FILE FROM COMMITS** -->

> **Solo developer · Evenings & weekends · ~10–15 hrs/week**
> Track progress by checking off tasks as you complete them. Update the status line at the top of each phase when you start and finish it. Log blockers and decisions in the notes sections as you go — your future self will thank you.

---

## At a Glance

| | |
|---|---|
| **Phases** | 5 |
| **Estimated weeks** | ~28 |
| **Estimated hours** | ~340 |
| **Target** | v0.1.0 public beta |
| **Stack** | TypeScript · Electron · VS Code fork |

### Phase summary

| Phase | Scope | Weeks | Hours | Milestone | Status |
|---|---|---|---|---|---|
| 1 | Foundation — fork, build, orient, rebrand | 1–3 | ~40 | Forge boots | Complete |
| 2 | AI Core — provider abstraction, first chat pane | 4–8 | ~60 | First AI response | Complete |
| 3 | Quad Canvas — multi-pane layout system | 9–14 | ~80 | Quad split works | Not Started |
| 4 | MCP, Agents & Plugins | 15–22 | ~100 | MCP tools work | Not Started |
| 5 | Polish, onboarding & beta release | 23–28 | ~60 | v0.1.0 ships | Not Started |

### Key advantage

You're not starting from scratch. The VS Code fork gives you a production-grade editor, build system, extension host, LSP client, git integration, and terminal for free. Your entire job is to add the AI layer on top.

---

## Phase 1 — Foundation

**Status:** `Completed` · **Weeks 1–3** · **~40 hours**

Get the VS Code codebase building on your machine, understand the architecture, make the first identity changes, and ship a Forge-branded editor that's identical to VS Code except for the name. This is your foundation — don't skip steps here.

**Goal:** Forge boots, shows the Forge mark, has the ember status bar, and opens files correctly.

---

### Week 1 — Environment & Fork

#### Setup

- [ ] **Fork microsoft/vscode on GitHub** (~1 hr)
  Create your fork. Set upstream remote so you can pull future VS Code updates. Name the repo `forge-ide/forge`.
  ```bash
  gh repo fork microsoft/vscode --fork-name forge
  ```

- [ ] **Install prerequisites** (~2 hrs)
  Node 20 LTS, Python 3.x (for native modules), Git, platform build tools (Xcode CLT on Mac, `build-essential` on Linux). VS Code has a full prereq list in their wiki.
  ```bash
  node -v  # must be 20.x
  ```

- [ ] **Run first build** (~4 hrs)
  Follow the VS Code "How to Contribute" guide exactly. First build takes 20–40 min. Expect errors — read them carefully, they're usually a missing native dependency.
  ```bash
  npm install && npm run watch
  ```

- [ ] **Run in development mode** (~1 hr)
  Launch the dev instance. You should see vanilla VS Code open. This is your canvas.
  ```bash
  ./scripts/code.sh  # macOS/Linux
  ```

#### Orientation — read before you touch anything

- [ ] **Read: vscode/wiki/Source-Code-Organization** (~3 hrs)
  Understand the layered architecture: `common → base → platform → editor → workbench`. The workbench is where you'll spend most of your time.

- [ ] **Read: how panels and editors work** (~3 hrs)
  Study `src/vs/workbench/browser/parts/` — specifically the editor part and panel part. These are the areas you'll modify most heavily for the quad canvas.

- [ ] **Read: the service injection system** (~2 hrs)
  VS Code uses a DI container. Every service is registered and consumed via decorators. You'll register your AI services this way. Study `IInstantiationService`.

---

### Weeks 2–3 — Identity & Repo Setup

#### Rebrand (surface-level only for now)

- [ ] **Rename product in package.json and product.json** (~1 hr)
  Change `name`, `applicationName`, `dataFolderName`, and `win32AppUserModelId` from `"code"` to `"forge-ide"`. This controls the app name, data directory, and window title.
  ```
  product.json → "nameShort": "Forge"
  ```

- [ ] **Replace icons and splash assets** (~4 hrs)
  Swap the VS Code icon SVGs in `resources/` with the Forge mark. Update all platform icon sizes (16, 32, 48, 64, 128, 256, 512, 1024px). Script this — there are many.

- [ ] **Apply Forge color theme as default** (~6 hrs)
  Create a built-in theme extension (`extensions/forge-theme/`) that applies the iron/ember palette. Set it as the default in `product.json`. Reuse the CSS variables from the design system.

- [ ] **Replace status bar color** (~1 hr)
  The VS Code blue status bar is in `workbench.colorCustomizations`. Override with `#ff4a12` (ember) in the default theme. This is the most visible brand change.

- [ ] **Bundle Barlow, Barlow Condensed, and Fira Code fonts** (~3 hrs)
  Download `.woff2` files from the respective repos. Add to `src/vs/workbench/browser/media/fonts/`. Declare `@font-face` blocks in `src/vs/workbench/browser/media/style.css`. Set `editor.fontFamily` default to Fira Code in `product.json`.

#### Repo hygiene

- [ ] **Set up your branch strategy** (~1 hr)
  `main` = stable Forge code. `upstream-sync` = tracks `microsoft/vscode` main for merging updates. `feature/*` for each new capability. Never commit AI features to `upstream-sync`.

- [ ] **Set up CI with GitHub Actions** (~3 hrs)
  Copy the existing vscode CI workflow and adapt it. You want: lint, typecheck, and a build check on every PR. Don't worry about the full test suite yet — it's enormous.
  ```
  .github/workflows/forge-ci.yml
  ```

- [ ] **Create ARCHITECTURE.md** (~2 hrs)
  Document your additions so future-you doesn't forget where things live. Write it as you go — one section per phase. Invaluable when you come back after a week away.

---

### Phase 1 Milestone 🔨

> **Forge boots, shows the Forge mark, has the ember status bar, and opens files correctly.**
> VS Code functionality is 100% intact. Only name, icons, colors, and fonts have changed. This is your clean foundation.

**Completed on:** 03/25/2026

**Notes:**

---

## Phase 2 — AI Core

**Status:** `Complete` · **Weeks 4–8** · **~60 hours** · *Requires Phase 1*

Build the provider abstraction package, wire it into VS Code's service system, and create a single working AI chat panel. This is the most architecturally critical phase — get the abstraction right and everything else becomes significantly easier.

**Goal:** Type a message, get a streaming AI response.

---

### Weeks 4–5 — Provider Package

#### @forge/ai-core package

- [x] **Create the AIProvider interface** (~4 hrs)
  Define the core contract: `complete()`, `stream()`, `listModels()`, `validateKey()`. Every provider implements this. Keep it minimal — you can extend later but can't shrink it.
  ```
  src/vs/platform/ai/common/aiProvider.ts
  ```

- [x] **Implement AnthropicProvider** (~6 hrs)
  Use the official `@anthropic-ai/sdk`. Implement streaming via the `stream()` method using Server-Sent Events. This is your reference implementation — get it solid before adding others.
  ```bash
  npm i @anthropic-ai/sdk
  ```

- [x] **Implement OpenAIProvider** (~4 hrs)
  Use the official `openai` package. The API shape is similar to Anthropic's. Any OpenAI-compatible endpoint (Groq, Azure, Together) works by changing the `baseURL`.
  ```bash
  npm i openai
  ```
  > **Bonus:** Also implemented GeminiProvider, LocalProvider (Ollama/LM Studio), and MistralProvider — 5 providers total.

- [x] **Implement ProviderRegistry** (~2 hrs)
  The registry pattern — `Map<string, ProviderCtor>` with `register()` and `resolve()`. Load providers from `forge.config.ts` on startup.

- [x] **Register as a VS Code service** (~4 hrs)
  Create `IAIProviderService` and register it in the workbench service collection. Any part of the workbench can now inject and use AI providers via the DI system.
  ```typescript
  registerSingleton(IAIProviderService, ...)
  ```

---

### Weeks 6–8 — First Chat Panel

#### Chat panel as a VS Code editor

- [x] **Create a custom EditorInput type** (~8 hrs)
  Forge chat panes are `EditorInput` instances — this is how VS Code's tab system works. Create `ForgeAIChatInput` extending `EditorInput`. This gives you tabs, split support, and state management for free.
  ```typescript
  class ForgeAIChatInput extends EditorInput
  ```

- [x] **Build the chat UI component** (~10 hrs)
  VS Code UI uses its own widget system (not React). Study how the terminal or the chat view in GitHub Copilot Chat is built. Build: message list, input area, model selector, send button. Apply Forge design tokens.

- [x] **Wire streaming to the UI** (~6 hrs)
  Use VS Code's event emitter system to stream tokens from the provider into the chat view. Each token appends to the last message. Add the blinking cursor during streaming.

- [x] **Add forge.config.ts loading** (~5 hrs)
  On workspace open, look for `forge.config.ts`. Parse it and initialize the configured providers. Fall back to a settings-based config if no file is found.
  > Implemented as `IForgeConfigService` with file watching for live config reload.

- [x] **Add activity bar AI Workspaces icon** (~3 hrs)
  Register a view container in the activity bar as a minimal stub. The full sidebar (workspace management, context system, quad controls) belongs in Phase 3, but the structural skeleton closes out Phase 2 and gives the product a visible, branded entry point.
  > **Note:** Chat currently opens via `forge.chat.new` command in the editor area. This task creates the activity bar icon + a simple sidebar with provider info and a "New Chat" button. Phase 3 grows it into the full workspace management surface.

  **Implementation steps:**

  1. **Constants file** — Create `src/vs/workbench/contrib/forgeAI/common/forgeAI.ts` with view container ID (`workbench.view.forgeAI`), view ID (`workbench.forgeAI.workspaceView`), and context key. Pattern: `src/vs/workbench/contrib/files/common/files.ts`.
  2. **ViewPaneContainer** — Create `src/vs/workbench/contrib/forgeAI/browser/forgeAIViewlet.ts`. Use `Codicon.flame` as a temporary icon (current hub-and-spoke SVG is multi-color/too detailed for 24px monochrome slot — custom icon deferred to Phase 3 or 5). Place in `ViewContainerLocation.Sidebar`, `order: 10`. Add `Ctrl+Shift+I` keybinding. Use `mergeViewWithContainerWhenSingleView: true` (flip to `false` in Phase 3 when more views are added). Pattern: `src/vs/workbench/contrib/search/browser/search.contribution.ts`.
  3. **Workspace ViewPane** — Create `src/vs/workbench/contrib/forgeAI/browser/forgeAIWorkspaceView.ts`. A `ViewPane` subclass that renders: active provider name/model from `IForgeConfigService`, a "New Chat" button executing `forge.chat.new` via `ICommandService`, and welcome content when no workspace is open. Pattern: `src/vs/workbench/contrib/files/browser/views/emptyView.ts`.
  4. **Styles** — Create `src/vs/workbench/contrib/forgeAI/browser/media/forgeAIViewlet.css` using Forge design tokens (`iron-850` background, `ember-400` button accent, `mono-xs` labels, `body-sm` text).
  5. **Contribution wiring** — Create `src/vs/workbench/contrib/forgeAI/browser/forgeAI.contribution.ts` to register the view container, view descriptor, and welcome content. Pattern: `src/vs/workbench/contrib/chat/browser/chatParticipant.contribution.ts`.
  6. **Workbench import** — Add contribution import to `src/vs/workbench/workbench.common.main.ts`.
  7. **Tests** — Create `src/vs/workbench/test/browser/contrib/forgeAI/forgeAIViewlet.test.ts` testing view container registration, view descriptor registration, and context key binding.

  **Key decisions:**
  - Separate `contrib/forgeAI/` module from `forgeChat/` (editor pane) — follows VS Code's one-contrib-per-activity-bar-entry pattern.
  - Chat-open behavior lives in the ViewPane, not the container — makes Phase 3 extension (add more ViewPanes) straightforward.
  - Phase 3 will: replace welcome content with workspace list UI, add @ context system elements, add conversation history, add quad canvas layout toggles, and optionally replace `Codicon.flame` with a custom monochrome SVG.

---

### Phase 2 Milestone ⬡

> **Open a chat pane, type a message, get a streaming response from Claude or GPT-4o.**
> Model selector works. `forge.config.ts` is loaded. The provider registry resolves correctly. Tokens stream into the UI. This is the moment Forge becomes real.

**Completed on:** 03/29/2026

**Notes:**

- Implemented 5 providers (Anthropic, OpenAI, Gemini, Mistral, Local) instead of the planned 2 — the abstraction held up well.
- Added `vscodeVersion` compatibility layer (unplanned) — Forge's own semver diverges from upstream, so extension management, gallery queries, and extension hosts needed to report the correct VS Code version. Documented in ARCHITECTURE.md.
- Chat pane uses the editor pane model (`EditorInput` → `EditorPane`) rather than a sidebar view container. This was the right call — it gets tab management, split support, and serialization for free from VS Code's infrastructure.
- `IForgeConfigService` supports live file watching and fires `onDidChange` events for reactive config updates.
- CSS design tokens (`--sp-*`, `--r-*`, `--color-text-disabled`) registered in stylelint allowlist but not yet injected at runtime — will need a token injection mechanism before the tokens resolve to real values.

---

## Phase 3 — Quad Canvas

**Status:** `not started` · **Weeks 9–14** · **~80 hours** · *Requires Phase 2*

Extend VS Code's editor group system to support the Forge quad layout — up to 4 independent panes, each with their own AI provider, all sharing the same workspace context. This is the most technically complex phase and the one that differentiates Forge most clearly.

**Goal:** 4 independent panes with 3 different AI providers running simultaneously.

---

### Weeks 9–11 — Layout Engine

#### Understand the editor group system first

- [ ] **Study IEditorGroupsService deeply** (~4 hrs)
  VS Code already supports split editors. Read `src/vs/workbench/services/editor/browser/editorGroupsService.ts`. Your quad layout is built on top of this — don't replace it, extend it.

- [ ] **Create ForgeLayoutService** (~8 hrs)
  A new service that wraps `IEditorGroupsService` and exposes higher-level layout commands: `setLayout('quad')`, `setLayout('split')`, `setLayout('focus')`, `setLayout('code+ai')`. Stores layout in workspace state.
  ```
  src/vs/workbench/services/forge/forgeLayout.ts
  ```

- [ ] **Implement quad split command** (~10 hrs)
  Register `forge.layout.quad` command. It opens 4 editor groups in a 2×2 grid, assigns each a `ForgeAIChatInput`, and sets the active provider per the workspace config. The layout toggle in the tab bar calls this.

- [ ] **Persist pane state across sessions** (~6 hrs)
  When Forge closes, save which panes were open, which provider each was using, and their conversation history. Restore on next open. Use VS Code's `IStorageService` for this.

---

### Weeks 12–14 — Context & Workspace

#### Context system — the @ mention

- [ ] **Build the @ context system** (~10 hrs)
  When the user types `@` in a chat input, show a picker: files, symbols, git diff, selection, another pane's history. Selected items become context chips. Their content is injected into the system prompt.

- [ ] **File context injection** (~5 hrs)
  When a file chip is attached, read its contents via `IFileService` and prepend to the message as a system context block. Respect token limits — truncate large files intelligently from the middle, not the end.

- [ ] **Active editor context** (~4 hrs)
  Automatically include the currently focused code file as context in the nearest AI pane. This is the "always-on" context that makes AI responses relevant without manual attachment.

- [ ] **Cross-pane context** (~5 hrs)
  Let users attach another pane's conversation history as context to a new message. Useful for "take what Claude said in pane 1 and ask GPT-4o to review it in pane 2".

- [ ] **Workspaces — named session collections** (~6 hrs)
  A workspace is a named set of pane configurations. Create, save, and switch between workspaces from the sidebar. Each workspace remembers its layout, providers, and conversation histories.

---

### Phase 3 Milestone ⊞

> **Quad split with 3 different AI providers running simultaneously, file context attached via @, workspaces saving correctly.**
> This is the "Forge moment" — where it stops feeling like VS Code with a chat plugin and starts feeling like a genuinely different product.

**Completed on:** ___________

**Notes:**

---

## Phase 4 — MCP, Agents & Plugins

**Status:** `not started` · **Weeks 15–22** · **~100 hours** · *Requires Phase 3*

Add MCP server management, tool call visibility, sub-agent orchestration, and the plugin system. These are what make Forge more than a multi-model chat UI. Take these one at a time and don't rush the MCP integration.

**Goal:** filesystem MCP tools work in chat, tool calls are visible inline, one sub-agent demo completes a multi-step file task.

---

### Weeks 15–18 — MCP Integration

#### MCP server management

- [ ] **Install and study the MCP TypeScript SDK** (~4 hrs)
  Read the `@modelcontextprotocol/sdk` docs. Understand `Client`, `StdioClientTransport`, and how tools/resources/prompts are listed and called. Write a standalone test script before integrating into Forge.
  ```bash
  npm i @modelcontextprotocol/sdk
  ```

- [ ] **Create IMCPService** (~10 hrs)
  Manages the lifecycle of all MCP server connections: connect, disconnect, list tools, call tools. Each server runs as a child process (stdio transport). Register as a VS Code service.
  ```
  src/vs/workbench/services/forge/mcpService.ts
  ```

- [ ] **Connect MCP tools to AI providers** (~12 hrs)
  When an AI provider is called, inject available MCP tools into the tools array of the API request. Handle `tool_use` responses — call the MCP server, get the result, send it back to the model. This is the core agentic loop.

- [ ] **Build the MCP panel in the activity bar** (~10 hrs)
  A view container showing all configured servers, their connection state, available tools, and a live call log. Reference the component designs in the design system.

- [ ] **Tool call visibility in chat** (~6 hrs)
  When a tool is called inline during a response, show it as a collapsible card in the chat bubble — tool name, arguments, result, duration. This is the transparency layer.

---

### Weeks 19–22 — Agents & Plugins

#### Sub-agent system

- [ ] **Design the agent execution model** (~3 hrs)
  An agent is a self-contained AI loop: system prompt + tools + a task string + a stop condition. It runs in a background worker, reports progress via events, and writes results back to the orchestrating pane. Sketch this on paper before coding.

- [ ] **Implement ForgeAgent class** (~10 hrs)
  The agent loop: call AI → check for `tool_use` → call MCP tool → feed result back → repeat until done or `max_turns` reached. Emit events at each step so the UI can update. Keep it under 200 lines.
  ```
  src/vs/workbench/services/forge/agent.ts
  ```

- [ ] **Sub-agent thread view in chat** (~8 hrs)
  When the orchestrating model spawns an agent, show it as a collapsible banner in the parent chat pane. Expand to see step-by-step progress.

- [ ] **Agent monitor panel** (~8 hrs)
  The bird's-eye task manager — list of all running/completed/queued agents with progress bars, token counts, and step traces. Accessible from the activity bar.

#### Plugin system — foundation only for beta

- [ ] **Define the plugin manifest format** (~4 hrs)
  A `forge-plugin.json` that declares: name, version, what it contributes (`mcp-server`, `agent-behaviour`, `ui-panel`). Keep this simple for beta — you can extend it later. Don't try to build the full registry yet.
  ```
  forge-plugin.json schema v0.1
  ```

- [ ] **Local plugin loading** (~5 hrs)
  Load plugins from `~/.forge/plugins/` at startup. This is enough for beta — users can install plugins manually. The registry UI can come post-beta.

---

### Phase 4 Milestone ⚒

> **filesystem MCP tools work in chat, tool calls are visible inline, one sub-agent demo completes a multi-step file task.**
> The demo: "Refactor src/providers/ to use a registry pattern" spawns two agents, they read and rewrite files via MCP, the diff appears in the diff pane. This is your beta demo.

**Completed on:** ___________

**Notes:**

---

## Phase 5 — Polish, Onboarding & Beta Release

**Status:** `not started` · **Weeks 23–28** · **~60 hours** · *Requires Phase 4*

Harden what you've built, implement the adaptive onboarding flow, set up distribution, and release v0.1.0 beta. Resist the urge to add more features. The goal of beta is to get real users and real feedback, not to be feature-complete.

**Goal:** v0.1.0 beta downloadable by the public.

---

### Weeks 23–25 — Onboarding & Hardening

#### Adaptive onboarding flow

- [ ] **Implement environment detection** (~5 hrs)
  On first launch: detect VS Code config, scan env vars for API keys, check if Ollama/LM Studio is running, check if npx is available for MCP. Store results and use them to shape onboarding.

- [ ] **Build onboarding as a custom editor** (~10 hrs)
  The onboarding screens are a `ForgeOnboardingInput` — a special `EditorInput` that shows on first launch. Each step is a view managed by the same `ForgeLayoutService`. No separate window needed.

- [ ] **VS Code config import** (~5 hrs)
  When VS Code is detected, offer to copy `keybindings.json`, `settings.json` (filtered), and installed extension IDs. Write to `~/.forge/` — never touch the VS Code config.

#### Stability hardening

- [ ] **Error handling for provider failures** (~5 hrs)
  Every AI call needs proper error boundaries: rate limit → toast + suggestion to switch, auth failure → clear prompt to fix key, network failure → retry button. These are the most common first-run failures.

- [ ] **MCP server crash recovery** (~4 hrs)
  MCP servers are child processes — they crash. Implement auto-restart with backoff and a clear error state in the MCP panel. Users should never see a silent failure.

- [ ] **Memory and performance baseline** (~4 hrs)
  Four AI panes + three MCP servers can get memory-hungry. Profile with Chrome DevTools (Electron exposes this). Target: <800MB RAM for a typical quad-pane session.

---

### Weeks 26–28 — Distribution & Launch

#### Build and distribution

- [ ] **Set up packaging** (~8 hrs)
  VS Code uses its own build pipeline (gulp + vscode-build-tools). Study how it packages for each platform. Target macOS first (ARM + Intel), then Linux `.deb`/`.rpm`, then Windows. Don't try all three simultaneously.
  ```bash
  npm run gulp vscode-darwin-arm64
  ```

- [ ] **Code signing — macOS** (~3 hrs)
  Unsigned apps get Gatekeeper warnings. For beta you can skip notarization but you need a Developer ID certificate. Apple Developer account costs $99/year. Budget for this.

- [ ] **GitHub Releases for distribution** (~2 hrs)
  Use GitHub Releases for beta distribution — upload `.dmg`, `.deb`, and `.exe` artifacts. No auto-updater needed yet for beta.

- [ ] **Release checklist** (~3 hrs)
  Before tagging v0.1.0: all five milestones hit, onboarding flow working, README complete with install instructions, `CONTRIBUTING.md` written, `LICENSE` file present (MIT), and at least 5 people have tested it on their machine.

#### Launch

- [ ] **Post the landing page** (~2 hrs)
  Deploy `forge-landing.html` to `forge-ide.dev`. Add the download links.

- [ ] **Submit to Hacker News Show HN** (~1 hr)
  Write a clear one-paragraph summary: what it is, what makes it different, what stage it's at, what feedback you want. Post on a weekday morning US time.

- [ ] **Set up Discord or GitHub Discussions** (~1 hr)
  You need somewhere for early users to ask questions and report issues. GitHub Discussions is zero-maintenance. Discord gives more real-time feedback. Pick one and link it from the README.

---

### Phase 5 Milestone 🚀

> **v0.1.0 is publicly downloadable, installs without errors, and someone you don't know has used it.**
> That last condition is the real milestone. Real users will find bugs you never imagined.

**Completed on:** ___________

**Notes:**

---

## Known Risks

| Risk | Level | Why it matters | Mitigation |
|---|---|---|---|
| VS Code upstream conflicts | 🔴 HIGH | VS Code ships weekly. A big change in the editor group system could break your quad canvas. | Keep changes in namespaced files (`forge*`). Review VS Code changelog weekly. Merge upstream monthly, not daily. |
| VS Code UI system learning curve | 🔴 HIGH | It's not React. The chat UI in Phase 2 will take longer than expected. | Study the GitHub Copilot Chat extension source before building yours. It solves the exact same UI problem. Don't reinvent — adapt. |
| Scope creep | 🟡 MED | At 10–15 hrs/week, every extra feature added to a phase pushes beta back significantly. | Maintain `LATER.md` — every idea that comes up mid-phase gets written there, not implemented. Review it between phases, not during. |
| MCP tool loop complexity | 🟡 MED | The agentic loop has many edge cases: infinite loops, large outputs, conflicting results, partial failures. | Hard-code `max_turns = 20` from day one. Build the tool call log UI before you build agents — you need visibility to debug. |
| Build system complexity | 🟡 MED | VS Code's build system is complex. Packaging for three platforms has many edge cases. | Do macOS first. Linux second. Windows third. Use GitHub Actions for cross-platform builds. |
| Burnout | 🟡 MED | 28 weeks of evenings and weekends on a single project is a marathon. | Ship something small every 2 weeks. Each milestone is a genuine reason to feel good. Take breaks between phases. |
| API key security | 🟢 LOW | A security flaw that leaks keys would be catastrophic for trust. | Use VS Code's `SecretStorage` API (system keychain) from day one. Never store keys in plain text. |

---

## Key References

### VS Code internals

| Resource | URL | When to use |
|---|---|---|
| How to Contribute | `github.com/microsoft/vscode/wiki/How-to-Contribute` | First stop. Build instructions, coding guidelines. |
| Source Code Organization | `github.com/microsoft/vscode/wiki/Source-Code-Organization` | Read before touching anything. |
| GitHub Copilot Chat source | `github.com/microsoft/vscode-copilot-chat` | Reference for Phase 2 chat UI. Study heavily. |
| VS Code Extension API | `code.visualstudio.com/api` | Explains concepts you'll encounter in core. |

### AI & MCP

| Resource | URL | When to use |
|---|---|---|
| MCP TypeScript SDK | `github.com/modelcontextprotocol/typescript-sdk` | Phase 4. Start with client examples. |
| Anthropic SDK | `github.com/anthropic-ai/anthropic-sdk-typescript` | Phase 2. Study the streaming example first. |
| OpenAI Node.js library | `github.com/openai/openai-node` | Phase 2. Second provider after Anthropic. |
| MCP server examples | `github.com/modelcontextprotocol/servers` | Phase 4. Use these, don't write your own for beta. |

### Build & distribution

| Resource | URL | When to use |
|---|---|---|
| VS Code build workflows | `github.com/microsoft/vscode/blob/main/.github/workflows` | Phase 5. Study before setting up yours. |
| Electron packaging | `electronjs.org/docs/latest/tutorial/application-distribution` | Phase 5. Reference when packaging fails. |
| Apple Developer ID | `developer.apple.com/developer-id` | Phase 5. Get account before you need it. |
| Cursor IDE | *(closed source, study via teardown)* | Best existing reference for a VS Code fork. |

---

## Progress Log

Use this section to log notable events, blockers, and decisions as you go. Newest entries at the top.

| Date | Phase | Note |
|---|---|---|
| 03/29/2026 | 2 | Activity bar viewlet (`contrib/forgeAI/`) implemented — Codicon.sparkle icon, sidebar with provider info and New Chat button, Ctrl+Shift+I keybinding. Phase 2 complete. |
| 03/29/2026 | 2 | PR #28: vscodeVersion compat layer, chat editor disposable leak fix, CSS design token compliance, forge-agents manifest |
| 03/29/2026 | 2 | PR #26 merged: AI provider interfaces, registry, 5 provider implementations, AIProviderService, ForgeConfigService, chat editor pane with streaming UI |
| 03/25/2026 | 1 | Phase 1 complete — Forge boots with brand identity |

---

*IMPLEMENTATION_PLAN.md — Forge IDE. Check off tasks as you complete them. Update phase status lines. Log decisions and blockers in the Notes sections. Don't edit the milestone descriptions — they're your definition of done.*
