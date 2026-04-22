# Risks

> Extracted from IMPLEMENTATION.md §14 — five high-risk items, five medium-risk items, and four unresolved unknowns

---

## 14. Risks and unresolved questions

### High-risk

1. **Monaco in iframe IPC latency.** Editing performance depends on snappy postMessage round-trips. LSP now runs inside the iframe via `monaco-languageclient` which mitigates autocomplete lag. **Mitigation.** Prototype in phase 1; if > 16ms per keystroke for non-LSP interactions, evaluate moving Monaco out of the iframe.

2. **`ghostty-vt` crate stability.** The crate is young. **Mitigation.** Prototype in phase 2; if unstable blockers hit, we contribute upstream rather than swapping to `vt100`. Terminal pane is self-contained.

3. **agentskills.io pre-v1 spec.** The open standard targeting H2 2026 for v1.0. If the spec shifts between now and Forge's v1 ship, we adjust. **Mitigation.** Track the spec repo; our skill loader has a well-isolated parser module.

4. **Windows via WSL user experience.** Running a Linux Tauri binary inside WSL works but isn't as smooth as a native Windows build. **Mitigation.** Document clearly what works and what doesn't (e.g. clipboard, notifications). Set correct expectation that v1.3 delivers native Windows.

5. **Provider API drift.** Anthropic/OpenAI SSE formats have changed before. **Mitigation.** `translate.rs` layer isolates; versioned test fixtures catch regressions.

### Medium-risk

6. **Session-as-process memory overhead.** Each session is a full Rust process. For users with 10+ sessions, this matters (we soft-limit to 10 active by default). **Mitigation.** Profile in phase 2; a shared broker process is an option if single-session memory > 100 MiB at idle.

7. **Podman/docker availability on macOS.** podman requires VM on macOS; users without Docker Desktop or podman machine won't have Level 2 out of box. **Mitigation.** Dashboard onboarding detects and guides; Level 2 remains opt-in.

8. **Approval UX for batch tool calls.** Agents often chain 5–20 tool calls. Approving each individually is painful. **Mitigation.** The four-scope approval (once/file/pattern/tool) makes this tractable. Pattern-based whitelisting specifically targets the common case "approve all writes under `./src/`."

9. **Cross-workspace usage aggregation.** Scanning N workspaces' jsonl logs for "last 30 days" is cheap until volume grows. **Mitigation.** Deferred decision; phase 3 picks a strategy based on real usage telemetry.

10. **LSP in webview is not Rust-sandboxed.** Language servers we auto-install run in the webview process, not routed through our Rust host. This trades sandboxing purity for editor responsiveness. **Mitigation.** Document the tradeoff; LSP servers are well-known trusted binaries (rust-analyzer, gopls, etc.), not arbitrary user code.

### Unresolved (tracked as issues)

- **Q1.** Cross-workspace usage aggregation strategy (phase 3 decision)
- **Q2.** Auto-compact thresholds (90%/98% are guesses, tune with telemetry)
- **Q3.** Memory storage format — plain markdown is simple but lacks structure. Consider `memory/<agent>.yaml` later if agents need tagged/dated entries.
