# Forge

A desktop development environment for directing and reviewing coding agents.

Agents (Claude Code, Codex, Gemini CLI, or anything that speaks ACP) do the writing inside sandboxed git worktrees. You review their work the way you'd review a teammate's pull request: comment on lines, request changes, and ship when it's right.

> **Status:** pre-alpha. The repo skeleton (M0) is in place; nothing works yet. See the plan for v0.1.

## What's here

- [`docs/spec/`](docs/spec/): the full spec
  - [`overview.md`](docs/spec/overview.md): goals, features, architecture, MVP scope
  - [`component-architecture.md`](docs/spec/component-architecture.md): how each crate works
  - [`build-plan.md`](docs/spec/build-plan.md): milestones M0–M8 with done-when checks
  - [`interfaces-and-data.md`](docs/spec/interfaces-and-data.md): API, events, config schemas, SQL schema
  - [`ui-screens.md`](docs/spec/ui-screens.md): wireframes, states, keybindings
  - [`quality-and-safety.md`](docs/spec/quality-and-safety.md): tests, performance, threat model, packaging
- [`AGENTS.md`](AGENTS.md): instructions for coding agents working on this repo
- [`docs/smoke.md`](docs/smoke.md): manual smoke checklist for real agents

## Stack

Rust daemon (`forged`) + Tauri 2 desktop app with SolidJS and CodeMirror 6. Linux and macOS in v0.1.

## Getting started

You need Rust (rustup picks up `rust-toolchain.toml`), Node 24 with `corepack enable` (for pnpm), `just`, and on Linux the WebKitGTK 4.1 dev packages and bubblewrap.

```sh
just dev   # run the app with hot reload; it starts forged if needed
just ci    # everything CI runs
```

Milestones are in [`build-plan.md`](docs/spec/build-plan.md).

## License

[Apache-2.0](LICENSE).
