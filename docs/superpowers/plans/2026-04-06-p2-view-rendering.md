# P2 View Rendering Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract render logic from `ForgeAgentMonitorView` and `ForgeMcpStatusView` into standalone helper functions so they can be unit-tested without the VS Code DI container.

**Architecture:** Two companion helper files (`forgeAgentMonitorViewHelpers.ts`, `forgeMcpStatusViewHelpers.ts`) export pure functions and DOM-building functions that take data + callbacks instead of service references. The view classes delegate to these helpers. Tests import helpers directly — no ViewPane, no DI.

**Tech Stack:** TypeScript, DOM (document.createElement), Mocha (VS Code browser test runner)

---

## File Map

| File | Action | Purpose |
|------|--------|---------|
| `src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorViewHelpers.ts` | **Create** | Pure + DOM helpers for agent monitor view |
| `src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusViewHelpers.ts` | **Create** | Pure + DOM helpers for MCP status view |
| `src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorView.ts` | **Modify** | Delegate to helpers |
| `src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusView.ts` | **Modify** | Delegate to helpers |
| `src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts` | **Modify** | Add 14 new tests |
| `src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts` | **Modify** | Add 11 new tests |

---

## Task 1: Agent monitor — pure functions + empty-state helpers

**Files:**
- Create: `src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorViewHelpers.ts`
- Modify: `src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts`

- [ ] **Step 1: Add failing tests to the existing test file**

Replace the entire content of `src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts` with:

```typescript
/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FORGE_AGENT_MONITOR_VIEW_ID } from '../../browser/forgeAgentMonitorView.js';
import {
	getAgentStatusClass,
	sortAgentsByStatus,
	createEmptyDefinitionsState,
	createEmptyAgentsState,
	createDefinitionRow,
	createAgentRow,
} from '../../browser/forgeAgentMonitorViewHelpers.js';
import { ForgeAgentStatus, ForgeAgentTask } from '../../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../../services/forge/common/forgeConfigResolutionTypes.js';

function makeTask(status: ForgeAgentStatus, overrides: Partial<ForgeAgentTask> = {}): ForgeAgentTask {
	return {
		id: 'test-id',
		name: 'test-agent',
		systemPrompt: '',
		taskDescription: 'do something',
		providerName: 'anthropic',
		model: 'claude-3-5-sonnet',
		maxTurns: 10,
		status,
		currentTurn: 3,
		steps: [],
		...overrides,
	};
}

function makeDef(overrides: Partial<AgentDefinition> = {}): AgentDefinition {
	return {
		name: 'my-agent',
		description: 'does stuff',
		systemPrompt: 'you are helpful',
		...overrides,
	};
}

suite('ForgeAgentMonitorView', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('has correct view ID', () => {
		assert.strictEqual(FORGE_AGENT_MONITOR_VIEW_ID, 'workbench.forgeAI.agentMonitorView');
	});

	// --- getAgentStatusClass ---

	test('getAgentStatusClass Running → "running"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Running), 'running');
	});

	test('getAgentStatusClass Queued → "queued"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Queued), 'queued');
	});

	test('getAgentStatusClass Completed → "completed"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Completed), 'completed');
	});

	test('getAgentStatusClass Error → "error"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.Error), 'error');
	});

	test('getAgentStatusClass MaxTurnsReached → "max_turns_reached"', () => {
		assert.strictEqual(getAgentStatusClass(ForgeAgentStatus.MaxTurnsReached), 'max_turns_reached');
	});

	// --- sortAgentsByStatus ---

	test('sortAgentsByStatus orders Running first, Error last', () => {
		const tasks = [
			makeTask(ForgeAgentStatus.Error),
			makeTask(ForgeAgentStatus.Queued),
			makeTask(ForgeAgentStatus.Running),
			makeTask(ForgeAgentStatus.Completed),
			makeTask(ForgeAgentStatus.MaxTurnsReached),
		];
		const sorted = sortAgentsByStatus(tasks);
		assert.strictEqual(sorted[0].status, ForgeAgentStatus.Running);
		assert.strictEqual(sorted[1].status, ForgeAgentStatus.Queued);
		assert.strictEqual(sorted[2].status, ForgeAgentStatus.Completed);
		assert.strictEqual(sorted[3].status, ForgeAgentStatus.MaxTurnsReached);
		assert.strictEqual(sorted[4].status, ForgeAgentStatus.Error);
	});

	test('sortAgentsByStatus does not mutate the input array', () => {
		const tasks = [makeTask(ForgeAgentStatus.Error), makeTask(ForgeAgentStatus.Running)];
		const original = [...tasks];
		sortAgentsByStatus(tasks);
		assert.strictEqual(tasks[0].status, original[0].status);
	});

	// --- createEmptyDefinitionsState ---

	test('createEmptyDefinitionsState returns element with correct class', () => {
		const el = createEmptyDefinitionsState();
		assert.strictEqual(el.className, 'forge-agent-empty');
	});

	test('createEmptyDefinitionsState text mentions definitions', () => {
		const el = createEmptyDefinitionsState();
		assert.ok(el.textContent!.length > 0);
	});

	// --- createEmptyAgentsState ---

	test('createEmptyAgentsState returns element with correct class', () => {
		const el = createEmptyAgentsState();
		assert.strictEqual(el.className, 'forge-agent-empty');
	});

	test('createEmptyAgentsState text mentions agents', () => {
		const el = createEmptyAgentsState();
		assert.ok(el.textContent!.length > 0);
	});

	// --- createDefinitionRow ---

	test('createDefinitionRow renders name and description', () => {
		const row = createDefinitionRow(makeDef(), false, () => { });
		assert.strictEqual(row.querySelector('.forge-agent-def-name')?.textContent, 'my-agent');
		assert.strictEqual(row.querySelector('.forge-agent-def-desc')?.textContent, 'does stuff');
	});

	test('createDefinitionRow disabled=false → button text is "Disable"', () => {
		const row = createDefinitionRow(makeDef(), false, () => { });
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-agent-def-toggle')!.textContent, 'Disable');
	});

	test('createDefinitionRow disabled=true → button text is "Enable"', () => {
		const row = createDefinitionRow(makeDef(), true, () => { });
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-agent-def-toggle')!.textContent, 'Enable');
	});

	test('createDefinitionRow toggle click fires onToggle callback', () => {
		let called = false;
		const row = createDefinitionRow(makeDef(), false, () => { called = true; });
		row.querySelector<HTMLButtonElement>('.forge-agent-def-toggle')!.click();
		assert.strictEqual(called, true);
	});

	// --- createAgentRow ---

	test('createAgentRow shows name and turn counter', () => {
		const task = makeTask(ForgeAgentStatus.Running, { currentTurn: 3, maxTurns: 10 });
		const row = createAgentRow(task, () => { });
		assert.strictEqual(row.querySelector('.forge-agent-row-name')?.textContent, 'test-agent');
		assert.strictEqual(row.querySelector('.forge-agent-row-turns')?.textContent, '3/10');
	});

	test('createAgentRow Running → "running" status class and Cancel button present', () => {
		const task = makeTask(ForgeAgentStatus.Running);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.running'), 'expected .forge-agent-status.running');
		assert.ok(row.querySelector('.forge-agent-cancel-btn'), 'expected cancel button');
	});

	test('createAgentRow Completed → "completed" status class, no Cancel button', () => {
		const task = makeTask(ForgeAgentStatus.Completed);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.completed'), 'expected .forge-agent-status.completed');
		assert.strictEqual(row.querySelector('.forge-agent-cancel-btn'), null);
	});

	test('createAgentRow MaxTurnsReached → "max_turns_reached" status class', () => {
		const task = makeTask(ForgeAgentStatus.MaxTurnsReached);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.max_turns_reached'));
	});

	test('createAgentRow Error → "error" status class', () => {
		const task = makeTask(ForgeAgentStatus.Error);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.error'));
	});

	test('createAgentRow Queued → "queued" status class, no Cancel button', () => {
		const task = makeTask(ForgeAgentStatus.Queued);
		const row = createAgentRow(task, () => { });
		assert.ok(row.querySelector('.forge-agent-status.queued'));
		assert.strictEqual(row.querySelector('.forge-agent-cancel-btn'), null);
	});

	test('createAgentRow Cancel click fires onCancel with agent id', () => {
		let cancelledId: string | undefined;
		const task = makeTask(ForgeAgentStatus.Running, { id: 'agent-42' });
		const row = createAgentRow(task, (id) => { cancelledId = id; });
		row.querySelector<HTMLButtonElement>('.forge-agent-cancel-btn')!.click();
		assert.strictEqual(cancelledId, 'agent-42');
	});
});
```

- [ ] **Step 2: Run tests — expect compile failure (helper file does not exist yet)**

```bash
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts
```

Expected: error mentioning `forgeAgentMonitorViewHelpers` not found.

- [ ] **Step 3: Create the helper file**

Create `src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorViewHelpers.ts`:

```typescript
/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { ForgeAgentStatus, ForgeAgentTask } from '../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';

/** Maps ForgeAgentStatus enum → CSS class string for the status dot. */
export function getAgentStatusClass(status: ForgeAgentStatus): string {
	return status;
}

/** Stable sort: Running → Queued → Completed → MaxTurnsReached → Error. Does not mutate input. */
export function sortAgentsByStatus(agents: ForgeAgentTask[]): ForgeAgentTask[] {
	const order: Record<string, number> = {
		[ForgeAgentStatus.Running]: 0,
		[ForgeAgentStatus.Queued]: 1,
		[ForgeAgentStatus.Completed]: 2,
		[ForgeAgentStatus.MaxTurnsReached]: 3,
		[ForgeAgentStatus.Error]: 4,
	};
	return [...agents].sort((a, b) => (order[a.status] ?? 5) - (order[b.status] ?? 5));
}

/** Returns the empty-state element shown when no agent definitions are loaded. */
export function createEmptyDefinitionsState(): HTMLElement {
	const el = document.createElement('div');
	el.className = 'forge-agent-empty';
	el.textContent = 'No agent definitions found in .agents/';
	return el;
}

/** Returns the empty-state element shown when no agents are running or recent. */
export function createEmptyAgentsState(): HTMLElement {
	const el = document.createElement('div');
	el.className = 'forge-agent-empty';
	el.textContent = 'No agents running';
	return el;
}

/**
 * Creates a definition row element.
 * @param def - The agent definition to display.
 * @param disabled - Whether the agent is currently disabled.
 * @param onToggle - Called when the Enable/Disable button is clicked.
 */
export function createDefinitionRow(def: AgentDefinition, disabled: boolean, onToggle: () => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-agent-def-row';
	if (disabled) {
		row.classList.add('disabled');
	}

	const name = document.createElement('span');
	name.className = 'forge-agent-def-name';
	name.textContent = def.name;
	if (disabled) {
		name.classList.add('disabled');
	}
	row.appendChild(name);

	const desc = document.createElement('span');
	desc.className = 'forge-agent-def-desc';
	desc.textContent = def.description || '(no description)';
	row.appendChild(desc);

	const toggle = document.createElement('button');
	toggle.className = 'forge-agent-def-toggle';
	toggle.title = disabled ? `Enable ${def.name}` : `Disable ${def.name}`;
	toggle.textContent = disabled ? 'Enable' : 'Disable';
	toggle.addEventListener('click', (e) => {
		e.stopPropagation();
		onToggle();
	});
	row.appendChild(toggle);

	return row;
}

/**
 * Creates an agent task row element.
 * @param agent - The agent task to display.
 * @param onCancel - Called with the agent id when the Cancel button is clicked.
 */
export function createAgentRow(agent: ForgeAgentTask, onCancel: (id: string) => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-agent-row';

	const dot = document.createElement('span');
	dot.className = `forge-agent-status ${getAgentStatusClass(agent.status)}`;
	row.appendChild(dot);

	const name = document.createElement('span');
	name.className = 'forge-agent-row-name';
	name.textContent = agent.name;
	row.appendChild(name);

	const turns = document.createElement('span');
	turns.className = 'forge-agent-row-turns';
	turns.textContent = `${agent.currentTurn}/${agent.maxTurns}`;
	row.appendChild(turns);

	const steps = document.createElement('span');
	steps.className = 'forge-agent-row-steps';
	steps.textContent = `${agent.steps.length} steps`;
	row.appendChild(steps);

	if (agent.status === ForgeAgentStatus.Running) {
		const cancelBtn = document.createElement('button');
		cancelBtn.className = 'forge-agent-cancel-btn';
		cancelBtn.textContent = 'Cancel';
		cancelBtn.addEventListener('click', () => onCancel(agent.id));
		row.appendChild(cancelBtn);
	}

	return row;
}
```

- [ ] **Step 4: Run tests — expect all to pass**

```bash
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts
```

Expected: all tests pass (the single existing test plus the new ones).

- [ ] **Step 5: Commit**

```bash
git add \
  src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorViewHelpers.ts \
  src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts
git commit -m "feat(test): extract agent monitor view helpers and add 20 tests"
```

---

## Task 2: Refactor forgeAgentMonitorView.ts to use helpers

**Files:**
- Modify: `src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorView.ts`

- [ ] **Step 1: Update the view to import and delegate to helpers**

Replace the entire content of `src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorView.ts` with:

```typescript
/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { reset } from '../../../../base/browser/dom.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IForgeAgentService } from '../../../services/forge/common/forgeAgentService.js';
import { ForgeAgentTask } from '../../../services/forge/common/forgeAgentTypes.js';
import { AgentDefinition } from '../../../services/forge/common/forgeConfigResolutionTypes.js';
import {
	sortAgentsByStatus,
	createEmptyDefinitionsState,
	createEmptyAgentsState,
	createDefinitionRow,
	createAgentRow,
} from './forgeAgentMonitorViewHelpers.js';

export const FORGE_AGENT_MONITOR_VIEW_ID = 'workbench.forgeAI.agentMonitorView';

export class ForgeAgentMonitorView extends ViewPane {

	private listContainer!: HTMLElement;

	constructor(
		options: IViewletViewOptions,
		@IForgeAgentService private readonly agentService: IForgeAgentService,
		@IKeybindingService keybindingService: IKeybindingService,
		@IContextMenuService contextMenuService: IContextMenuService,
		@IConfigurationService configurationService: IConfigurationService,
		@IContextKeyService contextKeyService: IContextKeyService,
		@IViewDescriptorService viewDescriptorService: IViewDescriptorService,
		@IInstantiationService instantiationService: IInstantiationService,
		@IOpenerService openerService: IOpenerService,
		@IThemeService themeService: IThemeService,
		@IHoverService hoverService: IHoverService,
	) {
		super(options, keybindingService, contextMenuService, configurationService, contextKeyService, viewDescriptorService, instantiationService, openerService, themeService, hoverService);
	}

	protected override renderBody(container: HTMLElement): void {
		super.renderBody(container);
		container.classList.add('forge-agent-monitor-view');

		this.listContainer = document.createElement('div');
		this.listContainer.className = 'forge-agent-list';
		container.appendChild(this.listContainer);

		this._register(this.agentService.onDidChangeAgent(() => {
			this.renderAll();
		}));

		this.renderAll();
	}

	private renderAll(): void {
		reset(this.listContainer);

		// Section 1: Available Agent Definitions
		const defsHeader = document.createElement('div');
		defsHeader.className = 'forge-agent-section-header';
		defsHeader.textContent = 'Available Agents';
		this.listContainer.appendChild(defsHeader);

		const definitions = this.agentService.getAvailableDefinitions();
		if (definitions.length === 0) {
			this.listContainer.appendChild(createEmptyDefinitionsState());
		} else {
			for (const def of definitions) {
				this.listContainer.appendChild(this.renderDefinitionRow(def));
			}
		}

		// Section 2: Running/Recent Agents
		const runHeader = document.createElement('div');
		runHeader.className = 'forge-agent-section-header';
		runHeader.textContent = 'Running & Recent';
		this.listContainer.appendChild(runHeader);

		const agents = this.agentService.getAllAgents();
		if (agents.length === 0) {
			this.listContainer.appendChild(createEmptyAgentsState());
		} else {
			for (const agent of sortAgentsByStatus(agents)) {
				this.listContainer.appendChild(this.renderAgentRow(agent));
			}
		}
	}

	private renderDefinitionRow(def: AgentDefinition): HTMLElement {
		const isDisabled = this.agentService.isAgentDisabled(def.name);
		return createDefinitionRow(def, isDisabled, () => this.agentService.toggleAgentDisabled(def.name, !isDisabled));
	}

	private renderAgentRow(agent: ForgeAgentTask): HTMLElement {
		return createAgentRow(agent, (id) => this.agentService.cancelAgent(id));
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
```

- [ ] **Step 2: Run tests — confirm existing tests still pass**

```bash
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts
```

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/vs/workbench/contrib/forgeAI/browser/forgeAgentMonitorView.ts
git commit -m "refactor(view): delegate agent monitor render methods to helper functions"
```

---

## Task 3: MCP status view — helpers + tests

**Files:**
- Create: `src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusViewHelpers.ts`
- Modify: `src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts`

- [ ] **Step 1: Add failing tests to the existing test file**

Replace the entire content of `src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts` with:

```typescript
/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import assert from 'assert';
import { ensureNoDisposablesAreLeakedInTestSuite } from '../../../../../base/test/common/utils.js';
import { FORGE_MCP_STATUS_VIEW_ID } from '../../browser/forgeMcpStatusView.js';
import {
	getServerStatusClass,
	getToolCountText,
	createEmptyServersState,
	createServerRow,
} from '../../browser/forgeMcpStatusViewHelpers.js';
import { ForgeMcpServerStatus } from '../../../../services/forge/common/forgeMcpTypes.js';
import { ForgeMcpServerStatusEntry } from '../../../../services/forge/common/forgeMcpService.js';

function makeServer(overrides: Partial<ForgeMcpServerStatusEntry> = {}): ForgeMcpServerStatusEntry {
	return {
		name: 'test-server',
		status: ForgeMcpServerStatus.Connected,
		toolCount: 3,
		disabled: false,
		...overrides,
	};
}

suite('ForgeMcpStatusView', () => {
	ensureNoDisposablesAreLeakedInTestSuite();

	test('has correct view ID', () => {
		assert.strictEqual(FORGE_MCP_STATUS_VIEW_ID, 'workbench.forgeAI.mcpStatusView');
	});

	// --- getServerStatusClass ---

	test('getServerStatusClass Connected, not disabled → "connected"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Connected, false), 'connected');
	});

	test('getServerStatusClass Connecting, not disabled → "connecting"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Connecting, false), 'connecting');
	});

	test('getServerStatusClass Error, not disabled → "error"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Error, false), 'error');
	});

	test('getServerStatusClass Disconnected, not disabled → "disconnected"', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Disconnected, false), 'disconnected');
	});

	test('getServerStatusClass disabled=true → "disabled" regardless of status', () => {
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Connected, true), 'disabled');
		assert.strictEqual(getServerStatusClass(ForgeMcpServerStatus.Error, true), 'disabled');
	});

	// --- getToolCountText ---

	test('getToolCountText disabled=false → "<n> tools"', () => {
		assert.strictEqual(getToolCountText(5, false), '5 tools');
		assert.strictEqual(getToolCountText(0, false), '0 tools');
	});

	test('getToolCountText disabled=true → "disabled"', () => {
		assert.strictEqual(getToolCountText(5, true), 'disabled');
	});

	// --- createEmptyServersState ---

	test('createEmptyServersState returns element with correct class and text', () => {
		const el = createEmptyServersState();
		assert.strictEqual(el.className, 'forge-mcp-empty');
		assert.strictEqual(el.textContent, 'No MCP servers configured');
	});

	// --- createServerRow ---

	test('createServerRow Connected → "connected" dot class, tool count text', () => {
		const row = createServerRow(makeServer({ toolCount: 4 }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.connected'), 'expected .connected dot');
		assert.strictEqual(row.querySelector('.forge-mcp-tool-count')?.textContent, '4 tools');
	});

	test('createServerRow Connecting → "connecting" dot class', () => {
		const row = createServerRow(makeServer({ status: ForgeMcpServerStatus.Connecting }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.connecting'));
	});

	test('createServerRow Error → "error" dot class', () => {
		const row = createServerRow(makeServer({ status: ForgeMcpServerStatus.Error }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.error'));
	});

	test('createServerRow Disconnected → "disconnected" dot class', () => {
		const row = createServerRow(makeServer({ status: ForgeMcpServerStatus.Disconnected }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.disconnected'));
	});

	test('createServerRow disabled=true → "disabled" dot class, "disabled" text, "Enable" button', () => {
		const row = createServerRow(makeServer({ disabled: true }), () => { });
		assert.ok(row.querySelector('.forge-mcp-server-dot.disabled'), 'expected .disabled dot');
		assert.strictEqual(row.querySelector('.forge-mcp-tool-count')?.textContent, 'disabled');
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-mcp-server-toggle')!.textContent, 'Enable');
	});

	test('createServerRow disabled=false → "Disable" button', () => {
		const row = createServerRow(makeServer(), () => { });
		assert.strictEqual(row.querySelector<HTMLButtonElement>('.forge-mcp-server-toggle')!.textContent, 'Disable');
	});

	test('createServerRow toggle click fires onToggle callback', () => {
		let called = false;
		const row = createServerRow(makeServer(), () => { called = true; });
		row.querySelector<HTMLButtonElement>('.forge-mcp-server-toggle')!.click();
		assert.strictEqual(called, true);
	});
});
```

- [ ] **Step 2: Run tests — expect compile failure (helper file does not exist yet)**

```bash
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts
```

Expected: error mentioning `forgeMcpStatusViewHelpers` not found.

- [ ] **Step 3: Create the helper file**

Create `src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusViewHelpers.ts`:

```typescript
/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { ForgeMcpServerStatus } from '../../../services/forge/common/forgeMcpTypes.js';
import { ForgeMcpServerStatusEntry } from '../../../services/forge/common/forgeMcpService.js';

/** Maps (status, disabled) → CSS class string for the status dot. */
export function getServerStatusClass(status: ForgeMcpServerStatus, disabled: boolean): string {
	if (disabled) {
		return 'disabled';
	}
	switch (status) {
		case ForgeMcpServerStatus.Connected: return 'connected';
		case ForgeMcpServerStatus.Connecting: return 'connecting';
		case ForgeMcpServerStatus.Error: return 'error';
		default: return 'disconnected';
	}
}

/** Returns "disabled" when disabled=true, otherwise "<n> tools". */
export function getToolCountText(toolCount: number, disabled: boolean): string {
	return disabled ? 'disabled' : `${toolCount} tools`;
}

/** Returns the empty-state element shown when no MCP servers are configured. */
export function createEmptyServersState(): HTMLElement {
	const el = document.createElement('div');
	el.className = 'forge-mcp-empty';
	el.textContent = 'No MCP servers configured';
	return el;
}

/**
 * Creates a server row element.
 * @param server - The server status entry to display.
 * @param onToggle - Called when the Enable/Disable button is clicked.
 */
export function createServerRow(server: ForgeMcpServerStatusEntry, onToggle: () => void): HTMLElement {
	const row = document.createElement('div');
	row.className = 'forge-mcp-server-row';
	if (server.disabled) {
		row.classList.add('disabled');
	}

	const dot = document.createElement('span');
	dot.className = 'forge-mcp-server-dot';
	dot.classList.add(getServerStatusClass(server.status, server.disabled));
	row.appendChild(dot);

	const name = document.createElement('span');
	name.className = 'forge-mcp-server-name';
	name.textContent = server.name;
	if (server.disabled) {
		name.classList.add('disabled');
	}
	row.appendChild(name);

	const toolCount = document.createElement('span');
	toolCount.className = 'forge-mcp-tool-count';
	toolCount.textContent = getToolCountText(server.toolCount, server.disabled);
	row.appendChild(toolCount);

	const toggle = document.createElement('button');
	toggle.className = 'forge-mcp-server-toggle';
	toggle.title = server.disabled ? `Enable ${server.name}` : `Disable ${server.name}`;
	toggle.textContent = server.disabled ? 'Enable' : 'Disable';
	toggle.addEventListener('click', (e) => {
		e.stopPropagation();
		onToggle();
	});
	row.appendChild(toggle);

	return row;
}
```

- [ ] **Step 4: Run tests — expect all to pass**

```bash
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts
```

Expected: all tests pass.

- [ ] **Step 5: Commit**

```bash
git add \
  src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusViewHelpers.ts \
  src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts
git commit -m "feat(test): extract MCP status view helpers and add 11 tests"
```

---

## Task 4: Refactor forgeMcpStatusView.ts to use helpers

**Files:**
- Modify: `src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusView.ts`

- [ ] **Step 1: Update the view to import and delegate to helpers**

Replace the entire content of `src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusView.ts` with:

```typescript
/*---------------------------------------------------------------------------------------------
 *  Copyright (c) Forge IDE. All rights reserved.
 *  Licensed under the MIT License. See License.txt in the project root for license information.
 *--------------------------------------------------------------------------------------------*/

import { IKeybindingService } from '../../../../platform/keybinding/common/keybinding.js';
import { IContextMenuService } from '../../../../platform/contextview/browser/contextView.js';
import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
import { IContextKeyService } from '../../../../platform/contextkey/common/contextkey.js';
import { IInstantiationService } from '../../../../platform/instantiation/common/instantiation.js';
import { IViewDescriptorService } from '../../../common/views.js';
import { IOpenerService } from '../../../../platform/opener/common/opener.js';
import { IThemeService } from '../../../../platform/theme/common/themeService.js';
import { ITelemetryService } from '../../../../platform/telemetry/common/telemetry.js';
import { IHoverService } from '../../../../platform/hover/browser/hover.js';
import { reset } from '../../../../base/browser/dom.js';
import { ViewPane } from '../../../browser/parts/views/viewPane.js';
import { IViewletViewOptions } from '../../../browser/parts/views/viewsViewlet.js';
import { IForgeMcpService, ForgeMcpServerStatusEntry } from '../../../services/forge/common/forgeMcpService.js';
import { createEmptyServersState, createServerRow } from './forgeMcpStatusViewHelpers.js';

export const FORGE_MCP_STATUS_VIEW_ID = 'workbench.forgeAI.mcpStatusView';

export class ForgeMcpStatusView extends ViewPane {

	private listContainer!: HTMLElement;

	constructor(
		options: IViewletViewOptions,
		@IForgeMcpService private readonly forgeMcpService: IForgeMcpService,
		@IKeybindingService keybindingService: IKeybindingService,
		@IContextMenuService contextMenuService: IContextMenuService,
		@IConfigurationService configurationService: IConfigurationService,
		@IContextKeyService contextKeyService: IContextKeyService,
		@IViewDescriptorService viewDescriptorService: IViewDescriptorService,
		@IInstantiationService instantiationService: IInstantiationService,
		@IOpenerService openerService: IOpenerService,
		@IThemeService themeService: IThemeService,
		@ITelemetryService telemetryService: ITelemetryService,
		@IHoverService hoverService: IHoverService,
	) {
		super(options, keybindingService, contextMenuService, configurationService, contextKeyService, viewDescriptorService, instantiationService, openerService, themeService, hoverService);
	}

	protected override renderBody(container: HTMLElement): void {
		super.renderBody(container);
		container.classList.add('forge-mcp-status-view');

		this.listContainer = document.createElement('div');
		this.listContainer.className = 'forge-mcp-server-list';
		container.appendChild(this.listContainer);

		this._register(this.forgeMcpService.onDidChangeServerStatus(() => {
			this.renderServerList();
		}));

		this._register(this.forgeMcpService.onDidChangeTools(() => {
			this.renderServerList();
		}));

		this.renderServerList();
	}

	private renderServerList(): void {
		reset(this.listContainer);
		const statuses = this.forgeMcpService.getServerStatuses();

		if (statuses.length === 0) {
			this.listContainer.appendChild(createEmptyServersState());
			return;
		}

		for (const server of statuses) {
			this.listContainer.appendChild(this.renderServerRow(server));
		}
	}

	private renderServerRow(server: ForgeMcpServerStatusEntry): HTMLElement {
		return createServerRow(server, () => this.forgeMcpService.toggleServerDisabled(server.name, !server.disabled));
	}

	protected override layoutBody(height: number, width: number): void {
		super.layoutBody(height, width);
	}
}
```

- [ ] **Step 2: Run tests — confirm existing tests still pass**

```bash
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts
```

Expected: all tests pass.

- [ ] **Step 3: Commit**

```bash
git add src/vs/workbench/contrib/forgeAI/browser/forgeMcpStatusView.ts
git commit -m "refactor(view): delegate MCP status view render methods to helper functions"
```

---

## Task 5: Compile check

- [ ] **Step 1: Run full TypeScript compile**

```bash
npm run compile
```

Expected: no errors. If there are errors, fix them before proceeding (they will likely be import path issues — double-check `.js` extensions on all imports in the new helper files).

- [ ] **Step 2: Run both test suites together**

```bash
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeAgentMonitorView.test.ts && \
./scripts/test.sh --run src/vs/workbench/contrib/forgeAI/test/browser/forgeMcpStatusView.test.ts
```

Expected: all tests in both files pass.
