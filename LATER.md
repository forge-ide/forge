# LATER.md

<!-- **AGENTS CAN EXCLUDE THIS FILE FROM COMMITS** -->

> Deferred items from Phase 4 (MCP agents). Each entry has a location, what was left incomplete, and what needs to happen to resolve it.

---

## Phase 4 Follow-ups

### Wire forgeChatAgentBanner into forgeChatView.ts

**Files:**
- `src/vs/workbench/contrib/forgeAI/browser/forgeChatAgentBanner.ts` — exports `renderAgentBanner`, `updateAgentBanner`, `appendAgentStep`
- `src/vs/workbench/contrib/forgeAI/browser/forgeChatView.ts` — spawns sub-agents via `IForgeAgentService` but does not render the banner

`forgeChatAgentBanner.ts` is fully implemented but not integrated. When the chat view spawns sub-agents, there is no live visual feedback about agent progress.

**Action:** In `forgeChatView.ts`, subscribe to `IForgeAgentService.onDidChangeAgent` and `onDidAgentStep`, then call `renderAgentBanner` / `updateAgentBanner` / `appendAgentStep` accordingly to render a live agent progress banner in the chat view.

---

### Implement allowedTools filtering in ForgeAgentService

**File:** `src/vs/workbench/services/forge/browser/forgeAgentService.ts`

`ForgeAgentTask.allowedTools` is populated from agent definitions but never applied in `runAgentLoop`. All tools from `forgeMcpService.listTools()` are passed to the agent regardless of what `allowedTools` specifies.

**Action:** In `runAgentLoop`, when `allowedTools` is set on the task, filter the tools list returned by `forgeMcpService.listTools()` to only include tools whose names appear in `allowedTools` before passing the list to the model.

---

*Last updated: 2026-04-02 — added Phase 4 follow-ups (banner integration, allowedTools filtering).*
