---
name: forge-milestone-security-audit
description: Use when auditing a Forge milestone for security issues — derives a per-milestone threat model, audits the current state of every crate and area touched by the milestone's merged PRs, runs whichever automated scanners apply (cargo audit, cargo deny, pnpm audit), and produces one GitHub issue per finding plus a consolidated report issue. Trigger on phrases like "security audit for Phase N", "audit the milestone", "do a security pass on Phase N before release", "sweep the milestone for vulns", or any time the user wants milestone-scoped security review rather than PR-diff review.
---

# forge-milestone-security-audit

## Overview

Milestone-scoped security audit. Scope is **not a diff** — it is the full current state of every crate and top-level area (`crates/*`, `web/packages/*`, `scripts/*`, etc.) touched by any PR merged under the milestone, plus adjacent code owned by the same crates. Output is **one GitHub issue per finding** plus **one consolidated report issue**. The main context coordinates; scope gathering, per-area audits, and scanner runs are delegated.

This skill is complementary to `/security-review` — that one reviews pending branch changes, this one audits a completed milestone worth of landed work before a release cut.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone title to audit |

If the argument is missing, list candidate milestones and ask the user to pick:

```bash
gh api repos/forge-ide/forge/milestones --jq '.[] | {title, state, open_issues, closed_issues}'
```

## Steps

### 1. Gather scope — invoke `Explore`

Delegate to an `Explore` subagent. Brief:

> Read the forge repo at the current working directory. For milestone `"<milestone>"`:
>
> 1. Fetch milestone metadata: `gh api repos/forge-ide/forge/milestones --jq '.[] | select(.title == "<milestone>")'`. Return title, description, state.
> 2. List PRs merged under this milestone: `gh pr list --repo forge-ide/forge --search 'milestone:"<milestone>" is:merged' --json number,title,files,mergedAt --limit 200`.
> 3. Reduce the union of changed file paths to their first two path segments (e.g. `crates/forge_ipc`, `web/packages/session-ui`). Dedupe. This is the **touched-areas list**.
> 4. For each touched area, read `Cargo.toml` / `package.json` and the crate's main entry (`lib.rs`, `src/index.ts`) and return: package name, one-line purpose, public entry points, and any security-relevant modules you can spot by name (auth, ipc, exec, sandbox, fs, net, creds).
> 5. Check which automated scanners are available and what they cover here: run `cargo audit --version`, `cargo deny --version`, `pnpm --version`, and look for `deny.toml`, `pnpm-lock.yaml`, `Cargo.lock`. Report which scanners are runnable and over what scope.
>
> Return: milestone facts, ordered touched-areas list with per-area context, available-scanners list.

The main context takes this scope forward; do not re-explore inline.

### 2. Derive the threat model — invoke `superpowers:brainstorming`

Do not bring a fixed vulnerability checklist. The load-bearing reason is that a milestone's *new capabilities* drive which threat classes actually apply — a Phase 0 IPC handshake and a Phase 3 container runtime share almost no threat surface, and a generic checklist either over-audits (noise) or under-audits (misses the real risks).

Use brainstorming with the user, seeded by the Phase 1 findings, to produce a short threat model:

- What new capabilities did this milestone add? What trust boundaries do they cross?
- Where does untrusted input enter? (user content, LLM output used as control flow, file paths, env, network responses, IPC frames)
- What secrets or credentials does the affected code handle?
- New dependencies — any that deserialize, exec, network, or parse untrusted formats?
- Rust-specific concerns: `unsafe` blocks, panics on untrusted input, `serde` with untagged enums
- TS/webview-specific concerns: HTML-injection sinks (raw-HTML props, direct `innerHTML` writes), eval-like patterns, CSP gaps
- Tool-call / sandbox escape concerns if the milestone touched agent-exec code

Output: a bulleted **threat model** — threat class, one-line rationale for why it matters to *this* milestone, and expected severity ceiling.

**HARD GATE:** Do not begin Step 3 until the threat model has been presented to the user and approved. An ad-hoc threat model produces ad-hoc findings.

### 3. Run automated scanners — Bash, parallel

Cheap deterministic signal first. Run whatever Step 1 reported as available; skip the rest. Save raw output for the report.

```bash
OUT=/tmp/forge-audit-<milestone-slug>/scanners
mkdir -p "$OUT"

cargo audit --json                  > "$OUT/cargo-audit.json"     2>&1 || true
cargo deny check --format json      > "$OUT/cargo-deny.json"      2>&1 || true
( cd web && pnpm audit --json )     > "$OUT/pnpm-audit.json"      2>&1 || true
```

Summarize: count advisories per severity per scanner. Do not dump raw output into the main context — just the summary.

### 4. Per-area code audit — invoke `superpowers:dispatching-parallel-agents`

One subagent per touched area, all dispatched in the same turn. Brief each identically (substituting area-specific values):

> Security-audit this code area.
>
> **Area:** `<path>`
> **Purpose:** `<from Step 1>`
> **Threat classes to weight, in order:** `<from Step 2>`
>
> Read every file in the area. Read tests too — they document intended invariants. For each *finding*, return an object with these fields exactly:
>
> - `title` — imperative, ≤60 chars
> - `severity` — critical | high | medium | low
> - `location` — `path:line` (primary); add more in `description` if spread
> - `threat_class` — the class from the threat model this maps to
> - `description` — what is wrong and why it matters *in this codebase* (not generic CWE prose)
> - `reproduction` — concrete steps, a PoC sketch, or a pointer to the untrusted input path
> - `remediation` — concrete fix; name the symbol, file, or config to change
> - `references` — CWE, advisory, RFC, or doc URL if any
>
> Rules:
> - Only report findings you can defend against a skeptical reviewer. Do **not** pad with style or lint nits.
> - If the area is clean, return `[]` with a one-sentence rationale.
> - If a file is auto-generated or vendored, say so and skip it.

Aggregate all returns into one list. Deduplicate across agents — the same `path:line` may surface from neighboring areas.

### 5. Triage — invoke `superpowers:brainstorming`

Present the deduped finding list + scanner summary to the user. Brainstorm:

- Findings to merge, split, or drop (false positives)
- Severity adjustments — subagents lack cross-milestone context the user has
- Issue-creation ordering (typically: highest severity first, grouped by area)

**HARD GATE:** Do not create any GitHub issues until the triaged list is approved.

### 6. Create finding issues — sequential

Find the next available F-number (same pattern as `forge-create-task`):

```bash
gh issue list --repo forge-ide/forge --state all --json title \
  --jq '[.[].title | scan("F-([0-9]+)") | .[0] | tonumber] | max + 1'
```

Create one issue per finding, **sequentially** — parallel creation causes F-number collisions. Title format matches the repo convention (`[F-NNN] <imperative title>`); severity rides on labels.

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <imperative title>" \
  --milestone "<milestone>" \
  --label "type: security,security: <severity>" \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: which area, which threat class from the milestone's threat model, why it matters here specifically.>

## Finding

- **Location:** `<path:line>`
- **Severity:** <severity>
- **Threat class:** <from threat model>

<Description — what is wrong, grounded in this codebase.>

## Reproduction

<Steps, PoC sketch, or untrusted-input path.>

## Remediation

<Concrete fix — name the symbol, file, or config.>

## References

<CWE / advisory / doc URLs, or "none".>

## Definition of Done

- [ ] <Fix applied — name the symbol or file>
- [ ] Regression test added that fails without the fix
- [ ] Tests pass in CI
EOF
)"
```

### 7. Create the consolidated report issue

One final issue, sequential with the running next F-number. Internal finding IDs (`SEC-01`, `SEC-02`, …) exist only inside this report to keep the table readable — they are *not* a new numbering namespace in the repo.

```
Title:  [F-NNN] Security audit report: <milestone>
Milestone: <milestone>
Labels: type: security, security: audit
```

Body template:

```markdown
## Summary

Milestone: <milestone>
Areas audited: <N>
Findings by severity: critical <a> / high <b> / medium <c> / low <d>

## Threat model

<Bulleted list from Step 2, verbatim.>

## Scope

| Area | Purpose | Touched by PRs |
|------|---------|----------------|
| `crates/forge_ipc` | IPC framing | #41, #52, #78 |
| ... | ... | ... |

## Findings

| ID | Severity | Title | Area | Issue |
|----|----------|-------|------|-------|
| SEC-01 | high | ... | `crates/forge_ipc` | #123 |
| SEC-02 | medium | ... | `web/packages/session-ui` | #124 |

## Automated scanners

- cargo audit: <N> advisories (<summary>)
- cargo deny: <N> issues (<summary>)
- pnpm audit: <N> advisories (<summary>)

Raw output: `/tmp/forge-audit-<milestone-slug>/scanners/`
```

### 8. Verify — invoke `superpowers:verification-before-completion`

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<milestone>" \
  --label "type: security" \
  --json number,title,labels
```

Confirm:
- Every triaged finding has a corresponding open issue with the right severity label
- The consolidated report issue exists and every row in its table links to a real issue number
- No unlabeled or orphan security issues under the milestone

Do not claim the audit is complete until this evidence is in hand.

## Required labels

The skill relies on these labels existing on the repo. Create them once (e.g. via `gh label create`) before first run; `gh issue create` will fail loudly otherwise.

| Label | Purpose |
|-------|---------|
| `type: security` | Marks any security-audit output (both findings and the report) |
| `security: critical` / `high` / `medium` / `low` | Severity on finding issues |
| `security: audit` | Marks the consolidated report issue |

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Milestone metadata, PR listing, scope reduction, scanner detection | `Explore` subagent |
| Threat model derivation | `superpowers:brainstorming` with the user |
| Per-area code audit | parallel subagents via `superpowers:dispatching-parallel-agents` |
| Scanner invocations | Bash; results summarized, raw output saved to disk |
| Finding triage | `superpowers:brainstorming` with the user |
| Issue creation | Main context, strictly sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Auditing only the PR diffs | Scope is the **current state** of every touched area + adjacent code in the same crates |
| Using `/security-review` instead | That is diff-scoped; this skill is milestone-scoped — different surface, different output shape |
| Hard-coding a vulnerability checklist | Derive the threat model per-milestone — new capabilities drive threat classes |
| Skipping scanner runs | Cheap deterministic signal; always run whichever apply |
| Creating finding issues in parallel | Sequential only — F-number collisions otherwise |
| Padding findings with style/lint nits | Each finding must survive a skeptical reviewer |
| Vague remediation ("harden the parser") | Name the symbol, file, or config to change |
| No regression test in DoD | Every fix needs a test that fails without it — otherwise the class returns |
| Skipping the consolidated report | Individual issues lose the milestone-level picture a release reviewer needs |
| Dumping raw scanner output into the main context | Summarize by severity; keep raw output on disk and link it |
| Claiming done without `gh issue list` evidence | `verification-before-completion` requires evidence, not a summary |
