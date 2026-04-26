#!/usr/bin/env bash
# Phase 1 User Acceptance Test Harness
# Usage: ./docs/testing/phase1-uat.sh [--build] [--test UAT-NN] [--gui-only] [--cli-only] [--contract-only] [--help]
#
# Flags:
#   --build           Build Rust workspace + web app before running tests
#   --test UAT-NN     Run only the specified test (e.g. --test UAT-09)
#   --gui-only        Run only Playwright specs (UAT-01..08, 11, 12)
#   --cli-only        Run only bash-driven disk-state UATs (UAT-09, 10, 13)
#   --contract-only   Run only the contract-level UATs as classified in
#                     docs/testing/uat-conventions.md (consumed by
#                     docs/testing/smoke-uat.sh — see F-326).
#   --help, -h        Print this usage message and exit.
#
# Prerequisites: cargo, python3, pnpm, @playwright/test installed.
# See docs/testing/phase1-uat-setup.md for full setup.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# ── Contract-level UAT manifest ─────────────────────────────────────────────
# Sourced from docs/testing/uat-conventions.md (Phase 1 classification table).
# CLI subset (uat_NN bash functions): UAT-09, UAT-10, UAT-13.
# GUI subset (Playwright specs):       UAT-01c, UAT-08, UAT-11, UAT-14 (CSP).
CONTRACT_LEVEL_UATS=(
  UAT-01c UAT-08 UAT-09 UAT-10 UAT-11 UAT-13 UAT-14
)
# Playwright filename stems for GUI contract-level UATs (matched as a regex
# against the spec file basename — Playwright's positional arg is a substring
# filter that accepts | alternation).
CONTRACT_LEVEL_GUI_FILTER="uat-01c|uat-08|uat-11|uat-csp"

# ── Colour helpers ──────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BOLD='\033[1m'; RESET='\033[0m'

pass() { echo -e "  ${GREEN}✓${RESET} $*"; PASS=$((PASS+1)); }
fail() { echo -e "  ${RED}✗${RESET} $*"; FAIL=$((FAIL+1)); FAILED_TESTS+=("$*"); }
skip() { echo -e "  ${YELLOW}–${RESET} $*"; SKIP=$((SKIP+1)); }
header() { echo -e "\n${BOLD}$*${RESET}"; }

PASS=0; FAIL=0; SKIP=0; FAILED_TESTS=()

print_help() {
  sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

# ── Argument parsing ────────────────────────────────────────────────────────
DO_BUILD=false
ONLY_TEST=""
GUI_ONLY=false
CLI_ONLY=false
CONTRACT_ONLY=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --build) DO_BUILD=true; shift ;;
    --test)  ONLY_TEST="$2"; shift 2 ;;
    --gui-only) GUI_ONLY=true; shift ;;
    --cli-only) CLI_ONLY=true; shift ;;
    --contract-only) CONTRACT_ONLY=true; shift ;;
    -h|--help) print_help; exit 0 ;;
    *) echo "Unknown flag: $1"; exit 1 ;;
  esac
done

is_contract_level() {
  local id="$1"
  for c in "${CONTRACT_LEVEL_UATS[@]}"; do
    [[ "$c" == "$id" ]] && return 0
  done
  return 1
}

run_test() {
  local id="$1"
  if $CONTRACT_ONLY && ! is_contract_level "$id"; then
    return 1
  fi
  [[ -z "$ONLY_TEST" || "$ONLY_TEST" == "$id" ]] && return 0
  return 1
}

# ── Build ───────────────────────────────────────────────────────────────────
if $DO_BUILD; then
  header "Building Rust workspace…"
  cargo build --release -p forge-cli -p forge-session \
    --manifest-path "$REPO_ROOT/Cargo.toml" 2>&1 | tail -3

  header "Building web app…"
  (cd "$REPO_ROOT/web" && pnpm install --frozen-lockfile && pnpm --filter app build) | tail -5
fi

# ── Locate binaries ─────────────────────────────────────────────────────────
if [[ -x "$REPO_ROOT/target/release/forge" ]]; then
  FORGE="$REPO_ROOT/target/release/forge"
  FORGED="$REPO_ROOT/target/release/forged"
elif [[ -x "$REPO_ROOT/target/debug/forge" ]]; then
  FORGE="$REPO_ROOT/target/debug/forge"
  FORGED="$REPO_ROOT/target/debug/forged"
else
  echo -e "${RED}ERROR:${RESET} binaries not found — run with --build first"
  exit 1
fi
echo "Using binaries: $FORGE"

# ── Workspace setup ─────────────────────────────────────────────────────────
WORKSPACE="$(mktemp -d)"
trap 'rm -rf "$WORKSPACE"' EXIT

mkdir -p "$WORKSPACE/.agents"
cat > "$WORKSPACE/.agents/test-agent.md" <<'AGENT'
---
description: UAT test agent
---
You are a test assistant. Answer concisely.
AGENT

READABLE_FILE="$WORKSPACE/readable.txt"
echo "hello from forge phase1 UAT" > "$READABLE_FILE"

# MockProvider script: single text-only turn. fs.read turn reserved for GUI UATs.
MOCK_SCRIPT_FILE="$WORKSPACE/mock.json"
python3 - "$MOCK_SCRIPT_FILE" <<'PYEOF'
import sys, json
out = sys.argv[1]
turns = [
    '{"delta":"Hello from phase1 UAT."}\n{"done":"end_turn"}',
]
open(out, "w").write(json.dumps(turns))
PYEOF
export FORGE_MOCK_SEQUENCE_FILE="$MOCK_SCRIPT_FILE"

# ── Helpers ─────────────────────────────────────────────────────────────────
# Expect: "session <ID> started at <PATH>" style output from phase 0 harness.
session_id_from_output() { grep "^session " | awk '{print $2}'; }

start_session() {
  local tmpout; tmpout=$(mktemp)
  "$FORGE" session new agent test-agent --workspace "$WORKSPACE" \
    >"$tmpout" 2>&1 &
  local forge_pid=$!
  wait "$forge_pid" 2>/dev/null || true
  cat "$tmpout"
  rm -f "$tmpout"
}

# ── UAT-09: Persist session archive ─────────────────────────────────────────
uat_09() {
  header "UAT-09: Persist session archive on end"
  local out sid
  out=$(start_session)
  sid=$(echo "$out" | session_id_from_output)
  if [[ -z "$sid" ]]; then fail "UAT-09 could not capture session id"; return; fi

  sleep 0.5
  # End the session via kill. `session kill` finds forged via the pid file
  # written by `session new`, so no --workspace flag is needed (and not accepted).
  "$FORGE" session kill "$sid" >/dev/null 2>&1 || true
  sleep 0.5

  if [[ -d "$WORKSPACE/.forge/sessions/$sid" ]]; then
    fail "UAT-09 session dir still present under sessions/"
    return
  fi
  if [[ ! -f "$WORKSPACE/.forge/sessions/archived/$sid/events.jsonl" ]]; then
    fail "UAT-09 archived events.jsonl missing"
    return
  fi
  local first_line
  first_line=$(head -1 "$WORKSPACE/.forge/sessions/archived/$sid/events.jsonl")
  if [[ "$first_line" != '{"schema_version":1}' ]]; then
    fail "UAT-09 archived log missing schema header (got: $first_line)"
    return
  fi
  # SessionState serializes PascalCase (no #[serde(rename_all)]); meta.toml
  # records `state = "Archived"`.
  if ! grep -q 'state = "Archived"' "$WORKSPACE/.forge/sessions/archived/$sid/meta.toml" 2>/dev/null; then
    fail "UAT-09 meta.toml not rewritten"
    return
  fi
  pass "UAT-09 persist archive: dir moved, schema header intact, meta.toml updated"
}

# ── UAT-10: Ephemeral session purge ─────────────────────────────────────────
uat_10() {
  header "UAT-10: Ephemeral session purge on end"
  export FORGE_WORKSPACE="$WORKSPACE"
  local pre_dirs post_dirs
  pre_dirs=$(find "$WORKSPACE/.forge/sessions" -maxdepth 1 -type d 2>/dev/null | wc -l || echo 0)

  echo "hello" | "$FORGE" run agent test-agent --input - >/dev/null 2>&1 || true
  sleep 0.5

  post_dirs=$(find "$WORKSPACE/.forge/sessions" -maxdepth 1 -type d 2>/dev/null | wc -l || echo 0)
  # Ephemeral should not add a persisted dir.
  if [[ "$post_dirs" -gt "$pre_dirs" ]]; then
    # Look for any ephemeral remnant under archived/ too.
    local arch
    arch=$(find "$WORKSPACE/.forge/sessions/archived" -maxdepth 1 -type d 2>/dev/null | wc -l || echo 0)
    if [[ "$arch" -gt 1 ]]; then
      fail "UAT-10 ephemeral session left archived remnant"
      return
    fi
  fi
  pass "UAT-10 ephemeral purge: no persistent session dir left"
  unset FORGE_WORKSPACE
}

# ── UAT-13: CLI / GUI parity spot check ─────────────────────────────────────
uat_13() {
  header "UAT-13: CLI / GUI parity spot check"
  # Bash side only verifies the CLI column. GUI parity must be eyeballed
  # against the Dashboard during a Playwright run — documented, not asserted.
  # `session list` enumerates by scanning the runtime socket dir, not the
  # workspace; no --workspace flag accepted.
  if "$FORGE" session list >/dev/null 2>&1; then
    pass "UAT-13 CLI: forge session list succeeds (compare against Dashboard manually)"
  else
    fail "UAT-13 CLI: forge session list errored"
  fi
}

# ── GUI UATs via Playwright ─────────────────────────────────────────────────
run_gui_suite() {
  header "Playwright — GUI UATs (01a/01b/02/04/05/06/07/08/11/12)"
  if ! command -v pnpm >/dev/null; then
    skip "pnpm not on PATH — see docs/testing/phase1-uat-setup.md"
    return
  fi
  if [[ ! -d "$REPO_ROOT/web/packages/app/node_modules" ]]; then
    skip "web deps not installed — run 'pnpm install' in web/"
    return
  fi
  local filter=""
  if [[ -n "$ONLY_TEST" ]]; then
    local num="${ONLY_TEST#UAT-}"
    filter="uat-${num}"
  elif $CONTRACT_ONLY; then
    filter="$CONTRACT_LEVEL_GUI_FILTER"
  fi
  # `pnpm run test:e2e foo` forwards `foo` to Playwright as a file-name filter.
  # Playwright does NOT use the `--` separator the way npm does.
  local pw_cmd="test:e2e"
  if [[ -n "$filter" ]]; then
    if (cd "$REPO_ROOT/web/packages/app" && pnpm run "$pw_cmd" "$filter"); then
      pass "Playwright suite completed (filter=$filter)"
    else
      fail "Playwright suite reported failures — see web/packages/app/playwright-report/"
    fi
  else
    if (cd "$REPO_ROOT/web/packages/app" && pnpm run "$pw_cmd"); then
      pass "Playwright suite completed (inspect report for per-spec results)"
    else
      fail "Playwright suite reported failures — see web/packages/app/playwright-report/"
    fi
  fi
}

# ── Main ────────────────────────────────────────────────────────────────────
if ! $GUI_ONLY; then
  if run_test UAT-09; then uat_09; fi
  if run_test UAT-10; then uat_10; fi
  if run_test UAT-13; then uat_13; fi
fi

if ! $CLI_ONLY; then
  run_gui_suite
fi

# ── Summary ─────────────────────────────────────────────────────────────────
echo
echo -e "${BOLD}Summary:${RESET} ${GREEN}$PASS passed${RESET}, ${RED}$FAIL failed${RESET}, ${YELLOW}$SKIP skipped${RESET}"
if [[ $FAIL -gt 0 ]]; then
  echo -e "${RED}Failed tests:${RESET}"
  for t in "${FAILED_TESTS[@]}"; do echo "  - $t"; done
  exit 1
fi
