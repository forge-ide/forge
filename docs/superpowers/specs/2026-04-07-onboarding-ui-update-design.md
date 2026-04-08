# Onboarding UI Update — Design Spec

**Date:** 2026-04-07
**Branch:** feature/onboard-ui-updates
**Reference mock:** `docs/ui-mocks/forge-onboarding.html`

## Goal

Update the Forge onboarding flow from its current 4-step implementation to match the 7-screen UI mock. Two new steps are added (Canvas Explainer, MCP Servers), existing steps receive visual polish, and the progress indicator is upgraded from dots to a percentage bar with step counter.

## Approach

Extend in place (Option A). Keep all existing infrastructure: `ForgeOnboardingEditor`, `ForgeOnboardingInput`, contribution registration, and the service layer. Add 2 new step files, update `ForgeOnboardingView`'s state machine, and polish existing step files to match the mock.

---

## Architecture

### Step State Machine

The view controller gains two new states. The full sequence:

```
'detecting' → 'welcome' → 'canvas' → ['import'] → 'provider' → 'mcp' → 'complete'
```

- `'detecting'` — silent boot scan, no UI, runs in background on open
- `'welcome'` — Step 1, always shown
- `'canvas'` — Step 2, always shown, **cannot be skipped**
- `'import'` — Step 3, **adaptive**: only shown if `env.hasVSCodeConfig === true`
- `'provider'` — Step 4 (or 3 if no VS Code), skippable
- `'mcp'` — Step 5 (or 4 if no VS Code), skippable
- `'complete'` — Ready to Forge summary

### Adaptive Step Count

```ts
const totalSteps = env.hasVSCodeConfig ? 5 : 4;
const progress = (currentStep / totalSteps) * 100; // CSS width %
```

The step counter label (`Step X of Y`) and progress bar both use this.

### File Changes

**Modify:**
- `forgeOnboardingView.ts` — extend state machine, replace dot progress with bar + counter
- `forgeOnboardingView.css` — progress bar styles, visual polish tokens
- `steps/step1Welcome.ts` — add detection badges, update copy to match mock
- `steps/step2Import.ts` — replace checkboxes with icon + toggle rows, update copy
- `steps/step3Provider.ts` — add hex badges, cloud/local tags, updated layout
- `steps/step4Ready.ts` — add launch grid (Open Folder / New AI Session), checkmark summary rows

**Add:**
- `steps/stepCanvasExplainer.ts` — new Canvas Explainer step
- `steps/stepMCP.ts` — new MCP Servers step

---

## Step Designs

*Note: progress percentages below are for the 5-step (VS Code detected) case. Without VS Code, step count is 4 and percentages adjust accordingly.*

### Step 1 — Welcome
- Progress bar: 20%
- Headline: `FORGE IDE`
- Subtitle: "You're running a VSCode fork built around one idea: the AI backing your IDE should be your choice."
- Detection badges (shown for each found item): VS Code config, API key, local server / npx
- Footer: `Skip setup →` (ghost) | `Get started →` (primary)

### Step 2 — Canvas Explainer *(new)*
- Progress bar: 40%
- Headline: `THE CANVAS`
- Subtitle: "Forge doesn't have a code editor with AI bolted on. The canvas is shared — AI chat and code live as equals."
- CSS quad-canvas preview: 2×2 grid showing 4 panes (top-left: green dot + blinking cursor, top-right: steel dot, bottom-left: amber/code dot, bottom-right: steel dot)
- Tag below preview: "Required · Cannot be skipped"
- Footer: `← Back` (ghost) | `Got it →` (primary)
- **No skip button.** Back navigates to the Welcome step.

### Step 3 — VS Code Import *(adaptive)*
- Progress bar: 60%
- Headline: `IMPORT YOUR CONFIG`
- Subtitle: "We found an existing VS Code setup. Choose what to bring across..."
- Five toggle rows with icon + label + metadata + toggle switch:
  1. ⌨ Keybindings — ON by default
  2. ◻ Theme & UI Settings — ON
  3. ⊡ Extensions — ON
  4. ⎇ Git settings — ON
  5. 🤖 Copilot / AI extension config — **OFF** by default
- Footer: `Skip import` (ghost) | `Import selected →` (primary)

### Step 4 — Connect AI Provider
- Progress bar: 80%
- Headline: `CONNECT A PROVIDER`
- Subtitle: "Choose how Forge talks to AI. You can connect multiple providers and switch between them per pane."
- Detection banner if API key found: "Anthropic API key detected"
- Four provider options with hex icon, name, cloud/local badge:
  1. Anthropic (auto-selected if key detected)
  2. OpenAI
  3. Custom endpoint
  4. Local · Ollama / LM Studio (greyed out if no server detected)
- API key input shown for selected provider (pre-filled if detected, masked)
- Inline validation: flag malformed key format, but do not block navigation
- Footer: `Skip for now` (ghost) | `Connect [Provider] →` (primary)

### Step 5 — MCP Servers *(new)*
- Progress bar: 100%
- Headline: `MCP SERVERS`
- Subtitle: "MCP servers let your AI read and write files, search code, query databases, and more."
- Detection banner if npx available
- Four server options with icon, name, badge:
  1. 🗂 Filesystem MCP — `recommended` badge (green), selected by default
  2. 🐙 GitHub MCP — selected by default
  3. 🌐 Browser MCP
  4. 🗄 Postgres MCP
- Note below list: "More MCP servers available in the plugin registry after setup."
- Footer: `Skip` (ghost) | `Enable selected →` (primary)

### Ready to Forge
- Progress bar: 100%
- Header zone: ember-tinted gradient background
- Headline: `READY TO FORGE`
- Subtitle: "Your workspace is configured. Open a folder or start a new AI session to begin."
- Checkmark summary (one row per configured item, shown only if configured):
  - ✓ Canvas layout set to Quad
  - ✓ VS Code config imported · keybindings, theme, extensions
  - ✓ [Provider] connected · [model]
  - ✓ [Selected MCPs] enabled
- 2-column launch grid:
  - 📂 Open Folder — "Start with an existing project"
  - ⬡ New AI Session — "Start with a blank canvas" (ember border, highlighted)
- Footer: `Enter Forge →` (primary, slightly larger)

---

## Service Layer

### MCP Preferences Storage

Add two methods to `IForgeOnboardingService`:

```ts
saveMCPSelections(servers: string[]): Promise<void>;
getMCPSelections(): Promise<string[]>;
```

- Storage key: `forge.onboarding.mcpSelections`
- Value: JSON-serialised array of server IDs: `'filesystem' | 'github' | 'browser' | 'postgres'`
- Backed by `IStorageService` (same as existing `markComplete`)
- The MCP service reads this key on startup to determine which servers to configure

### No Other Service Changes

Environment detection, API key secret storage, and VS Code config import all use existing service methods unchanged.

---

## Launch Actions (Ready Screen)

Both actions call `markComplete()` and close the onboarding editor before firing:

- **Open Folder** → `ICommandService.executeCommand('workbench.action.files.openFolder')`
- **New AI Session** → `ICommandService.executeCommand('forge.workspace.create')`
- **Enter Forge →** button → closes onboarding without a launch action (user picks later)

If a launch command fails, log via `ILogService` and close onboarding — do not leave the user stuck.

---

## Error Handling

- Detection failures → treat as "not found", do not block onboarding
- API key format invalid → show inline error on provider step, do not block navigation
- MCP save failure → log via `ILogService`, do not block completion
- Launch command failure → log, close onboarding gracefully

---

## Testing

New and updated test files (alongside source, run via `./scripts/test.sh`):

| File | Coverage |
|---|---|
| `forgeOnboardingView.test.ts` | Step navigation, adaptive count (4 vs 5), progress bar %, cannot skip canvas |
| `stepCanvasExplainer.test.ts` | Renders correctly, back/next fire, no skip button present |
| `stepMCP.test.ts` | Selection state, save calls service, skip bypasses save |
| `forgeOnboardingService.test.ts` | Extend: `saveMCPSelections` / `getMCPSelections` round-trip |
