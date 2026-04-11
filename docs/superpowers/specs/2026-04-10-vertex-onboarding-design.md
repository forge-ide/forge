# Vertex AI — Onboarding Step 4 Provider Configuration

**Date:** 2026-04-10
**Status:** Approved

---

## Overview

Add Google Vertex AI as a selectable provider in the onboarding flow's "Connect a Provider" step. Vertex requires three inputs (project ID, location, optional service account JSON) rather than a single API key, so the provider card system is extended with a field schema to support multi-field providers generically.

---

## Architecture

### `ProviderDefinition` Schema Extension

Add an optional `fields` array to `ProviderDefinition`. When absent, the card uses the existing single API key input (all current providers are backward compatible with no changes).

```ts
interface ProviderField {
    id: string;
    label: string;
    placeholder?: string;
    envVar?: string;        // pre-fill from process.env if set; shows "from environment" badge
    type?: 'text' | 'json'; // 'text' = single-line input (default), 'json' = textarea + browse
    optional?: boolean;     // if true, field may be empty and step still validates
}

interface ProviderDefinition {
    id: string;
    label: string;
    description: string;
    isLocal: boolean;
    fields?: ProviderField[]; // absent → legacy single apiKey input
}
```

### Vertex Entry in `PROVIDERS`

```ts
{
    id: 'vertex',
    label: 'Google Vertex AI',
    description: 'Gemini and Claude models via Google Cloud',
    isLocal: false,
    fields: [
        { id: 'projectId',          label: 'Project ID',           envVar: 'GOOGLE_CLOUD_PROJECT',  type: 'text' },
        { id: 'location',           label: 'Location',             envVar: 'GOOGLE_CLOUD_LOCATION', type: 'text', placeholder: 'us-central1' },
        { id: 'serviceAccountJson', label: 'Service Account JSON', type: 'json', optional: true },
    ]
}
```

---

## UI / Rendering

### Text fields (`type: 'text'`)

Standard single-line input. If `envVar` is defined and the variable is present in `process.env` at startup, the field is pre-filled and displays a small "from environment" badge inline with the label.

### JSON field (`type: 'json'`)

Multi-line textarea. A "Browse..." link next to the label triggers `IFileDialogService.showOpenDialog` filtered to `.json` files. On selection, the file's contents are read and populated into the textarea. Leaving the field empty is valid (bootstrap falls back to Application Default Credentials).

### Env-var detection banner

If both `GOOGLE_CLOUD_PROJECT` and `GOOGLE_CLOUD_LOCATION` are present in the environment, the Vertex card auto-checks and displays the same top-of-card detection banner used for Anthropic:
> "We found GOOGLE_CLOUD_PROJECT and GOOGLE_CLOUD_LOCATION in your environment"

### Validation

The "Continue" button is blocked if Vertex is selected and either `projectId` or `location` is empty. `serviceAccountJson` being empty does not block progress.

---

## Credential Persistence

Field values are routed to two stores based on field `id`:

| Field | Store | Key / Path |
|---|---|---|
| `projectId` | `IConfigurationService` | `forge.providers[].projectId` |
| `location` | `IConfigurationService` | `forge.providers[].location` |
| `serviceAccountJson` | `SecretStorage` | `forge.provider.apiKey.vertex` |

`forgeProviderBootstrap.ts` already reads from both of these locations — no changes required to the bootstrap or credential service.

---

## Files to Change

- `src/vs/workbench/browser/parts/editor/forgeOnboarding/steps/step3Provider.ts`
  - Extend `ProviderDefinition` and `ProviderField` types
  - Add Vertex entry to `PROVIDERS` array
  - Update card rendering to handle `fields` array (text + json field types)
  - Update save logic to route `projectId`/`location` to config and `serviceAccountJson` to SecretStorage
  - Add env-var detection for `GOOGLE_CLOUD_PROJECT` / `GOOGLE_CLOUD_LOCATION`

---

## Out of Scope

- Changes to `forgeProviderBootstrap.ts`, `vertexProvider.ts`, or `forgeCredentialService.ts`
- Adding Vertex to the provider switcher or settings UI (separate task)
- Model selection during onboarding
