#!/bin/bash
set -e
DIR=/tmp/forge-audit-phase-1/issues
REPO="forge-ide/forge"
MILESTONE="Phase 1: Single Provider + GUI"
OUT=/tmp/forge-audit-phase-1/created-issues.json

# Order: id, title, severity
ORDER=(
  "H3|forge-fs write_preview leaks arbitrary file contents into approval event|high"
  "H7|forge-session passes allowed_paths = [**] — fs tools reach any absolute path|high"
  "H8|forge-session UDS socket falls back to world-accessible /tmp path with shared UID|high"
  "H4|forge-providers NDJSON line buffer is unbounded — local squatter can OOM session|high"
  "H5|forge-providers reqwest client has no timeouts — slow-drip DoS|high"
  "H6|forge-session shell.exec timeout orphans sandboxed child; survives session shutdown|high"
  "H1|forge-cli session_kill passes pid<=0 to libc::kill, signaling process group|high"
  "H2|forge-cli session_kill has no PID ownership or staleness check|high"
  "H9|forge-shell CSP is null — no defense-in-depth against webview XSS|high"
  "H10|forge-shell Tauri commands have no per-session authorization — any window can approve any session|high"
  "H11|forge-shell session_hello accepts arbitrary filesystem socket_path|high"
  "M7|forge-session ignores client-supplied ApprovalScope; always records Once|medium"
  "M8|forge-session shell.exec cwd accepted verbatim and omitted from approval preview|medium"
  "M9|forge-session sandbox missing RLIMIT_NPROC/NOFILE/FSIZE — fork-bomb and disk-fill feasible|medium"
  "M6|forge-session UDS pre-bind remove_file → bind is a TOCTOU race|medium"
  "M1|forge-cli unvalidated session_id interpolated into pid/socket paths|medium"
  "M5|forge-providers OLLAMA_BASE_URL trusted without scheme/host validation|medium"
  "M4|forge-providers list_models() buffers entire response body with no size cap|medium"
  "M2|forge-core unbounded line reads in event_log::read_since and Transcript::from_file|medium"
  "M3|forge-fs no size limit on read_file/write/edit — memory DoS|medium"
  "M10|forge-shell session:event is broadcast app-wide — cross-session disclosure|medium"
  "M11|forge-shell capability glob session-* + unvalidated session-id in window label|medium"
  "M12|web/packages/app session:event payloads cast to string without runtime narrowing|medium"
  "L1|forge-core SessionMeta lacks deny_unknown_fields — forward-compat trust drift|low"
  "L2|forge-session shell.exec timeout_ms has no ceiling|low"
  "L3|forge-core ToolCallId is 64-bit — bump to 128 at next touch|low"
  "L4|forge-shell session_send_message has no text-size bound below 4 MiB wire cap|low"
  "L5|forge-shell session_approve_tool accepts scope: String without enum validation|low"
  "S|Phase 1 supply-chain hygiene — cargo scanners in CI, workspace licensing, unmaintained deps|low"
)

F=42
echo "[" > "$OUT"
FIRST=1

for entry in "${ORDER[@]}"; do
  IFS="|" read -r id title sev <<< "$entry"
  full_title="[F-$(printf '%03d' $F)] $title"
  labels="type: security,security: $sev"
  body_file="$DIR/$id.md"
  
  url=$(gh issue create \
    --repo "$REPO" \
    --title "$full_title" \
    --milestone "$MILESTONE" \
    --label "$labels" \
    --body-file "$body_file")
  
  number="${url##*/}"
  
  echo "  ✓ F-$(printf '%03d' $F) [$id] #$number — $title" | cut -c1-110
  
  if [ $FIRST -eq 0 ]; then echo "," >> "$OUT"; fi
  printf '  {"id":"%s","f_num":%d,"number":%s,"url":"%s","title":%s,"severity":"%s"}' \
    "$id" "$F" "$number" "$url" "$(echo "$full_title" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read().strip()))')" "$sev" >> "$OUT"
  FIRST=0
  
  F=$((F+1))
done

echo "" >> "$OUT"
echo "]" >> "$OUT"

echo ""
echo "✓ Created $((F-42)) issues, next F-number: F-$(printf '%03d' $F)"
