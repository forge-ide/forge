# Configuration Guide

Forge uses a `forge.json` file to configure AI providers, models, and behavior. This guide covers everything you need to set up and customize your configuration.

---

## Config File Location

Forge looks for `forge.json` in two locations, in order:

1. **Workspace** — `<workspace-root>/forge.json` (takes priority)
2. **Global** — `<user-data-dir>/forge/forge.json` (fallback)

If neither exists, Forge starts with no providers configured.

Both files are watched for changes. Edits are picked up automatically (100ms debounce) — no restart required.

---

## Schema

```jsonc
{
  // Required. Must match a provider name below.
  "defaultProvider": "anthropic",

  // Optional. Falls back to the first model in the default provider's list.
  "defaultModel": "claude-sonnet-4-6",

  // Optional. Enable streaming responses. Default: true.
  "stream": true,

  // Required. At least one provider.
  "providers": [
    {
      // Required. Provider identifier.
      "name": "anthropic",

      // Optional. Custom API endpoint. Overrides SDK default.
      "baseURL": "https://api.anthropic.com",

      // Optional. Custom env var for the API key.
      // Falls back to the built-in default (see Credentials below).
      "envKey": "ANTHROPIC_API_KEY",

      // Required. At least one model.
      "models": [
        {
          // Required. Model identifier sent to the provider API.
          "id": "claude-sonnet-4-6",

          // Optional. Max output tokens. Default: 4096.
          "maxTokens": 4096,

          // Optional. Context budget in tokens. Default: 8000.
          "contextBudget": 8000
        }
      ]
    }
  ]
}
```

### Field Reference

| Field | Type | Required | Default | Description |
|---|---|---|---|---|
| `defaultProvider` | string | Yes | — | Provider name for new panes. Must match a provider's `name`. |
| `defaultModel` | string | No | First model in default provider | Model to use when none specified. |
| `stream` | boolean | No | `true` | Enable streaming responses globally. |
| `providers` | array | Yes | — | Provider configurations. At least one required. |
| `providers[].name` | string | Yes | — | Provider ID (`"anthropic"`, `"openai"`, `"google"`, `"local"`, etc.) |
| `providers[].baseURL` | string | No | SDK default | Custom API endpoint. |
| `providers[].envKey` | string | No | See Credentials | Env var name for API key lookup. |
| `providers[].models` | array | Yes | — | Available models. At least one required. |
| `providers[].models[].id` | string | Yes | — | Model identifier (e.g., `"claude-sonnet-4-6"`, `"gpt-4o"`). |
| `providers[].models[].maxTokens` | number | No | `4096` | Maximum output tokens. |
| `providers[].models[].contextBudget` | number | No | `8000` | Context budget in tokens. |

---

## Credentials

Forge resolves API keys for each provider in this order:

1. **System keychain** — stored via the Forge onboarding flow or `setApiKey` command (key: `forge.provider.<name>`)
2. **Environment variable** — checked if keychain has no entry
3. **Skip** — provider is silently skipped if no credential is found

### Default Environment Variables

| Provider | Default Env Var |
|---|---|
| `anthropic` | `ANTHROPIC_API_KEY` |
| `openai` | `OPENAI_API_KEY` |
| `google` | `GOOGLE_API_KEY` |

Override the env var name with the `envKey` field on any provider:

```json
{
  "name": "google",
  "envKey": "GEMINI_API_KEY",
  "models": [{ "id": "gemini-2.0-flash" }]
}
```

Providers without valid credentials are skipped during startup — they don't cause errors.

---

## Examples

### Minimal — Single Provider

```json
{
  "defaultProvider": "anthropic",
  "providers": [
    {
      "name": "anthropic",
      "models": [
        { "id": "claude-sonnet-4-6" }
      ]
    }
  ]
}
```

### Multi-Provider with Model Overrides

```json
{
  "defaultProvider": "anthropic",
  "defaultModel": "claude-opus-4-6",
  "stream": true,
  "providers": [
    {
      "name": "anthropic",
      "models": [
        { "id": "claude-sonnet-4-6", "maxTokens": 4096 },
        { "id": "claude-opus-4-6", "maxTokens": 8192, "contextBudget": 10000 },
        { "id": "claude-haiku-4-5", "maxTokens": 2048 }
      ]
    },
    {
      "name": "openai",
      "models": [
        { "id": "gpt-4o" },
        { "id": "gpt-4o-mini", "maxTokens": 2048 }
      ]
    }
  ]
}
```

### Custom Endpoint / Local Model

```json
{
  "defaultProvider": "local",
  "providers": [
    {
      "name": "local",
      "baseURL": "http://localhost:11434/v1",
      "models": [
        { "id": "llama3" },
        { "id": "codellama" }
      ]
    }
  ]
}
```

### Corporate Proxy

```json
{
  "defaultProvider": "openai",
  "providers": [
    {
      "name": "openai",
      "baseURL": "https://api-proxy.example.com/v1",
      "envKey": "CORP_OPENAI_KEY",
      "models": [
        { "id": "gpt-4o" }
      ]
    }
  ]
}
```

---

## MCP Servers (.mcp.json)

MCP servers are configured in `.mcp.json` files using the ecosystem-standard format shared by Claude Code, Cursor, Windsurf, and Junie.

**Locations (later wins on name conflict):**
- `~/.mcp.json` — global personal servers
- Directories listed in `forge.json` `configPaths.mcp`
- `.mcp.json` in the project root — project-specific servers

**Format:**
```json
{
  "mcpServers": {
    "server-name": {
      "command": "npx",
      "args": ["-y", "package-name"],
      "env": { "KEY": "${env:KEY}" }
    }
  }
}
```

---

## Agent Definitions (.agents/)

Agent definitions are markdown files with YAML frontmatter.

**Locations (later wins on name conflict):** `~/.agents/`, `configPaths.agents` entries, `.agents/`

**Format:**
```markdown
---
name: agent-name
description: What this agent does
tools: [filesystem, github]
maxTurns: 10
provider: anthropic
model: claude-sonnet-4-6
---

System prompt for the agent goes here.
```

---

## Skill Definitions (.skills/)

Skill definitions follow the same format as agent definitions.

**Locations (later wins on name conflict):** `~/.skills/`, `configPaths.skills` entries, `.skills/`

---

## Additional Config Paths (forge.json)

`configPaths` adds extra search directories for MCP servers, agents, and skills. `disabled` filters entries from the resolved set.

```json
{
  "configPaths": {
    "mcp": ["~/shared/mcp-configs/"],
    "agents": ["~/my-agents/"],
    "skills": ["~/my-skills/"]
  },
  "disabled": {
    "mcpServers": ["server-to-disable"],
    "agents": ["agent-to-disable"]
  }
}
```

---

## How Settings Resolve

Model settings follow a fallback chain:

```
Model-level override  →  Global default  →  Hardcoded default
```

For example, given this config:

```json
{
  "stream": false,
  "providers": [{
    "name": "anthropic",
    "models": [
      { "id": "claude-sonnet-4-6" },
      { "id": "claude-opus-4-6", "maxTokens": 8192 }
    ]
  }]
}
```

- `claude-sonnet-4-6` gets: `maxTokens: 4096` (default), `contextBudget: 8000` (default), `stream: false` (global)
- `claude-opus-4-6` gets: `maxTokens: 8192` (override), `contextBudget: 8000` (default), `stream: false` (global)

---

## Troubleshooting

**No providers appear after startup**
- Verify `forge.json` is valid JSON (check for trailing commas)
- Confirm `defaultProvider` matches a provider `name` exactly
- Ensure the API key is set via keychain or environment variable

**Provider is skipped silently**
- No credential was found. Set the env var or store the key via Forge's onboarding flow.
- Check `envKey` spelling if using a custom env var name.

**Config changes aren't picked up**
- File watcher has a 100ms debounce. Wait a moment and check again.
- If editing the global config, make sure there isn't a workspace-level `forge.json` taking priority.

**Malformed JSON**
- Forge logs a warning and falls back to the next config location (or empty defaults).
- The editor remains functional — you won't lose your session.
