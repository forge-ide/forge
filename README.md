# Forge IDE

Forge IDE is an AI-agnostic development environment — a fork of VS Code that replaces the assumption of a single AI backend with a first-class, configurable, multi-provider AI system built into the core of the editor.

Forge is a tool, not a product. It does not have opinions about which AI provider you should use. It has opinions about how a development environment should be structured.

## Key Features

**Quad-split canvas.** The editor surface is divided into up to four independent panes. Each pane connects to its own AI provider with its own conversation context. Switch providers per-pane without affecting anything else.

**Provider abstraction.** Anthropic, OpenAI, and local models are first-class citizens behind a common interface. Switching providers is a configuration change, not a code change. Adding a new provider means implementing one interface.

**MCP server management.** Forge manages Model Context Protocol servers and exposes tool calls as inline, collapsible cards in the chat thread. Tool results are visible and inspectable — not hidden inside an opaque context window.

**Sub-agent system.** Multi-step file tasks are delegated to sub-agents that run inside the editor process. Each step is traced in the thread view. Agents stop at a defined turn limit — they do not run indefinitely.

**Adaptive onboarding.** The first-run flow walks through provider selection, API key storage, and MCP configuration. Keys are stored in the OS secret store — never written to disk as plaintext.

## Built on VS Code

Forge is a fork of [VS Code](https://github.com/microsoft/vscode) (Code - OSS), the open-source editor from Microsoft. The VS Code codebase is used as a foundation — the editor core, extension host, language services, and workbench shell are inherited from upstream.

Forge-specific code lives in:

- `src/vs/platform/ai/` — provider interface and implementations
- `src/vs/workbench/services/forge/` — layout, MCP, agents, config
- `src/vs/workbench/browser/forge/` — onboarding and Forge UI
- `extensions/forge-theme/` — default visual theme

Distributed under the MIT license.

## Getting Started

See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) for build and development instructions.

## Contributing

Bug reports and feature requests: <https://github.com/forge-ide/forge/issues>

Before contributing code, read [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) and [`.claude/CLAUDE.md`](.claude/CLAUDE.md). They describe the layer constraints, service patterns, and design rules that all changes must follow.

## License

MIT License. Copyright Jeff Roche.

The VS Code portions of this codebase are copyright Microsoft Corporation, also under the MIT license.
