# Build Approach

> Extracted from IMPLEMENTATION.md §1-2 — five core commitments, what not to do first, and repository layout rationale

---

## 1. Approach and sequencing principles

Forge commits to **Rust + Tauri shell with Monaco in the webview** (CONCEPT.md §7). The implementation plan sequences accordingly.

### Five commitments that drive every decision

1. **Session-as-process first.** The very first milestone is a session process that runs to completion with no GUI. Every feature after that must preserve this — if the GUI can't be detached, we've broken a core commitment.
2. **IPC before UI.** All host↔session and Tauri↔webview boundaries are specified and typed before any UI code ships. Changing an IPC contract later is painful; changing UI is cheap.
3. **One provider, end-to-end, before breadth.** Anthropic first, through every layer (provider → session → chat pane → transcript → usage). No "OpenAI is mostly done, just need to wire it up" branches.
4. **Sandboxing as a feature, not a retrofit.** Level-0 (trusted) and Level-1 (process) are built in weeks 3–5. Level-2 (container) arrives in phase 2 but the API surface that makes it possible is present from day 1.
5. **CLI and GUI are siblings.** Every user-facing action goes through `forge-core` and is accessible both from the CLI and the GUI. No GUI-only features.

### What we do NOT do first

- Editor ergonomics beyond "Monaco loads, files open, edits save." Refactoring UI, multi-cursor, snippets — all post-v1.
- Extension/plugin system. Skills and MCP are our extensibility story for v1.
- Marketplace/registry. Git URLs and local paths only.
- Theming. One theme. Dark. Ember.

---

## 2. Repository layout

A cargo workspace at the root, with the webview frontend as a sibling `pnpm` project, plus docs.

```
forge/
├── Cargo.toml              # workspace root
├── Cargo.lock
├── rust-toolchain.toml     # pins stable (no nightly)
├── .editorconfig
├── AGENTS.md               # our own agent instructions (dogfood)
├── .mcp.json               # our own MCP registry (dogfood)
├── .agents/                # our own agent definitions (dogfood)
├── README.md
├── LICENSE
│
├── crates/                 # Rust workspace members
│   ├── forge-core/         # shared types, config, errors
│   ├── forge-providers/    # provider trait + built-in implementations
│   ├── forge-mcp/          # MCP client + server manager
│   ├── forge-agents/       # agent definitions, orchestration
│   ├── forge-session/      # session process binary (`forged`)
│   ├── forge-oci/          # container management
│   ├── forge-fs/           # scoped filesystem, diff/patch
│   ├── forge-lsp/          # LSP bootstrap (default server install/update)
│   ├── forge-term/         # ghostty-vt integration
│   ├── forge-ipc/          # UDS + JSON framing (shared types)
│   ├── forge-cli/          # `forge` binary (CLI entrypoint)
│   └── forge-shell/        # Tauri shell binary (GUI entrypoint)
│
├── web/                    # webview frontend (pnpm workspace)
│   ├── package.json
│   ├── pnpm-workspace.yaml
│   └── packages/
│       ├── app/            # SolidJS shell app (Tauri webview)
│       ├── monaco-host/    # isolated iframe hosting Monaco
│       ├── design/         # CSS tokens from DESIGN.md
│       └── ipc/            # generated TS types + typed IPC client
│
├── docs/
│   └── architecture/       # ADRs
│
├── scripts/                # dev tooling
│   ├── dev.sh              # concurrent run of forged + shell + vite
│   ├── gen-ts-types.sh     # ts-rs export
│   └── release/
│
└── tests/                  # integration tests (cross-crate)
    ├── headless_session/
    ├── tauri_ipc/
    └── cli_smoke/
```

### Why this shape
- Cargo workspace gives us one build graph, one lockfile, easy cross-crate refactoring.
- `forge-shell` (GUI) and `forge-cli` (CLI) are thin binaries on top of `forge-core` — no duplication, no "GUI-only logic."
- The webview sits in `web/` so frontend engineers don't need a Rust toolchain to iterate on UI (though the full dev loop does).
- Monaco lives in an **iframe** (`packages/monaco-host`) rather than directly in `packages/app` — this keeps Monaco's window globals, web workers, and language-service assumptions from leaking into our Solid tree.
