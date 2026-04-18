---
name: forge-milestone-backcompat-audit
description: Use when checking whether a Forge milestone broke any public-facing interface — diffs IPC message shapes (docs/architecture/ipc-contracts.md + the Rust/TS types that back it), config schema, CLI flags, and any exposed plugin/extension API from the milestone-start baseline to current, classifies each change as safe/breaking, and flags unannounced breaks. Produces one GitHub issue per breaking change plus a consolidated report. Trigger on phrases like "did we break compatibility in Phase N", "backcompat audit for the milestone", "check for breaking changes", "IPC contract diff for the milestone", or any pre-release compatibility pass.
---

# forge-milestone-backcompat-audit

## Overview

Milestone-scoped backwards-compatibility audit. Unlike the other milestone audits, scope is **genuinely diff-based** — the question "did we break anyone" is intrinsically about a delta, not a current state. The baseline is the milestone-start commit; the comparison point is current HEAD.

**Scope is restricted to public-facing interfaces only**:
- IPC message shapes defined on the crate side and consumed by the webview (`docs/architecture/ipc-contracts.md` is the authoritative catalog)
- Config schema — files under `crates/forge_core/src/config*` or wherever the config lives
- CLI flags — the `clap`-annotated struct (or equivalent) in the CLI entry point
- Any exposed plugin/extension API, if the project has one yet

Output is **one GitHub issue per breaking change** plus **one consolidated report**. Every breaking change must have either (a) a migration note in the changelog / release notes / an ADR, or (b) an explicit decision to document it now — this skill surfaces the ones with neither.

## Arguments

| Argument | Form | Meaning |
|----------|------|---------|
| Milestone (required) | `"Phase N: Title"` | The GitHub milestone title to audit |

If the argument is missing, list candidate milestones and ask:

```bash
gh api repos/forge-ide/forge/milestones --jq '.[] | {title, state, open_issues, closed_issues}'
```

## Steps

### 1. Locate interfaces and derive the baseline — invoke `Explore`

Delegate to an `Explore` subagent. Brief:

> For milestone `"<milestone>"`:
>
> 1. Fetch milestone metadata and merged PRs sorted ascending by merge date: `gh pr list --repo forge-ide/forge --search 'milestone:"<milestone>" is:merged sort:created-asc' --json number,mergeCommit,baseRefOid,mergedAt,files,title,body --limit 200`.
> 2. **Derive the baseline SHA**: the `baseRefOid` of the earliest merged PR. Return it.
> 3. Locate the interface surfaces. Forge's:
>    - **IPC**: read `docs/architecture/ipc-contracts.md` and follow it to the Rust type definitions that back each contract (typically under `crates/forge_*` as `#[derive(Serialize, Deserialize)]` types). List each contract surface as a (doc section, file path, type name) triple.
>    - **Config schema**: find the type(s) that represent user config — likely a `Config` struct under `crates/forge_core` or similar. Return file path and type name.
>    - **CLI flags**: find the `clap` `Parser` / `Args` types in the CLI crate. Return file path and type name.
>    - **Any exposed plugin/extension API**: if the repo has one yet, list the public traits/types; if not, say so.
> 4. For each interface surface, check whether it changed between baseline and HEAD: `git diff <baseline>..HEAD -- <file>` on each listed file. Return pass-through if no diff, or the full diff hunks if changed.
> 5. Look for PRs in the milestone whose title or body contains "breaking", "migration", or "backcompat" markers — these are self-declared breaking changes. Return their PR numbers and what each claims.
>
> Return: milestone facts, baseline SHA, interface-surface inventory, per-surface diff (or pass-through), self-declared breaking PR list.

### 2. Derive the change-class model — invoke `superpowers:brainstorming`

Not every diff is a breaking change. The load-bearing reason this is brainstormed and not checklisted: whether a change is breaking depends on what *consumers* depend on, and that depends on the interface. For serde-tagged enums, adding a variant can break consumers that match exhaustively; for plain structs with `#[serde(default)]`, adding a field is usually safe. Forge's IPC boundary is a JSON contract, so the rules are specific to JSON-shape compatibility.

Use brainstorming with the user, seeded by Phase 1 findings, to settle the classification rules for *this* audit:

- **Type-level changes**:
  - Struct field removed → breaking
  - Struct field renamed (serialized name changed) → breaking
  - Struct field type narrowed (e.g. `String` → specific enum) → breaking
  - Struct field type widened (e.g. integer → number) → usually safe if consumers are permissive
  - New required field → breaking
  - New optional field (`#[serde(default)]` or `Option<T>`) → safe
  - Enum variant removed → breaking
  - Enum variant renamed → breaking
  - New enum variant → breaking for exhaustive matchers, safe for permissive ones (IPC consumers in JS are typically permissive; Rust consumers typically aren't)
  - Tagged-enum tag renamed or discriminator shape changed → breaking
- **Config schema**: same rules as types, plus: default value change is **behavior-breaking** even if schema-compatible
- **CLI flags**: removed flag, renamed flag, semantically-changed flag → breaking; new flag with default → safe
- **Consumer asymmetry**: the IPC boundary has *two* consumers — Rust core and TS webview. A change is breaking if *either* side depends on the old shape. Be explicit about which side is affected.

Output: a concrete classification rule sheet for this milestone, plus a note for each interface surface saying which consumer directions matter.

**HARD GATE:** Do not begin Step 3 until the classification rules are presented and approved.

### 3. Classify each diff — invoke `superpowers:dispatching-parallel-agents`

For each interface surface that changed (from Phase 1), dispatch a subagent. Each one takes a single surface's diff and applies the Step 2 rules.

Brief each identically (substituting the surface):

> Classify the changes on this interface surface using the approved rule sheet.
>
> **Surface:** `<path>` — `<type name>`
> **Consumer directions that matter:** `<e.g. "Rust core → TS webview", "user config file → crates/forge_core">`
> **Rule sheet:** `<from Step 2, verbatim>`
> **Diff:**
> ```
> <full diff hunks for this surface>
> ```
> **Self-declared breaking PRs for this surface (if any):** `<from Phase 1>`
>
> For each atomic change in the diff (field added/removed, variant added/removed, type changed, etc.), return an object with these fields:
>
> - `change` — one-line description (e.g. "removed field `ToolCallEvent.request_id`")
> - `classification` — safe | breaking | behavior-breaking
> - `rule_applied` — which rule from the sheet justified the classification
> - `consumer_impact` — which side breaks: Rust-side consumers, TS-side consumers, or both; and a one-line "what breaks" explanation
> - `self_declared` — yes | no (whether a PR in the milestone already called this change out)
> - `migration_note_present` — yes | no | unknown (was a migration note written in changelog/release notes/ADR?)
> - `suggested_migration` — if breaking and no note exists: the note that should be written
>
> Rules:
> - When in doubt between safe and breaking, classify breaking — false positives are cheap; missed breaks are expensive
> - If the rule sheet doesn't cover a case, say so explicitly — do not invent a classification

Aggregate. Group by classification. The report pivots here: every `breaking` or `behavior-breaking` without a migration note is a finding.

### 4. Triage — invoke `superpowers:brainstorming`

Present the classified change list. Brainstorm:

- Any `breaking` classifications the user wants to reclassify (they know consumer realities the skill doesn't)
- Self-declared breaking changes: were the migration notes adequate, or do they need revision?
- Unannounced breaks: must every one get a new migration note, or is some scope OK to leave unannounced pre-1.0?
- Severity: default is `compat: high` for breaking, `compat: medium` for behavior-breaking, `compat: critical` for a silent break that would shatter live consumers (like existing IPC sessions)

**HARD GATE:** Do not create any GitHub issues until the triaged list is approved.

### 5. Create finding issues — sequential

One issue per **unannounced or inadequately-announced breaking change**. Self-declared breaks with adequate migration notes don't need a new issue — they get recorded in the report.

Find the next F-number, then:

```bash
gh issue create \
  --repo forge-ide/forge \
  --title "[F-NNN] <imperative title>" \
  --milestone "<milestone>" \
  --label "type: bug,compat: <severity>" \
  --body "$(cat <<'EOF'
## Scope

<One paragraph: which interface surface, which consumer direction breaks, why this was not caught in PR review.>

## Change

- **Interface:** `<path>` — `<type name>`
- **Classification:** breaking | behavior-breaking
- **Rule applied:** <from classification rule sheet>
- **Consumer impact:** <Rust | TS | both> — <what breaks>

### Diff

```diff
<relevant hunks>
```

### PR(s) responsible

- #<number>: <title>

## Migration note needed

<The migration note that should be written — where (changelog / release notes / ADR) and what it should say.>

## Remediation

One of:
- Write the migration note (and optionally revert-and-reland with a proper note)
- Revert the breaking change if it was accidental
- Document the break in the release notes as an intentional decision

## Definition of Done

- [ ] Migration note written at <target location>
- [ ] (If applicable) consumer side updated to handle the new shape
- [ ] Backcompat audit re-run and this change is `self_declared: yes` with `migration_note_present: yes`
EOF
)"
```

### 6. Create the consolidated report issue

```
Title:  [F-NNN] Backcompat audit report: <milestone>
Milestone: <milestone>
Labels: type: bug, compat: audit
```

Body template:

```markdown
## Summary

Milestone: <milestone>
Baseline SHA: <short-sha>
Interface surfaces inspected: <N>
Surfaces changed: <M>
Classified changes: safe <a> / breaking <b> / behavior-breaking <c>
Unannounced breaks (new issues filed): <k>

## Classification rules (from Step 2)

<Verbatim.>

## Interface surface inventory

| Surface | Type | Consumer directions | Changed? |
|---------|------|---------------------|----------|
| `docs/architecture/ipc-contracts.md#session-events` | `SessionEvent` | Rust → TS | yes |
| config schema | `Config` | user → Rust | no |
| CLI | `Args` | user → binary | yes |

## Classified changes

| ID | Interface | Change | Classification | Self-declared | Note present | Issue |
|----|-----------|--------|----------------|---------------|--------------|-------|
| C-01 | `SessionEvent` | removed `request_id` | breaking | no | no | #123 |
| C-02 | `Args` | new `--verbose` flag with default | safe | — | — | — |

## Self-declared breaking changes (no new issue needed)

| PR | Change | Migration note location |
|----|--------|-------------------------|
| #N | ... | CHANGELOG.md §<milestone> |

Raw diff output: `/tmp/forge-backcompat-audit-<milestone-slug>/diffs/`
```

### 7. Verify — invoke `superpowers:verification-before-completion`

```bash
gh issue list --repo forge-ide/forge \
  --milestone "<milestone>" \
  --search 'label:"compat: audit" OR label:"compat: critical" OR label:"compat: high" OR label:"compat: medium" OR label:"compat: low"' \
  --json number,title,labels
```

Confirm every triaged unannounced-break has an issue and the consolidated report exists with the full surface inventory. Do not claim done without this evidence.

## Delegation rules

| Work type | Where it runs |
|-----------|---------------|
| Interface inventory, baseline derivation, per-surface diffs, self-declared-PR sweep | `Explore` subagent |
| Classification rule derivation | `superpowers:brainstorming` with the user |
| Per-surface change classification | parallel subagents via `superpowers:dispatching-parallel-agents` |
| Triage of classifications and migration-note adequacy | `superpowers:brainstorming` with the user |
| Issue creation | Main context, strictly sequential |
| Final verification | `superpowers:verification-before-completion` |

## Common mistakes

| Mistake | Correct |
|---------|---------|
| Treating every diff as breaking | Most diffs are safe; classification requires rules, not alarm |
| Treating every diff as safe | When in doubt, classify breaking — false positives are cheap, missed breaks expensive |
| Forgetting consumer asymmetry at the IPC boundary | Every IPC change must be classified for *both* Rust and TS consumer directions |
| Collapsing schema-compatible default-value changes to "safe" | Default-value changes are behavior-breaking even when schema-compatible |
| Not checking for self-declared breaking PRs | A PR that already announced its break doesn't need a new issue — just record it in the report |
| Running this on a milestone with no interface changes | Exit early after Phase 1 if no surface actually changed — don't create an empty report |
| Confusing this with quality-review's "API consistency" | That is about internal shape; this is about whether external consumers break |
| Creating issues in parallel | Sequential only — F-number collisions |
| Claiming done without `gh issue list` evidence | `verification-before-completion` requires it |
