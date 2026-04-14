# Forge Provider Config → VS Code User Settings — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate Forge provider/AI configuration (`defaultProvider`, `defaultModel`, `stream`, `providers`) from `forge.json` file I/O to VS Code user settings; move ownership of `configPaths`/`disabled` directly into `ForgeConfigResolutionService`.

**Architecture:** `ForgeConfigService` drops file I/O and reads/writes via `IConfigurationService`. `ForgeConfigResolutionService` gains private `forge.json` read/write methods for the discovery fields it owns and drops its `IForgeConfigService` dependency. The `IForgeConfigService` interface is unchanged — no consumer modifications needed.

**Tech Stack:** TypeScript, VS Code DI (`createDecorator`, `registerSingleton`), `IConfigurationService`, `IFileService`, `VSBuffer`

---

### Task 1: Register forge.defaultProvider, forge.defaultModel, and forge.stream

**Files:**
- Modify: `src/vs/workbench/browser/parts/editor/forgeChat/forgeChat.contribution.ts`

- [ ] **Step 1: Add the three new property registrations**

  In the `registerConfiguration` block (currently ends at line ~227), add inside `properties`:

  ```typescript
  'forge.defaultProvider': {
      type: 'string',
      default: '',
      description: localize('forge.defaultProvider', "Name of the default AI provider (e.g. 'anthropic', 'openai', 'vertex')."),
  },
  'forge.defaultModel': {
      type: 'string',
      default: '',
      description: localize('forge.defaultModel', "Model ID to use when none is specified."),
  },
  'forge.stream': {
      type: 'boolean',
      default: true,
      description: localize('forge.stream', "Stream AI responses incrementally."),
  },
  ```

  The full `properties` block should look like:

  ```typescript
  properties: {
      'forge.autoAttachActiveEditor': {
          type: 'boolean',
          default: false,
          description: localize('forge.autoAttachActiveEditor', "Automatically attach the active code editor as context to the nearest AI chat pane."),
      },
      'forge.providers': {
          type: 'array',
          default: [],
          description: localize('forge.providers', "AI provider configurations saved by the Forge onboarding flow."),
          items: {
              type: 'object',
              properties: {
                  name: { type: 'string' },
                  baseURL: { type: 'string' },
                  envKey: { type: 'string' },
                  projectId: { type: 'string' },
                  location: { type: 'string' },
                  models: { type: 'array', items: { type: 'object' } },
              },
              required: ['name', 'models'],
          },
      },
      'forge.defaultProvider': {
          type: 'string',
          default: '',
          description: localize('forge.defaultProvider', "Name of the default AI provider (e.g. 'anthropic', 'openai', 'vertex')."),
      },
      'forge.defaultModel': {
          type: 'string',
          default: '',
          description: localize('forge.defaultModel', "Model ID to use when none is specified."),
      },
      'forge.stream': {
          type: 'boolean',
          default: true,
          description: localize('forge.stream', "Stream AI responses incrementally."),
      },
  },
  ```

- [ ] **Step 2: Verify compile**

  ```bash
  npm run compile 2>&1 | grep -E 'error|Error' | head -20
  ```

  Expected: no errors.

- [ ] **Step 3: Commit**

  ```bash
  git add src/vs/workbench/browser/parts/editor/forgeChat/forgeChat.contribution.ts
  git commit -m "feat(config): register forge.defaultProvider, forge.defaultModel, forge.stream settings"
  ```

---

### Task 2: Write failing tests for the new ForgeConfigService behavior

**Files:**
- Create: `src/vs/workbench/services/forge/common/forgeConfigService.test.ts`

- [ ] **Step 1: Create the test file**

  ```typescript
  // src/vs/workbench/services/forge/common/forgeConfigService.test.ts
  import * as assert from 'assert';
  import { IConfigurationService } from '../../../../platform/configuration/common/configuration.js';
  import { NullLogService } from '../../../../platform/log/common/log.js';
  import { ForgeConfigService } from './forgeConfigService.js';

  function makeConfigService(initial: Record<string, unknown> = {}): {
  	mockConfig: IConfigurationService;
  	updates: Array<{ key: string; value: unknown }>;
  } {
  	const store: Record<string, unknown> = { ...initial };
  	const updates: Array<{ key: string; value: unknown }> = [];
  	const listeners: Array<(e: { affectsConfiguration(k: string): boolean }) => void> = [];

  	const mockConfig: Partial<IConfigurationService> = {
  		getValue: <T>(key: string) => (store[key] as T),
  		updateValue: async (key: string, value: unknown) => {
  			store[key] = value;
  			updates.push({ key, value });
  			for (const l of listeners) {
  				l({ affectsConfiguration: (k: string) => k === key });
  			}
  		},
  		onDidChangeConfiguration: (l: (e: { affectsConfiguration(k: string): boolean }) => void) => {
  			listeners.push(l);
  			return { dispose: () => { listeners.splice(listeners.indexOf(l), 1); } };
  		},
  	};

  	return { mockConfig: mockConfig as IConfigurationService, updates };
  }

  suite('ForgeConfigService', () => {

  	test('getConfig() reads values from IConfigurationService', () => {
  		const { mockConfig } = makeConfigService({
  			'forge.defaultProvider': 'anthropic',
  			'forge.defaultModel': 'claude-opus-4-5',
  			'forge.stream': false,
  			'forge.providers': [{ name: 'anthropic', models: [] }],
  		});
  		const sut = new ForgeConfigService(mockConfig, new NullLogService());

  		const config = sut.getConfig();

  		assert.strictEqual(config.defaultProvider, 'anthropic');
  		assert.strictEqual(config.defaultModel, 'claude-opus-4-5');
  		assert.strictEqual(config.stream, false);
  		assert.deepStrictEqual(config.providers, [{ name: 'anthropic', models: [] }]);
  	});

  	test('getConfig() returns safe defaults when values are absent', () => {
  		const { mockConfig } = makeConfigService({});
  		const sut = new ForgeConfigService(mockConfig, new NullLogService());

  		const config = sut.getConfig();

  		assert.strictEqual(config.defaultProvider, '');
  		assert.strictEqual(config.stream, true);
  		assert.deepStrictEqual(config.providers, []);
  	});

  	test('updateConfig() writes each supplied field to IConfigurationService', async () => {
  		const { mockConfig, updates } = makeConfigService({});
  		const sut = new ForgeConfigService(mockConfig, new NullLogService());

  		await sut.updateConfig({ defaultProvider: 'openai', stream: false });

  		assert.strictEqual(updates.length, 2);
  		assert.ok(updates.some(u => u.key === 'forge.defaultProvider' && u.value === 'openai'));
  		assert.ok(updates.some(u => u.key === 'forge.stream' && u.value === false));
  	});

  	test('updateConfig() does not write unspecified fields', async () => {
  		const { mockConfig, updates } = makeConfigService({});
  		const sut = new ForgeConfigService(mockConfig, new NullLogService());

  		await sut.updateConfig({ defaultProvider: 'openai' });

  		assert.strictEqual(updates.length, 1);
  		assert.strictEqual(updates[0].key, 'forge.defaultProvider');
  	});

  	test('onDidChange fires when a forge key changes', async () => {
  		const { mockConfig } = makeConfigService({});
  		const sut = new ForgeConfigService(mockConfig, new NullLogService());

  		let firedCount = 0;
  		sut.onDidChange(() => { firedCount++; });

  		await sut.updateConfig({ defaultProvider: 'anthropic' });

  		assert.strictEqual(firedCount, 1);
  	});

  	test('onDidChange does not fire when an unrelated key changes', () => {
  		const store: Record<string, unknown> = {};
  		const listeners: Array<(e: { affectsConfiguration(k: string): boolean }) => void> = [];
  		const mockConfig: Partial<IConfigurationService> = {
  			getValue: <T>(key: string) => store[key] as T,
  			updateValue: async () => {},
  			onDidChangeConfiguration: (l) => {
  				listeners.push(l);
  				return { dispose: () => {} };
  			},
  		};
  		const sut = new ForgeConfigService(mockConfig as IConfigurationService, new NullLogService());

  		let firedCount = 0;
  		sut.onDidChange(() => { firedCount++; });

  		// Simulate a change to an unrelated key
  		for (const l of listeners) {
  			l({ affectsConfiguration: (k: string) => k === 'editor.fontSize' });
  		}

  		assert.strictEqual(firedCount, 0);
  	});
  });
  ```

- [ ] **Step 2: Run the tests and confirm they fail**

  ```bash
  ./scripts/test.sh --run src/vs/workbench/services/forge/common/forgeConfigService.test.ts 2>&1 | tail -20
  ```

  Expected: tests fail because `ForgeConfigService` constructor still expects `IFileService`, not `IConfigurationService`.

---

### Task 3: Rewrite ForgeConfigService to use IConfigurationService

**Files:**
- Modify: `src/vs/workbench/services/forge/common/forgeConfigService.ts`

- [ ] **Step 1: Replace the file contents**

  ```typescript
  /*---------------------------------------------------------------------------------------------
   *  Copyright (c) Forge IDE. All rights reserved.
   *  Licensed under the MIT License. See License.txt in the project root for license information.
   *--------------------------------------------------------------------------------------------*/

  import { Emitter, Event } from '../../../../base/common/event.js';
  import { Disposable } from '../../../../base/common/lifecycle.js';
  import { IConfigurationService, ConfigurationTarget } from '../../../../platform/configuration/common/configuration.js';
  import { InstantiationType, registerSingleton } from '../../../../platform/instantiation/common/extensions.js';
  import { createDecorator } from '../../../../platform/instantiation/common/instantiation.js';
  import { ILogService } from '../../../../platform/log/common/log.js';

  import { type ForgeConfig, type ForgeProviderConfig, resolveModelConfig, type ResolvedModelConfig } from './forgeConfigTypes.js';

  export type { ForgeConfig };

  export const IForgeConfigService = createDecorator<IForgeConfigService>('forgeConfigService');

  export interface IForgeConfigService {
  	readonly _serviceBrand: undefined;
  	readonly onDidChange: Event<ForgeConfig>;
  	getConfig(): ForgeConfig;
  	updateConfig(partial: Partial<ForgeConfig>): Promise<void>;
  	resolveModel(providerName?: string, modelId?: string): ResolvedModelConfig | undefined;
  	getProviders(): readonly ForgeProviderConfig[];
  }

  export class ForgeConfigService extends Disposable implements IForgeConfigService {

  	declare readonly _serviceBrand: undefined;

  	private readonly _onDidChange = this._register(new Emitter<ForgeConfig>());
  	readonly onDidChange = this._onDidChange.event;

  	constructor(
  		@IConfigurationService private readonly configurationService: IConfigurationService,
  		@ILogService private readonly logService: ILogService,
  	) {
  		super();

  		this._register(
  			this.configurationService.onDidChangeConfiguration(e => {
  				if (
  					e.affectsConfiguration('forge.defaultProvider') ||
  					e.affectsConfiguration('forge.defaultModel') ||
  					e.affectsConfiguration('forge.stream') ||
  					e.affectsConfiguration('forge.providers')
  				) {
  					this._onDidChange.fire(this.getConfig());
  				}
  			})
  		);
  	}

  	getConfig(): ForgeConfig {
  		return {
  			defaultProvider: this.configurationService.getValue<string>('forge.defaultProvider') ?? '',
  			defaultModel: this.configurationService.getValue<string>('forge.defaultModel') ?? '',
  			stream: this.configurationService.getValue<boolean>('forge.stream') ?? true,
  			providers: this.configurationService.getValue<ForgeProviderConfig[]>('forge.providers') ?? [],
  		};
  	}

  	resolveModel(providerName?: string, modelId?: string): ResolvedModelConfig | undefined {
  		return resolveModelConfig(this.getConfig(), providerName, modelId);
  	}

  	getProviders(): readonly ForgeProviderConfig[] {
  		return this.getConfig().providers;
  	}

  	async updateConfig(partial: Partial<ForgeConfig>): Promise<void> {
  		const updates: Promise<void>[] = [];

  		if ('defaultProvider' in partial) {
  			updates.push(this.configurationService.updateValue('forge.defaultProvider', partial.defaultProvider, ConfigurationTarget.USER));
  		}
  		if ('defaultModel' in partial) {
  			updates.push(this.configurationService.updateValue('forge.defaultModel', partial.defaultModel, ConfigurationTarget.USER));
  		}
  		if ('stream' in partial) {
  			updates.push(this.configurationService.updateValue('forge.stream', partial.stream, ConfigurationTarget.USER));
  		}
  		if ('providers' in partial) {
  			updates.push(this.configurationService.updateValue('forge.providers', partial.providers, ConfigurationTarget.USER));
  		}

  		if (updates.length === 0) {
  			return;
  		}

  		try {
  			await Promise.all(updates);
  		} catch (error) {
  			this.logService.warn('[ForgeConfigService] Failed to update configuration', error);
  			throw error;
  		}
  	}
  }

  registerSingleton(IForgeConfigService, ForgeConfigService, InstantiationType.Eager);
  ```

- [ ] **Step 2: Run the tests and confirm they pass**

  ```bash
  ./scripts/test.sh --run src/vs/workbench/services/forge/common/forgeConfigService.test.ts 2>&1 | tail -20
  ```

  Expected: all 5 tests pass.

- [ ] **Step 3: Compile check**

  ```bash
  npm run compile 2>&1 | grep -E 'error|Error' | head -20
  ```

  Expected: no errors.

- [ ] **Step 4: Commit**

  ```bash
  git add src/vs/workbench/services/forge/common/forgeConfigService.ts \
          src/vs/workbench/services/forge/common/forgeConfigService.test.ts
  git commit -m "feat(config): back ForgeConfigService with IConfigurationService instead of forge.json"
  ```

---

### Task 4: Add forge.json discovery-config I/O to ForgeConfigResolutionService

**Files:**
- Modify: `src/vs/workbench/services/forge/browser/forgeConfigResolution.ts`

This task removes the `IForgeConfigService` dependency and replaces it with direct file reads/writes for `configPaths` and `disabled`.

- [ ] **Step 1: Update imports and constructor**

  Replace the import block and constructor with:

  ```typescript
  import { VSBuffer } from '../../../../base/common/buffer.js';
  import { Disposable } from '../../../../base/common/lifecycle.js';
  import { Emitter, Event } from '../../../../base/common/event.js';
  import { IFileService } from '../../../../platform/files/common/files.js';
  import { URI } from '../../../../base/common/uri.js';
  import { ILogService } from '../../../../platform/log/common/log.js';
  import { IEnvironmentService } from '../../../../platform/environment/common/environment.js';
  import { IForgeConfigResolutionService } from '../common/forgeConfigResolution.js';
  import {
  	ResolvedConfig,
  	McpServerEntry,
  	AgentDefinition,
  	SkillDefinition,
  	DisabledConfig,
  	ConfigPaths,
  	parseMcpJson,
  	parseAgentMarkdown,
  } from '../common/forgeConfigResolutionTypes.js';
  import { parseSkillMarkdown } from '../common/forgeSkillTypes.js';
  import { IPathService } from '../../path/common/pathService.js';
  ```

  And constructor:

  ```typescript
  export class ForgeConfigResolutionService
  	extends Disposable
  	implements IForgeConfigResolutionService {
  	declare readonly _serviceBrand: undefined;

  	private readonly _onDidChangeResolved = this._register(
  		new Emitter<ResolvedConfig>(),
  	);
  	readonly onDidChangeResolved: Event<ResolvedConfig> =
  		this._onDidChangeResolved.event;

  	private _cached: ResolvedConfig | undefined;
  	private _lastWorkspaceRoot: string | undefined;
  	private _debounceTimer: ReturnType<typeof setTimeout> | undefined;
  	private _watchersInitialized = false;

  	private readonly _globalForgeJsonUri: URI;

  	constructor(
  		@IFileService private readonly fileService: IFileService,
  		@IEnvironmentService environmentService: IEnvironmentService,
  		@IPathService private readonly pathService: IPathService,
  		@ILogService private readonly logService: ILogService,
  	) {
  		super();
  		this._globalForgeJsonUri = URI.joinPath(
  			environmentService.userRoamingDataHome,
  			'forge',
  			'forge.json',
  		);
  	}
  ```

- [ ] **Step 2: Add readDiscoveryConfig() and writeDiscoveryConfig() private methods**

  Add these two private methods to the class (before `_initFileWatchers`):

  ```typescript
  private async readDiscoveryConfig(workspaceRoot: string): Promise<{ configPaths?: ConfigPaths; disabled?: DisabledConfig }> {
  	const candidates = [
  		URI.joinPath(URI.file(workspaceRoot), 'forge.json'),
  		this._globalForgeJsonUri,
  	];

  	for (const uri of candidates) {
  		try {
  			const content = await this.fileService.readFile(uri);
  			const parsed = JSON.parse(content.value.toString()) as { configPaths?: ConfigPaths; disabled?: DisabledConfig };
  			if (parsed.configPaths !== undefined || parsed.disabled !== undefined) {
  				return {
  					configPaths: parsed.configPaths,
  					disabled: parsed.disabled,
  				};
  			}
  		} catch {
  			// File absent or unreadable — try next candidate
  		}
  	}

  	return {};
  }

  private async writeDiscoveryConfig(partial: { configPaths?: ConfigPaths; disabled?: DisabledConfig }): Promise<void> {
  	let existing: Record<string, unknown> = {};
  	try {
  		const content = await this.fileService.readFile(this._globalForgeJsonUri);
  		existing = JSON.parse(content.value.toString()) as Record<string, unknown>;
  	} catch {
  		// File doesn't exist yet — will be created
  	}

  	const updated = { ...existing, ...partial };
  	const serialized = JSON.stringify(updated, undefined, '\t');
  	await this.fileService.writeFile(this._globalForgeJsonUri, VSBuffer.fromString(serialized));
  }
  ```

- [ ] **Step 3: Update resolve() to use readDiscoveryConfig()**

  Replace the opening lines of `resolve()` that access `configService`:

  Old:
  ```typescript
  async resolve(workspaceRoot: string): Promise<ResolvedConfig> {
  	const config = this.configService.getConfig();
  	const homeUri = await this.pathService.userHome();
  	const homePath = homeUri.path;
  	const configPaths = config.configPaths;
  	const disabled: DisabledConfig = config.disabled ?? {
  		mcpServers: [],
  		agents: [],
  	};
  ```

  New:
  ```typescript
  async resolve(workspaceRoot: string): Promise<ResolvedConfig> {
  	const { configPaths, disabled: rawDisabled } = await this.readDiscoveryConfig(workspaceRoot);
  	const disabled: DisabledConfig = rawDisabled ?? { mcpServers: [], agents: [] };
  	const homeUri = await this.pathService.userHome();
  	const homePath = homeUri.path;
  ```

- [ ] **Step 4: Update setMcpServerDisabled() to use readDiscoveryConfig/writeDiscoveryConfig**

  Replace the method body:

  ```typescript
  async setMcpServerDisabled(
  	serverName: string,
  	disabled: boolean,
  ): Promise<void> {
  	const { disabled: current } = await this.readDiscoveryConfig(this._lastWorkspaceRoot ?? '');
  	const currentDisabled = current ?? { mcpServers: [], agents: [] };
  	const mcpSet = new Set(currentDisabled.mcpServers);

  	if (disabled) {
  		mcpSet.add(serverName);
  	} else {
  		mcpSet.delete(serverName);
  	}

  	await this.writeDiscoveryConfig({
  		disabled: { mcpServers: Array.from(mcpSet), agents: currentDisabled.agents },
  	});
  }
  ```

- [ ] **Step 5: Update setAgentDisabled() to use readDiscoveryConfig/writeDiscoveryConfig**

  Replace the method body:

  ```typescript
  async setAgentDisabled(agentName: string, disabled: boolean): Promise<void> {
  	const { disabled: current } = await this.readDiscoveryConfig(this._lastWorkspaceRoot ?? '');
  	const currentDisabled = current ?? { mcpServers: [], agents: [] };
  	const agentSet = new Set(currentDisabled.agents);

  	if (disabled) {
  		agentSet.add(agentName);
  	} else {
  		agentSet.delete(agentName);
  	}

  	await this.writeDiscoveryConfig({
  		disabled: { mcpServers: currentDisabled.mcpServers, agents: Array.from(agentSet) },
  	});
  }
  ```

- [ ] **Step 6: Compile check**

  ```bash
  npm run compile 2>&1 | grep -E 'error|Error' | head -20
  ```

  Expected: no errors. If TypeScript complains about `ConfigPaths` not being exported from `forgeConfigResolutionTypes.ts`, export it from there.

- [ ] **Step 7: Commit**

  ```bash
  git add src/vs/workbench/services/forge/browser/forgeConfigResolution.ts
  git commit -m "refactor(config): ForgeConfigResolutionService owns forge.json discovery config directly"
  ```

---

### Task 5: Remove configPaths and disabled from ForgeConfig type

**Files:**
- Modify: `src/vs/workbench/services/forge/common/forgeConfigTypes.ts`

- [ ] **Step 1: Remove the two fields and their import**

  Remove the import line:
  ```typescript
  import { ConfigPaths, DisabledConfig } from './forgeConfigResolutionTypes.js';
  ```

  Remove from `ForgeConfig`:
  ```typescript
  readonly configPaths?: ConfigPaths;
  readonly disabled?: DisabledConfig;
  ```

  The resulting `ForgeConfig` interface should be:

  ```typescript
  export interface ForgeConfig {
  	readonly defaultProvider: string;
  	readonly defaultModel?: string;
  	readonly stream?: boolean;
  	readonly providers: ForgeProviderConfig[];
  }
  ```

- [ ] **Step 2: Compile check — confirm no consumers reference the removed fields**

  ```bash
  npm run compile 2>&1 | grep -E 'error|Error' | head -20
  ```

  Expected: no errors. If any file still reads `config.configPaths` or `config.disabled` via `IForgeConfigService`, that is a bug — find it with:

  ```bash
  grep -r 'configPaths\|\.disabled' src/vs/workbench/services/forge/ --include='*.ts' -l
  ```

  Only `forgeConfigResolution.ts` (browser) should appear, and only inside `readDiscoveryConfig`/`writeDiscoveryConfig` where it reads raw JSON — not via `ForgeConfig`.

- [ ] **Step 3: Commit**

  ```bash
  git add src/vs/workbench/services/forge/common/forgeConfigTypes.ts
  git commit -m "refactor(config): remove configPaths and disabled from ForgeConfig"
  ```

---

### Task 6: Full compile and validate

- [ ] **Step 1: Full compile**

  ```bash
  npm run compile 2>&1 | grep -E 'error|Error' | head -20
  ```

  Expected: no errors.

- [ ] **Step 2: Run the ForgeConfigService tests**

  ```bash
  ./scripts/test.sh --run src/vs/workbench/services/forge/common/forgeConfigService.test.ts 2>&1 | tail -20
  ```

  Expected: all 5 tests pass.

- [ ] **Step 3: Run forge:validate**

  Use the `forge:validate` skill to run all static validation checks.

- [ ] **Step 4: Final commit if any lint fixes were needed**

  ```bash
  git add -A
  git commit -m "chore: fix any lint issues from validation"
  ```
