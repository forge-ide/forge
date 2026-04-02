# Contributing to Forge IDE

Thank you for your interest in contributing to Forge! This document covers how to get involved, report issues, and submit changes.

## Asking Questions

Have a question about using or building Forge? Open a [GitHub Discussion](https://github.com/forge-ide/forge/discussions) — it's the best place for Q&A, ideas, and general conversation.

## Reporting Issues

Found a bug or want to request a feature? [Open an issue](https://github.com/forge-ide/forge/issues/new) on GitHub.

### Before Opening an Issue

- Search [existing issues](https://github.com/forge-ide/forge/issues) to avoid duplicates.
- If the issue is with an AI provider (Anthropic, OpenAI, etc.) rather than Forge itself, check whether it reproduces with a different provider.

### What to Include in a Bug Report

- Forge version (`Help > About`)
- Operating system and version
- Steps to reproduce (numbered, minimal)
- What you expected vs. what happened
- Any error output from `Help > Toggle Developer Tools`

## Branching Strategy

Forge uses a **forking model**. All contributions — including from maintainers — go through a pull request from a personal fork. Nobody pushes directly to `forge-ide/forge`.

### Workflow

```sh
1. Fork forge-ide/forge to your GitHub account
2. Clone your fork: git clone https://github.com/YOUR_USERNAME/forge.git
3. Add the upstream remote: git remote add upstream https://github.com/forge-ide/forge.git
4. Create a branch off main: git checkout -b fix/mcp-reconnect
5. Make your changes and commit
6. Keep your branch up to date: git fetch upstream && git rebase upstream/main
7. Push to your fork: git push origin fix/mcp-reconnect
8. Open a pull request: your fork's branch → forge-ide/forge:main
```

### Branch naming

| Prefix | Use for | Example |
| --- | --- | --- |
| `feature/` | New capabilities | `feature/gemini-provider` |
| `fix/` | Bug fixes | `fix/mcp-reconnect-backoff` |
| `design/` | Visual / CSS only | `design/ember-token-update` |
| `docs/` | Documentation only | `docs/branching-strategy` |

Names should be lowercase, hyphen-separated, and specific.

### The `upstream-sync` branch

There is an `upstream-sync` branch that tracks `microsoft/vscode:main` and is merged into `main` monthly. This is a maintainer-only branch — do not base work on it and do not merge it into your feature branch. Always rebase against `main`.

---

## Contributing Code

### Setup

```bash
git clone https://github.com/forge-ide/forge.git
cd forge
npm install
npm run compile
./scripts/code.sh   # macOS / Linux
scripts\code.bat    # Windows
```

### Before You Write Code

Read [AGENT.md](../.claude/CLAUDE.md) — it explains the layered architecture, DI system, and what is safe to change. It also lists areas that require extra care and things that must not be changed. Reading it fully will save you time.

### Code Style

- Tabs for indentation, not spaces
- `noImplicitAny` and `strictNullChecks` are enforced — never use `any`
- PascalCase for types and enums; camelCase for functions, methods, and variables
- Arrow functions preferred over anonymous function expressions
- Always wrap loop and conditional bodies in curly braces
- Use `async/await` — do not mix with `.then()/.catch()` chains
- Wrap all event listener subscriptions in `this._register(...)` to prevent leaks

### Design Rules

All UI changes must follow [DESIGN.md](DESIGN.md) (in this same folder):

- Colors: `var(--color-*)` tokens only — no raw hex values
- Fonts: Barlow Condensed (headings), Barlow (body), Fira Code (code/identifiers) — no others
- Spacing: `var(--sp-*)` tokens only — no raw pixel values
- Active state: `iron-750` background + `ember-400` indicator (left border for items, bottom border for tabs)

### Testing Your Changes

Run the relevant unit tests before submitting:

```bash
./scripts/test.sh --run src/vs/platform/ai/test/common/providerRegistry.test.ts  # common/browser tests
npm run test-node -- --run src/vs/platform/ai/test/node/anthropicProvider.test.ts # node tests
npm run compile                        # verify zero TypeScript errors
```

See AGENT.md section 10 for the full manual test checklist for each area (canvas, MCP, agents, providers).

### Pull Requests

- One logical change per PR — avoid bundling unrelated fixes
- Reference the issue your PR addresses (`Closes #123`)
- Describe what changed and why, not just what the diff shows
- Do not suppress TypeScript errors with `@ts-ignore` or `@ts-expect-error` unless the existing file already uses this pattern

## Security

If you discover a security vulnerability, please **do not** open a public issue. Email [security@forge-ide.com](mailto:security@forge-ide.com) with details. We will respond promptly.

## License

By contributing to Forge, you agree that your contributions will be licensed under the [MIT License](LICENSE.txt).
