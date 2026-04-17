# Scope

> Extracted from CONCEPT.md — v1.0 in/out of scope, deliberately deferred items, and open questions

---

## 12. Scope boundaries

### In v1.0
Everything in §§2–11. Including Windows (via WSL2, native build pending).

### Out of v1.0 — deliberately
- **Remote development** (SSH sessions, containers-as-dev-env) — v1.1 focus
- **Plugin marketplace** — skills/MCPs/agents install via filesystem, Git URLs, or local paths in v1
- **Debugger UI** — debug via terminal/LSP for v1
- **Real-time collaboration** — not scoped
- **OAuth provider login** — stdin credentials in v1, browser flow in v1.1
- **Bundled OCI runtime** — users install podman or docker themselves in v1
- **HTTP tarball skill distribution** — Git URLs and local paths only in v1; checksummed tarballs in v1.1
- **Native Windows build** — v1.3; WSL covers v1
- **Image attachments in composer** — v1.1
- **Native Rust editor** — a possibility if Monaco constraints bite, but not planned

---

## 13. Open questions

Most of what was flagged here in earlier drafts has been resolved. What remains:

1. **Auto-compact thresholds (90% / 98%).** Guesses. Real thresholds need telemetry to calibrate once users run long sessions.
2. **Cross-workspace usage aggregation strategy.** Deferred to phase 3 — pick between per-session scan, per-workspace aggregate, or monthly global aggregate when we have volume signal.
3. **Branched message tree UI details.** SPECS.md has the mini-spec; the rendering particulars (how to visualize branch points, how navigation works, how branches coexist in the event log without breaking replay) need implementation refinement.
4. **agentskills.io v1.0 spec stability.** The open standard is pre-v1 (v1.0 targeting H2 2026). If it shifts materially before Forge ships, we adjust. Flagged in IMPLEMENTATION.md risks.

Other former opens (terminal choice, OCI runtime, transcript versioning, snapshot cadence, prices data, skill distribution) are now resolved in this document or IMPLEMENTATION.md.
