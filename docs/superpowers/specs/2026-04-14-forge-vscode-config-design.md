# Forge Provider Config → VS Code User Settings

**Date:** 2026-04-14
**Scope:** Migrate provider/AI configuration from `forge.json` to VS Code user settings. Discovery config (`configPaths`, `disabled`) stays file-based.

---

## Problem

`forge.json` is a custom file format that users must manually locate, edit, and maintain. Provider/AI settings (`defaultProvider`, `defaultModel`, `stream`, `providers`) belong in VS Code user settings where they benefit from the settings UI, workspace vs. user scoping, and first-class VS Code tooling. This is a clean break — no migration of existing `forge.json` provider config.

---

## Scope

**In scope:** `defaultProvider`, `defaultModel`, `stream`, `providers[]`

**Out of scope:** `configPaths`, `disabled` — these stay in `forge.json`, read directly by `ForgeConfigResolutionService`

---

## Design

### 1. Configuration Schema

Register four keys in `forgeChat.contribution.ts`:

| Key | Type | Default |
|---|---|---|
| `forge.providers` | `array` of `ForgeProviderConfig` | `[]` |
| `forge.defaultProvider` | `string` | `""` |
| `forge.defaultModel` | `string` | `""` |
| `forge.stream` | `boolean` | `true` |

`forge.providers` is already registered (added in the preceding fix). The remaining three are new.

### 2. `ForgeConfigService` Implementation

**Constructor changes:**
- Inject `IConfigurationService`; remove `IFileService` (no longer needed for provider config)
- Remove `workspaceConfigUri`, `globalConfigUri`, file watcher setup, and `loadConfigAsync()`

**`getConfig()`:**
Reads live from `IConfigurationService.getValue<T>('forge.*')` and assembles `ForgeConfig`. No cached state — VS Code settings are always in-memory.

```
getConfig(): ForgeConfig {
  return {
    defaultProvider: this._configurationService.getValue<string>('forge.defaultProvider') ?? '',
    defaultModel:    this._configurationService.getValue<string>('forge.defaultModel') ?? '',
    stream:          this._configurationService.getValue<boolean>('forge.stream') ?? true,
    providers:       this._configurationService.getValue<ForgeProviderConfig[]>('forge.providers') ?? [],
  };
}
```

**`onDidChange`:**
Subscribe to `IConfigurationService.onDidChangeConfiguration`, filter for `forge.defaultProvider`, `forge.defaultModel`, `forge.stream`, or `forge.providers`, then fire `_onDidChange` with the new config. Event signature stays `Event<ForgeConfig>` — no callsite changes.

**`updateConfig(partial)`:**
Each field in `partial` maps to a `configurationService.updateValue('forge.<field>', value, ConfigurationTarget.USER)` call. All writes run in parallel via `Promise.all`.

**`resolveModel()`:** No change — calls `getConfig()` internally.

**`IForgeConfigService` interface:** Unchanged. Zero callsite modifications required.

### 3. `forge.json` Scope Reduction

`ForgeConfigService` stops reading `forge.json` entirely.

`ForgeConfigResolutionService` gets a new private `readDiscoveryConfig()` method that reads `forge.json` directly on each `resolve()` call. No file watcher needed — `resolve()` is already triggered by workspace changes.

A local `ForgeDiscoveryConfig` type (`{ configPaths?: ConfigPaths; disabled?: DisabledConfig }`) is defined privately inside `forgeConfigResolution.ts`. It is not exported.

### 4. Type Changes (`forgeConfigTypes.ts`)

`ForgeConfig` drops `configPaths` and `disabled`:

```typescript
interface ForgeConfig {
  readonly defaultProvider: string;
  readonly defaultModel?: string;
  readonly stream?: boolean;
  readonly providers: ForgeProviderConfig[];
  // configPaths and disabled removed — owned by ForgeConfigResolutionService
}
```

`ConfigPaths` and `DisabledConfig` types remain in `forgeConfigTypes.ts` (still used by `ForgeConfigResolutionService`).

`ForgeProviderConfig`, `ForgeModelConfig`, `ResolvedModelConfig`, and `PROVIDER_ENV_VARS` are unchanged.

---

## Files Changed

| File | Change |
|---|---|
| `forgeChat.contribution.ts` | Register `forge.defaultProvider`, `forge.defaultModel`, `forge.stream` |
| `forgeConfigTypes.ts` | Remove `configPaths` and `disabled` from `ForgeConfig` |
| `forgeConfigService.ts` (common) | Drop file I/O; read/write via `IConfigurationService` |
| `forgeConfigService.ts` (browser/electron impl) | Same — swap backing |
| `forgeConfigResolution.ts` | Add `readDiscoveryConfig()` private method; stop reading those fields via `IForgeConfigService` |

---

## Non-Goals

- No migration of existing `forge.json` provider config
- No changes to `IForgeConfigService` interface
- No changes to `ForgeProviderBootstrap`, chat views, or any other consumers
- No changes to secret storage (API keys stay in `ISecretStorageService`)
