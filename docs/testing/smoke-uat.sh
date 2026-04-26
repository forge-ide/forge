#!/usr/bin/env bash
# Persistent Smoke-UAT Suite (F-326)
# Usage: ./docs/testing/smoke-uat.sh [--build] [--phases 0,1,2] [--cli-only] [--gui-only] [--help]
#
# Aggregates the contract-level UATs from every existing phase harness into
# a single regression sweep that is safe to run on every release-cut and
# (eventually) every main-commit CI build. Each per-phase script remains the
# canonical home of its scenarios; this runner only invokes them with the
# `--contract-only` flag defined in docs/testing/uat-conventions.md.
#
# Flags:
#   --build         Forwarded to each phase script (builds Rust + web + mocks).
#   --phases LIST   Comma-separated subset of phases to run (default: 0,1,2).
#                   Example: --phases 0,2  skips Phase 1.
#   --cli-only      Forwarded to each phase script that supports it (1, 2).
#   --gui-only      Forwarded to each phase script that supports it (1, 2).
#                   Phase 0 is skipped under --gui-only since it has no GUI.
#   --help, -h      Print this usage message and exit.
#
# Exit code: non-zero if any phase script fails or if invalid flags are given.
#
# Notes:
#   --cli-only and --gui-only are mutually exclusive.
#   Skipped scenarios (missing prerequisites, etc.) do NOT fail the suite.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TESTING_DIR="$REPO_ROOT/docs/testing"

# ── Colour helpers ──────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BOLD='\033[1m'; CYAN='\033[0;36m'; RESET='\033[0m'

print_help() {
  sed -n '2,23p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

# ── Argument parsing ────────────────────────────────────────────────────────
DO_BUILD=false
PHASES_RAW="0,1,2"
CLI_ONLY=false
GUI_ONLY=false
while [[ $# -gt 0 ]]; do
  case "$1" in
    --build) DO_BUILD=true; shift ;;
    --phases) PHASES_RAW="$2"; shift 2 ;;
    --cli-only) CLI_ONLY=true; shift ;;
    --gui-only) GUI_ONLY=true; shift ;;
    -h|--help) print_help; exit 0 ;;
    *) echo "Unknown flag: $1" >&2; print_help; exit 2 ;;
  esac
done

if $CLI_ONLY && $GUI_ONLY; then
  echo "ERROR: --cli-only and --gui-only are mutually exclusive" >&2
  exit 2
fi

IFS=',' read -ra PHASES <<<"$PHASES_RAW"

# ── Aggregate counters ──────────────────────────────────────────────────────
TOTAL_PHASES=0
PASSED_PHASES=0
FAILED_PHASES=()

# Per-phase pass/fail/skip counts are parsed from each script's "Results:"
# (phase 0) or "Summary:" (phase 1/2) line.
TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_SKIP=0

# ── Per-phase invocation ────────────────────────────────────────────────────
run_phase() {
  local phase="$1"
  local script="$TESTING_DIR/phase${phase}-uat.sh"

  if [[ ! -x "$script" && ! -f "$script" ]]; then
    echo -e "${YELLOW}SKIP${RESET} phase ${phase}: $script not found"
    return 0
  fi

  # Phase 0 has no GUI — under --gui-only there is nothing to do.
  if $GUI_ONLY && [[ "$phase" == "0" ]]; then
    echo -e "${YELLOW}SKIP${RESET} phase 0 under --gui-only (no GUI surface)"
    return 0
  fi

  local args=(--contract-only)
  $DO_BUILD && args+=(--build)
  # Phase 0 has no --cli-only / --gui-only flags; only forward to 1+.
  if [[ "$phase" != "0" ]]; then
    $CLI_ONLY && args+=(--cli-only)
    $GUI_ONLY && args+=(--gui-only)
  fi

  echo
  echo -e "${BOLD}${CYAN}═══ Phase ${phase} smoke (contract-only) ═══${RESET}"
  echo -e "${CYAN}> bash $script ${args[*]}${RESET}"

  local log
  log="$(mktemp)"
  local rc=0
  bash "$script" "${args[@]}" 2>&1 | tee "$log"
  rc="${PIPESTATUS[0]}"

  TOTAL_PHASES=$((TOTAL_PHASES + 1))

  # Best-effort parse of the per-phase summary line.
  # Phase 0 prints:   "Results: N passed  M failed  K skipped  (T total)"
  # Phase 1/2 print: "Summary: N passed, M failed, K skipped"
  local summary
  summary="$(grep -E 'Results:|Summary:' "$log" | tail -1 || true)"
  if [[ -n "$summary" ]]; then
    local p f s
    p="$(echo "$summary" | grep -oE '[0-9]+ passed'   | grep -oE '[0-9]+' || echo 0)"
    f="$(echo "$summary" | grep -oE '[0-9]+ failed'   | grep -oE '[0-9]+' || echo 0)"
    s="$(echo "$summary" | grep -oE '[0-9]+ skipped'  | grep -oE '[0-9]+' || echo 0)"
    TOTAL_PASS=$((TOTAL_PASS + p))
    TOTAL_FAIL=$((TOTAL_FAIL + f))
    TOTAL_SKIP=$((TOTAL_SKIP + s))
  fi

  rm -f "$log"

  if [[ $rc -ne 0 ]]; then
    FAILED_PHASES+=("phase${phase}")
  else
    PASSED_PHASES=$((PASSED_PHASES + 1))
  fi
  return 0  # never abort — aggregate every phase before final exit
}

for ph in "${PHASES[@]}"; do
  case "$ph" in
    0|1|2) run_phase "$ph" ;;
    *) echo -e "${RED}ERROR${RESET}: unknown phase '$ph' — expected 0, 1, or 2" >&2; exit 2 ;;
  esac
done

# ── Aggregate summary ───────────────────────────────────────────────────────
echo
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${BOLD}Smoke-UAT aggregate${RESET}"
echo -e "  Phases run:   ${TOTAL_PHASES}"
echo -e "  Phases pass:  ${GREEN}${PASSED_PHASES}${RESET}"
echo -e "  Phases fail:  ${RED}${#FAILED_PHASES[@]}${RESET}"
echo -e "  Scenarios:    ${GREEN}${TOTAL_PASS} passed${RESET}  ${RED}${TOTAL_FAIL} failed${RESET}  ${YELLOW}${TOTAL_SKIP} skipped${RESET}"

if [[ ${#FAILED_PHASES[@]} -gt 0 ]]; then
  echo -e "${RED}Failed phases:${RESET}"
  for ph in "${FAILED_PHASES[@]}"; do
    echo "  - $ph"
  done
  echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
  exit 1
fi

echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
exit 0
