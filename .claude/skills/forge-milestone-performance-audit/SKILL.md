---
name: forge-milestone-performance-audit
description: Use when auditing performance regressions accumulated across a Forge milestone — compares current state against the milestone-start baseline on startup latency, IPC roundtrip time, binary size, bundle size, and memory/allocation hot paths, and scans touched code for perf anti-patterns (sync I/O in async, unnecessary clones, O(n²) loops, heavy imports). Produces one GitHub issue per finding plus a consolidated report. Trigger on phrases like "perf audit for Phase N", "did we regress on performance this milestone", "check the milestone for perf drift", "startup time sweep", or any milestone-level performance pass before a release cut.
---

# forge-milestone-performance-audit

## Overview

Milestone-scoped performance audit. The distinct value vs. any single PR's perf review is **cumulative regression detection** — each PR may add 2ms of startup, 50KB of bundle, or one extra allocation, but ten of them together become a user-visible regression no individual review caught.

This skill combines **two signal sources**:
- **Empirical measurement** — benchmarks and binary/bundle sizes compared against a *milestone-start baseline*
- **Static perf review** — parallel subagents reading touched code for perf anti-patterns

Output is **one GitHub issue per finding** plus **one consolidated report issue**.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone title to audit |

If the argument is missing, list candidate milestones and ask:

```bash
gh api repos/forge-ide/forge/milestones --jq '.[] | {title, state, open_issues, closed_issues}'
```

## Steps

### 1. Gather scope and derive the baseline — invoke `Explore`

Delegate to an `Explore` subagent. Brief:

> For milestone `"<milestone>"`:
>
> 1. Fetch milestone metadata: `gh api repos/forge-ide/forge/milestones --jq '.[] | select(.title == "<milestone>")'`.
> 2. List merged PRs sorted by merge date ascending: `gh pr list --repo forge-ide/forge --search 'milestone:"<milestone>" is:merged sort:created-asc' --json number,mergeCommit,baseRefOid,mergedAt,files --limit 200`.
> 3. **Derive the milestone-start baseline commit**: the `baseRefOid` of the *earliest* merged PR under the milestone. This is the point `main` was at when the milestone began. Return its SHA.
> 4. Reduce changed file paths to first-two-segment areas. Dedupe. This is the **touched-areas list**.
> 5. For each area, identify perf-relevant surface: hot paths (IPC dispatch, serialization, session boot), async entry points, any `criterion` bench modules (`benches/` directories), and any bundle-size-sensitive entry points (`web/packages/app/src/main.tsx`, etc.).
> 6. Check runnable tooling: `cargo bench --version` (cargo itself), availability of `criterion` in `Cargo.toml`, `pnpm --version`, `hyperfine --version`. Report what can run.
>
> Return: milestone facts, baseline SHA, touched-areas list with perf-relevant surface, tooling availability.

### 2. Derive the concern model — invoke `superpowers:brainstorming`

The load-bearing reason this is brainstormed and not checklisted: which perf concerns matter depends on what the milestone touched. A milestone that rewrote IPC framing concentrates on serialization throughput; a milestone that added UI concentrates on bundle size and render cost; a milestone of internal refactors may barely move any meaningful metric.

Seeded with Phase 1 findings, brainstorm with the user to produce a concern model covering whichever apply:

- **Startup latency** — cold start, first session ready
- **IPC roundtrip** — message dispatch latency, serialization cost, frame-size growth
- **Bundle size** — JS bundle byte count; tree-shaking regressions; new heavy deps
- **Binary size** — release binary growth
- **Allocation hot paths** — new `clone()`/`to_string()` in hot loops, `Arc`/`Box` where a reference would do
- **Concurrency / blocking** — sync I/O or CPU-heavy work on async runtimes; lock contention
- **Render cost** — unneeded re-renders, layout thrash, missing memoization (frontend)
- **Big-O regressions** — O(n²) introduced over a loop over collections
- **Cold cache behavior** — disk reads added to hot paths

Output: a bulleted concern model — concern, one-line rationale for this milestone, target metric where there is one (e.g. "startup ≤ X ms", "bundle ≤ Y KB").

**HARD GATE:** Do not begin Step 3 until the concern model is presented and approved.

### 3. Measure baseline vs. current — Bash

Check out the baseline, measure; check out current, measure; diff.

```bash
BASELINE=<sha-from-step-1>
OUT=/tmp/forge-perf-audit-<milestone-slug>
mkdir -p "$OUT/baseline" "$OUT/current"

# Save current HEAD to return to
CURRENT=$(git rev-parse HEAD)

# --- baseline ---
git checkout "$BASELINE" --quiet
cargo build --release                                   > "$OUT/baseline/build.txt"    2>&1 || true
ls -la target/release/forge* 2>/dev/null                > "$OUT/baseline/binsize.txt"
( cd web && pnpm -r build )                             > "$OUT/baseline/webbuild.txt" 2>&1 || true
find web/packages/*/dist -type f -name '*.js' -exec du -b {} + 2>/dev/null \
                                                         > "$OUT/baseline/bundlesize.txt"
# Criterion benches if present — keep to small/fast ones; note which ran
cargo bench --all -- --quick 2>&1 | tee                 "$OUT/baseline/bench.txt" || true

# --- current ---
git checkout "$CURRENT" --quiet
cargo build --release                                   > "$OUT/current/build.txt"    2>&1 || true
ls -la target/release/forge* 2>/dev/null                > "$OUT/current/binsize.txt"
( cd web && pnpm -r build )                             > "$OUT/current/webbuild.txt" 2>&1 || true
find web/packages/*/dist -type f -name '*.js' -exec du -b {} + 2>/dev/null \
                                                         > "$OUT/current/bundlesize.txt"
cargo bench --all -- --quick 2>&1 | tee                 "$OUT/current/bench.txt" || true
```

Summarize the deltas: binary size (bytes + %), bundle size (bytes + % per JS file), bench results (if any, report per-bench delta). Do not dump raw output into the main context.

**If the baseline checkout fails** (e.g., build breakage at that SHA), note it and proceed with only the static review in Step 4 — a partial audit with clearly-labeled gaps beats no audit.

### 4. Per-area static perf review — invoke `superpowers:dispatching-parallel-agents`

One subagent per touched area. Brief each identically:

> Perf-review this code area at milestone scope.
>
> **Area:** `<path>`
> **Purpose:** `<from Step 1>`
> **Concern classes to weight, in order:** `<from Step 2>`
> **Baseline SHA:** `<from Step 1>` (so you can `git diff <baseline>..HEAD -- <area>` to focus on what changed)
>
> Your job: read the *current* state of the area and the *diff since baseline*. Flag findings with a plausible perf cost. Examples of what to look for, not a fixed list — use the concern model:
>
> - Sync I/O / blocking ops added to async paths
> - `clone()` / `to_string()` / `to_vec()` added to code on a hot path
> - New deps that widen binary or bundle measurably (check `Cargo.toml`, `package.json` for adds)
> - Loop nesting or inner loops over newly-larger collections
> - New re-renders (frontend): missing `useMemo`/`useCallback`/`React.memo` on objects that cross hot boundaries
> - Missing batching on IPC/event dispatch added during the milestone
>
> For each *finding*, return an object with these fields exactly:
>
> - `title` — imperative, ≤60 chars
> - `severity` — critical | high | medium | low (ground severity in user-visible impact, not theoretical cycles)
> - `location` — `path:line` (list all sites for cross-file patterns)
> - `concern_class` — from the Step 2 concern model
> - `cost_hypothesis` — *why* this likely costs — name the hot path it sits on and the operation's frequency; speculation is fine if labeled as such
> - `remediation` — concrete fix: remove which clone, move which op off the hot path, lazy-load which import, memoize which value
> - `bench_suggestion` — if applicable: "add a `criterion` bench at `<path>` measuring `<operation>`" to catch regression next time
>
> Rules:
> - Only report findings where a plausible cost story can be told. "This could be slow" with no hot-path claim is not a finding.
> - If the area is clean, return `[]` with a one-sentence rationale.

Aggregate, deduplicate.

### 5. Triage — invoke `superpowers:brainstorming`

Present the measurement deltas + static findings. Brainstorm:

- Which static findings are likely the *cause* of measured regressions? (Pairing measurements to causes is the interpretive step.)
- Which measurement regressions have no obvious static cause? (Those may need deeper investigation; treat as an open question for the user.)
- Findings to merge, split, or drop
- Severity adjustments
- Which findings warrant a **new criterion bench** or bundle-size budget (surface these — preventing regression is cheaper than catching it)

**HARD GATE:** Do not create any GitHub issues until the triaged list is approved.

### 6. Create finding issues — sequential

Find the next F-number, then:

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <imperative title>" \
  --milestone "<milestone>" \
  --label "type: bug,perf: <severity>" \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: which area, which concern class, how it connects (if it does) to a measured regression.>

## Finding

- **Location:** `<path:line>` (list all sites)
- **Severity:** <severity>
- **Concern class:** <class>

### Cost hypothesis

<Why this likely costs — name the hot path, the operation frequency, and the user-visible symptom.>

### Measured delta (if pairable)

- Metric: <startup | IPC | binary | bundle | bench name>
- Baseline: <value>
- Current: <value>
- Delta: <absolute + %>

## Remediation

<Concrete fix — remove which clone, lazy-load which import, etc.>

## Bench / budget suggestion

<If applicable: "add criterion bench at <path>" or "add bundle-size budget for <file>".>

## Definition of Done

- [ ] <Fix applied — name the symbol/file>
- [ ] If a bench/budget was suggested: added and failing without the fix
- [ ] `cargo build --release` size within baseline + <threshold>
- [ ] `pnpm -r build` bundle size within baseline + <threshold>
- [ ] Tests pass in CI
EOF
)"
```

### 7. Create the consolidated report issue

```
Title:  [F-NNN] Performance audit report: <milestone>
Milestone: <milestone>
Labels: type: bug, perf: audit
```

Body template:

```markdown
## Summary

Milestone: <milestone>
Baseline SHA: <short-sha>
Areas reviewed: <N>
Findings by severity: critical <a> / high <b> / medium <c> / low <d>

## Concern model

<Bulleted list from Step 2, verbatim.>

## Measured deltas

| Metric | Baseline | Current | Delta |
|--------|----------|---------|-------|
| Release binary | X MB | Y MB | +Z KB (+N%) |
| `forge-web` bundle | X KB | Y KB | +Z KB (+N%) |
| bench: `ipc_roundtrip` | X µs | Y µs | +Z µs (+N%) |

## Findings

| ID | Severity | Class | Title | Area | Paired metric | Issue |
|----|----------|-------|-------|------|---------------|-------|
| P-01 | high | bundle-size | ... | `web/packages/app` | bundle +Z KB | #123 |

## Suggested benches / budgets

- `criterion` bench at `crates/forge_ipc/benches/frame.rs` for `encode_frame`
- Bundle-size budget: `web/packages/app/dist/main.js` ≤ <budget>

Raw measurement output: `/tmp/forge-perf-audit-<milestone-slug>/`
```

### 8. Verify — invoke `superpowers:verification-before-completion`

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<milestone>" \
  --search 'label:"perf: audit" OR label:"perf: critical" OR label:"perf: high" OR label:"perf: medium" OR label:"perf: low"' \
  --json number,title,labels
```

Confirm every triaged finding has an issue, and the consolidated report exists. Do not claim done without this evidence.

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Milestone metadata, PR listing, baseline-SHA derivation, tooling detection | `Explore` subagent |
| Concern model derivation | `superpowers:brainstorming` with the user |
| Baseline-vs-current measurement | Bash (`git checkout`, `cargo build`, `pnpm build`, `cargo bench`); raw output on disk |
| Per-area static perf review | parallel subagents via `superpowers:dispatching-parallel-agents` |
| Pairing measurements to causes | `superpowers:brainstorming` with the user |
| Issue creation | Main context, strictly sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Skipping the baseline measurement | Perf audit without a baseline is vibes — the *delta* is the signal |
| Not recording the baseline SHA in the report | Future audits need a chain of baselines to catch slow drift |
| Static findings without a hot-path claim | "This allocates" is not a perf finding unless you name the path it sits on |
| Pairing every static finding to a measurement | Not every finding has a measured delta; say so explicitly rather than forcing a pairing |
| Chasing micro-optimizations on cold paths | Severity is grounded in user-visible impact, not theoretical cycles |
| Running full `cargo bench` on slow benches | Use `--quick` or filter; a perf audit that takes hours won't get run before releases |
| Ignoring bundle size | In a webview-based app, KB of JS is the user's latency story |
| Forgetting to `git checkout` back to current | Leaving the tree detached at baseline breaks the session — always return |
| Dumping raw bench output into the main context | Summarize the deltas; keep raw output on disk |
| Creating issues in parallel | Sequential only — F-number collisions |
| Claiming done without `gh issue list` evidence | `verification-before-completion` requires it |
