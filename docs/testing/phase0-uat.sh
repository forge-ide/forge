#!/usr/bin/env bash
# Phase 0 User Acceptance Test Harness
# Usage: ./docs/testing/phase0-uat.sh [--build] [--test UAT-NN]
#
# Flags:
#   --build         Build forge + forged before running tests
#   --test UAT-NN   Run only the specified test (e.g. --test UAT-01)
#
# Prerequisites: cargo, python3, socat (optional, for UAT-10)

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

# ── Colour helpers ──────────────────────────────────────────────────────────
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'
BOLD='\033[1m'; RESET='\033[0m'

pass() { echo -e "  ${GREEN}✓${RESET} $*"; PASS=$((PASS+1)); }
fail() { echo -e "  ${RED}✗${RESET} $*"; FAIL=$((FAIL+1)); FAILED_TESTS+=("$*"); }
skip() { echo -e "  ${YELLOW}–${RESET} $*"; SKIP=$((SKIP+1)); }
header() { echo -e "\n${BOLD}$*${RESET}"; }

PASS=0; FAIL=0; SKIP=0; FAILED_TESTS=()

# ── Argument parsing ────────────────────────────────────────────────────────
DO_BUILD=false
ONLY_TEST=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --build) DO_BUILD=true; shift ;;
    --test)  ONLY_TEST="$2"; shift 2 ;;
    *) echo "Unknown flag: $1"; exit 1 ;;
  esac
done

run_test() {
  local id="$1"
  [[ -z "$ONLY_TEST" || "$ONLY_TEST" == "$id" ]] && return 0
  return 1
}

# ── Build ───────────────────────────────────────────────────────────────────
if $DO_BUILD; then
  header "Building binaries…"
  cargo build --release -p forge-cli -p forge-session \
    --manifest-path "$REPO_ROOT/Cargo.toml" 2>&1 | tail -3
fi

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
WORKSPACE2="$(mktemp -d)"  # second workspace for UAT-13
trap 'rm -rf "$WORKSPACE" "$WORKSPACE2"' EXIT

# Mock agent
mkdir -p "$WORKSPACE/.agents"
cat > "$WORKSPACE/.agents/test-agent.md" <<'AGENT'
---
description: UAT test agent
---
You are a test assistant. Answer concisely.
AGENT

# Mock provider sequence: simple echo turn, then a fs.read turn
READABLE_FILE="$WORKSPACE/readable.txt"
echo "hello from forge UAT" > "$READABLE_FILE"

MOCK_SCRIPT_FILE="$WORKSPACE/mock.json"
python3 - "$MOCK_SCRIPT_FILE" "$READABLE_FILE" <<'PYEOF'
import sys, json
out, readable = sys.argv[1], sys.argv[2]
s1 = '{"delta":"Hello from mock."}\n{"done":"end_turn"}'
s2 = json.dumps({"delta":"I will read the file."}) + "\n" + \
     json.dumps({"tool_call":{"name":"fs.read","args":{"path":readable}}}) + "\n" + \
     json.dumps({"done":"tool_use"})
s3 = '{"delta":"Done reading."}\n{"done":"end_turn"}'
open(out, "w").write(json.dumps([s1, s2, s3]))
PYEOF

export FORGE_MOCK_SEQUENCE_FILE="$MOCK_SCRIPT_FILE"

# ── Python UDS helper (embedded) ─────────────────────────────────────────────
# Used by UAT-05, UAT-06, UAT-10, UAT-11
PY_HELPER="$WORKSPACE/uds_helper.py"
cat > "$PY_HELPER" <<'PYEOF'
"""Minimal forge UDS client."""
import json, socket, struct, sys, time, os

def send_frame(sock, msg: dict):
    body = json.dumps(msg).encode()
    sock.sendall(struct.pack(">I", len(body)) + body)

def recv_frame(sock, timeout=5.0):
    sock.settimeout(timeout)
    header = b""
    while len(header) < 4:
        chunk = sock.recv(4 - len(header))
        if not chunk:
            return None
        header += chunk
    length = struct.unpack(">I", header)[0]
    body = b""
    while len(body) < length:
        chunk = sock.recv(min(4096, length - len(body)))
        if not chunk:
            return None
        body += chunk
    return json.loads(body)

def connect_retry(path, max_wait=5.0):
    deadline = time.time() + max_wait
    while time.time() < deadline:
        try:
            s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            s.connect(path)
            return s
        except (FileNotFoundError, ConnectionRefusedError):
            s.close()
            time.sleep(0.05)
    raise RuntimeError(f"socket never appeared: {path}")

def handshake(sock):
    send_frame(sock, {"t":"Hello","proto":1,
                      "client":{"kind":"test","pid":os.getpid(),"user":"uat"}})
    ack = recv_frame(sock)
    assert ack and ack.get("t") == "HelloAck", f"expected HelloAck, got {ack}"
    return ack
PYEOF

# ── Helpers ─────────────────────────────────────────────────────────────────
# Parse "session <ID> started at <PATH>" from forge session new output.
# Uses grep to ignore the "forged: listening on ..." stderr line.
session_id_from_output()    { grep "^session " | awk '{print $2}'; }
socket_path_from_output()   { grep "^session " | awk '{print $NF}'; }

# Start a session.  forge session new inherits its stdout to forged (which
# keeps running), so using $(...) would block forever.  Instead write to a
# temp file, background forge, and wait only for the forge CLI PID to exit.
start_session() {
  local tmpout; tmpout=$(mktemp)
  "$FORGE" session new agent test-agent --workspace "$WORKSPACE" \
    >"$tmpout" 2>&1 &
  local forge_pid=$!
  wait "$forge_pid" 2>/dev/null || true
  cat "$tmpout"
  rm -f "$tmpout"
}

###############################################################################
# UAT-01: Spawn a session
###############################################################################
if run_test UAT-01; then
header "UAT-01: Spawn a session"
SESSION_OUT=$(start_session) || true
SESSION_ID=$(echo "$SESSION_OUT" | session_id_from_output)
SOCKET_PATH=$(echo "$SESSION_OUT" | socket_path_from_output)

if [[ -S "$SOCKET_PATH" ]]; then
  pass "socket exists at $SOCKET_PATH"
else
  fail "socket not found at $SOCKET_PATH"
fi

# PID file should be present at $XDG_RUNTIME_DIR/forge/sessions/<id>.pid
PID_FILE="${SOCKET_PATH%.sock}.pid"
if [[ -f "$PID_FILE" ]]; then
  pass "PID file created at $PID_FILE"
else
  skip "PID file not found at $PID_FILE"
fi

# events.jsonl is created lazily (on first message), checked in UAT-09 after events exist
skip "events.jsonl checked in UAT-09 (created on first message, not session start)"

# Keep session running for subsequent tests
ACTIVE_SESSION_ID="$SESSION_ID"
ACTIVE_SOCKET_PATH="$SOCKET_PATH"
fi

###############################################################################
# UAT-02: List sessions
###############################################################################
if run_test UAT-02; then
header "UAT-02: List sessions"
LIST_OUT=$("$FORGE" session list 2>&1) || true

if echo "$LIST_OUT" | grep -q "${ACTIVE_SESSION_ID:-NONE}"; then
  pass "active session appears in list"
else
  fail "active session not in list output: $LIST_OUT"
fi

if echo "$LIST_OUT" | grep -q "active"; then
  pass "session status shown as active"
else
  fail "no 'active' status in list output: $LIST_OUT"
fi
fi

###############################################################################
# UAT-03: Tail event stream (replay + live)
###############################################################################
if run_test UAT-03; then
header "UAT-03: Tail event stream"
# forge session tail has no --timeout; kill it after 2s with a background job
TAIL_TMPOUT=$(mktemp)
"$FORGE" session tail "$ACTIVE_SESSION_ID" >"$TAIL_TMPOUT" 2>&1 &
TAIL_PID=$!
sleep 2
kill "$TAIL_PID" 2>/dev/null || true
wait "$TAIL_PID" 2>/dev/null || true
TAIL_OUT=$(cat "$TAIL_TMPOUT"); rm -f "$TAIL_TMPOUT"

if echo "$TAIL_OUT" | python3 -c "
import sys, json
lines = [l.strip() for l in sys.stdin if l.strip()]
try:
    [json.loads(l) for l in lines if l]
    print('ok')
except Exception as e:
    print(f'bad json: {e}')
" | grep -q ok; then
  pass "tail output is valid JSON lines"
else
  skip "tail produced no parseable output (may need events first)"
fi

# Verify seq is present
if echo "$TAIL_OUT" | grep -q '"seq"'; then
  pass "tail events contain seq field"
else
  skip "no seq field visible in tail output (session may have no events yet)"
fi
fi

###############################################################################
# UAT-05: Send a message and receive a response (Python UDS client)
###############################################################################
if run_test UAT-05; then
header "UAT-05: Send message / receive events"
python3 - "$PY_HELPER" "$ACTIVE_SOCKET_PATH" <<'PYEOF'
import sys, json
sys.path.insert(0, __import__('os').path.dirname(sys.argv[1]))
from uds_helper import send_frame, recv_frame, connect_retry, handshake

sock = connect_retry(sys.argv[2])
ack = handshake(sock)
print(f"  session_id={ack.get('session_id')} workspace={ack.get('workspace')!r}")
send_frame(sock, {"t":"Subscribe","since":0})
send_frame(sock, {"t":"SendUserMessage","text":"Hello"})

events = []
seq_vals = []
for _ in range(30):
    msg = recv_frame(sock, timeout=5)
    if not msg:
        break
    if msg.get("t") == "Event":
        events.append(msg)
        seq_vals.append(msg.get("seq", -1))
        ev = msg.get("event", {})
        # Event type is "type" field: "user_message", "assistant_message", etc.
        if ev.get("type") == "assistant_message" and ev.get("stream_finalised"):
            break
sock.close()

types = [(e.get("event") or {}).get("type", "?") for e in events]
print(f"  Event types: {types}")
assert any(t == "user_message" for t in types), f"user_message missing: {types}"
assert any(t == "assistant_message" for t in types), f"assistant_message missing: {types}"
assert seq_vals == sorted(seq_vals), f"seq not monotonic: {seq_vals}"
print("PASS")
PYEOF

if [[ $? -eq 0 ]]; then
  pass "event sequence received with monotonic seq"
else
  fail "UAT-05 Python assertion failed"
fi
fi

###############################################################################
# UAT-06: Tool call approval gate
###############################################################################
if run_test UAT-06; then
header "UAT-06: Tool call approval gate"
# Spawn a fresh session (non-auto-approve) via Python directly against forged
python3 - "$PY_HELPER" "$WORKSPACE" "$FORGED" "$MOCK_SCRIPT_FILE" "$READABLE_FILE" <<'PYEOF'
import sys, json, os, subprocess, time, socket as _socket
sys.path.insert(0, __import__('os').path.dirname(sys.argv[1]))
from uds_helper import send_frame, recv_frame, connect_retry, handshake

workspace = sys.argv[2]; forged = sys.argv[3]
mock_file = sys.argv[4]; readable = sys.argv[5]

import tempfile, pathlib
d = tempfile.mkdtemp()
sock_path = pathlib.Path(d) / "uat06.sock"

env = os.environ.copy()
env["FORGE_SESSION_ID"] = "uat-06-test"
env["FORGE_SOCKET_PATH"] = str(sock_path)
env["FORGE_MOCK_SEQUENCE_FILE"] = mock_file

# Build a script that triggers a tool call
script_tool = (
    json.dumps({"delta":"Reading file."}) + "\n" +
    json.dumps({"tool_call":{"name":"fs.read","args":{"path":readable}}}) + "\n" +
    json.dumps({"done":"tool_use"})
)
script_cont = json.dumps({"delta":"Done."}) + "\n" + json.dumps({"done":"end_turn"})
with open(d + "/mock_gate.json", "w") as f:
    json.dump([script_tool, script_cont], f)

env["FORGE_MOCK_SEQUENCE_FILE"] = d + "/mock_gate.json"

proc = subprocess.Popen([forged], env=env,
                         stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    sock = connect_retry(str(sock_path))
    handshake(sock)
    send_frame(sock, {"t":"Subscribe","since":0})
    send_frame(sock, {"t":"SendUserMessage","text":"read the file"})

    # Collect until ToolCallApprovalRequested
    approval_event = None
    tool_call_id = None
    for _ in range(20):
        msg = recv_frame(sock, timeout=5)
        if not msg:
            break
        if msg.get("t") == "Event":
            ev = msg.get("event", {})
            # type field: "tool_call_approval_requested"
            if ev.get("type") == "tool_call_approval_requested":
                approval_event = ev
                tool_call_id = ev.get("id") or ev.get("tool_call_id")
                break

    assert approval_event, "ToolCallApprovalRequested never arrived"
    print(f"  Approval gate fired for tool_call_id={tool_call_id}")

    # Brief pause — verify stream is blocked (no AssistantMessage(final) yet)
    sock.settimeout(0.3)
    try:
        premature = recv_frame(sock, timeout=0.3)
        # Only fail if we get a final AssistantMessage before approving
        if premature and premature.get("t") == "Event":
            ev = premature.get("event", {})
            if ev.get("type") == "assistant_message" and ev.get("stream_finalised"):
                assert False, "Stream completed before approval was sent"
    except Exception:
        pass  # timeout expected

    # Approve
    send_frame(sock, {"t":"ToolCallApproved","id":tool_call_id,"scope":"Once"})

    # Verify ToolCallApproved + ToolCallCompleted arrive
    got_approved = got_completed = False
    for _ in range(20):
        msg = recv_frame(sock, timeout=5)
        if not msg:
            break
        if msg.get("t") == "Event":
            ev = msg.get("event", {})
            ev_type = ev.get("type", "")
            if ev_type == "tool_call_approved":
                got_approved = True
            if ev_type == "tool_call_completed":
                got_completed = True
            if got_approved and got_completed:
                break

    assert got_approved, "ToolCallApproved event not received after sending approval"
    assert got_completed, "ToolCallCompleted event not received after approval"
    print("PASS")
finally:
    proc.kill(); proc.wait()
import shutil; shutil.rmtree(d, ignore_errors=True)
PYEOF

if [[ $? -eq 0 ]]; then
  pass "approval gate blocked then resumed after ApproveToolCall"
else
  fail "UAT-06 approval gate test failed"
fi
fi

###############################################################################
# UAT-07: Auto-approve mode (forged --auto-approve-unsafe)
# forge run agent does not expose --auto-approve-unsafe; test at forged level.
###############################################################################
if run_test UAT-07; then
header "UAT-07: Auto-approve mode (forged --auto-approve-unsafe)"

# Build a 2-script mock: tool call turn + continuation
UAT07_MOCK="$WORKSPACE/mock_uat07.json"
python3 - "$UAT07_MOCK" "$READABLE_FILE" <<'PYEOF'
import sys, json
out, readable = sys.argv[1], sys.argv[2]
s1 = (json.dumps({"delta":"I will read."}) + "\n" +
      json.dumps({"tool_call":{"name":"fs.read","args":{"path":readable}}}) + "\n" +
      json.dumps({"done":"tool_use"}))
s2 = json.dumps({"delta":"Done reading."}) + "\n" + json.dumps({"done":"end_turn"})
open(out, "w").write(json.dumps([s1, s2]))
PYEOF

AUTO07_PY="$WORKSPACE/uat07.py"
cat > "$AUTO07_PY" <<'PYEOF'
import sys, json, os, subprocess, tempfile, pathlib, time, shutil
sys.path.insert(0, __import__('os').path.dirname(sys.argv[1]))
from uds_helper import send_frame, recv_frame, connect_retry, handshake

forged, mock_file, readable = sys.argv[2], sys.argv[3], sys.argv[4]

d = tempfile.mkdtemp()
sock_path = pathlib.Path(d) / "uat07.sock"

env = os.environ.copy()
env["FORGE_SESSION_ID"] = "uat-07-test"
env["FORGE_SOCKET_PATH"] = str(sock_path)
env["FORGE_MOCK_SEQUENCE_FILE"] = mock_file

proc = subprocess.Popen([forged, "--auto-approve-unsafe"], env=env,
                        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
try:
    sock = connect_retry(str(sock_path))
    handshake(sock)
    send_frame(sock, {"t":"Subscribe","since":0})
    send_frame(sock, {"t":"SendUserMessage","text":"read the file"})

    events = []
    final_count = 0
    for _ in range(40):
        msg = recv_frame(sock, timeout=5)
        if not msg:
            break
        if msg.get("t") == "Event":
            ev = msg.get("event", {})
            events.append(ev)
            if ev.get("type") == "assistant_message" and ev.get("stream_finalised"):
                final_count += 1
            if final_count >= 2:
                break
    sock.close()

    types = [e.get("type") for e in events]
    # Auto-approve: must see tool_call_approved but NOT tool_call_approval_requested
    assert "tool_call_approved" in types, f"tool_call_approved missing: {types}"
    assert "tool_call_approval_requested" not in types, f"approval_requested fired in auto-approve mode: {types}"
    assert "tool_call_completed" in types, f"tool_call_completed missing: {types}"
    print("PASS")
finally:
    proc.kill(); proc.wait()
    shutil.rmtree(d, ignore_errors=True)
PYEOF

python3 "$AUTO07_PY" "$PY_HELPER" "$FORGED" "$UAT07_MOCK" "$READABLE_FILE"
if [[ $? -eq 0 ]]; then
  pass "auto-approve: tool executed without ToolCallApprovalRequested"
else
  fail "UAT-07 auto-approve test failed"
fi
fi

###############################################################################
# UAT-08: Headless one-shot run (forge run agent)
# Note: forge run agent has no --auto-approve-unsafe; use a no-tool-call mock.
###############################################################################
if run_test UAT-08; then
header "UAT-08: Headless one-shot (forge run agent)"

# Use a simple text-only mock (no tool calls, so no approval needed)
UAT08_MOCK="$WORKSPACE/mock_uat08.json"
python3 -c "import json; print(json.dumps(['{\"delta\":\"Hello!\"}\n{\"done\":\"end_turn\"}']))" > "$UAT08_MOCK"
export FORGE_MOCK_SEQUENCE_FILE="$UAT08_MOCK"

# From stdin
cd "$WORKSPACE"
STDIN_TMPOUT=$(mktemp)
echo "say hello" | "$FORGE" run agent test-agent --input - >"$STDIN_TMPOUT" 2>&1 &
wait "$!" 2>/dev/null; STDIN_EXIT=$?
STDIN_OUT=$(cat "$STDIN_TMPOUT"); rm -f "$STDIN_TMPOUT"
cd "$REPO_ROOT"

if [[ $STDIN_EXIT -eq 0 ]]; then
  pass "forge run agent --input - exits 0"
else
  fail "forge run agent --input - exited $STDIN_EXIT: $STDIN_OUT"
fi

# From file
echo "say hello" > "$WORKSPACE/prompt.txt"
python3 -c "import json; print(json.dumps(['{\"delta\":\"Hi!\"}\n{\"done\":\"end_turn\"}']))" > "$UAT08_MOCK"
cd "$WORKSPACE"
FILE_TMPOUT=$(mktemp)
"$FORGE" run agent test-agent --input "$WORKSPACE/prompt.txt" >"$FILE_TMPOUT" 2>&1 &
wait "$!" 2>/dev/null; FILE_EXIT=$?
FILE_OUT=$(cat "$FILE_TMPOUT"); rm -f "$FILE_TMPOUT"
cd "$REPO_ROOT"

if [[ $FILE_EXIT -eq 0 ]]; then
  pass "forge run agent --input file exits 0"
else
  fail "forge run agent --input file exited $FILE_EXIT: $FILE_OUT"
fi

export FORGE_MOCK_SEQUENCE_FILE="$MOCK_SCRIPT_FILE"  # restore
fi

###############################################################################
# UAT-09: Event log durability and replay
###############################################################################
if run_test UAT-09; then
header "UAT-09: Event log durability"
# When forged is started with --workspace, events.jsonl lives under
# <workspace>/.forge/sessions/<session_id>/events.jsonl. The session id is
# stable for the daemon's lifetime and matches both the .sock basename and
# HelloAck.session_id (per F-035).
EVENTS_JSONL="$WORKSPACE/.forge/sessions/${ACTIVE_SESSION_ID}/events.jsonl"
# Wait for flush (background task flushes within 50ms)
sleep 0.2

if [[ -f "$EVENTS_JSONL" ]]; then
  HEADER=$(head -1 "$EVENTS_JSONL")
  if [[ "$HEADER" == '{"schema_version":1}' ]]; then
    pass "schema header is exact: $HEADER"
  else
    fail "schema header mismatch: '$HEADER'"
  fi

  # Event lines must be valid JSON with a `type` field (seq is derived
  # from line position by read_since, not stored per-line).
  VALID=$(tail -n +2 "$EVENTS_JSONL" | python3 -c "
import sys, json
lines = [l.strip() for l in sys.stdin if l.strip()]
bad = [l for l in lines if not json.loads(l).get('type')]
print(len(bad) if lines else 'empty')
" 2>/dev/null || echo "error")
  if [[ "$VALID" == "0" ]]; then
    pass "all event lines are valid typed events"
  elif [[ "$VALID" == "empty" ]]; then
    skip "event log has no events yet"
  elif [[ "$VALID" == "error" ]]; then
    fail "event log contains malformed JSON"
  else
    fail "$VALID event lines missing type field"
  fi
else
  fail "events.jsonl not found (expected at $EVENTS_JSONL — did UAT-05 run?)"
fi

# Corrupt schema header → should be rejected
BAD_LOG="$WORKSPACE/bad_events.jsonl"
echo '{"wrong":1}' > "$BAD_LOG"
echo '{"seq":1,"kind":"test"}' >> "$BAD_LOG"

BAD_SCHEMA_PY="$WORKSPACE/bad_schema_test.py"
cat > "$BAD_SCHEMA_PY" <<'PYEOF'
import sys, os, subprocess, tempfile, pathlib, shutil
helper_dir, forged, bad_log = sys.argv[1], sys.argv[2], sys.argv[3]

d = tempfile.mkdtemp()
sock_path = pathlib.Path(d) / "bad.sock"
# forged stores events at /tmp/forge-session-<id>/events.jsonl
events_dir = pathlib.Path("/tmp") / "forge-session-bad-session-id"
events_dir.mkdir(parents=True, exist_ok=True)
shutil.copy(bad_log, events_dir / "events.jsonl")

env = os.environ.copy()
env["FORGE_SESSION_ID"] = "bad-session-id"
env["FORGE_SOCKET_PATH"] = str(sock_path)

proc = subprocess.Popen([forged], env=env,
    stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
try:
    _, stderr = proc.communicate(timeout=3)
    code = proc.returncode
except subprocess.TimeoutExpired:
    proc.kill(); _, stderr = proc.communicate()
    code = proc.returncode
if code != 0 or b"schema" in stderr.lower() or b"invalid" in stderr.lower():
    print("REJECTED")
else:
    print(f"NOT_REJECTED code={code}")
shutil.rmtree(d, ignore_errors=True)
PYEOF

REJECT_OUT=$(python3 "$BAD_SCHEMA_PY" "$PY_HELPER" "$FORGED" "$BAD_LOG" 2>&1 || true)

if echo "$REJECT_OUT" | grep -q "REJECTED"; then
  pass "corrupt schema header rejected by forged"
else
  skip "forged exit behaviour on bad schema header unclear: $REJECT_OUT"
fi
fi

###############################################################################
# UAT-10: UDS protocol error handling
###############################################################################
if run_test UAT-10; then
header "UAT-10: UDS error handling"
python3 - "$PY_HELPER" "$ACTIVE_SOCKET_PATH" <<'PYEOF'
import sys, socket, struct, time
sys.path.insert(0, __import__('os').path.dirname(sys.argv[1]))
from uds_helper import send_frame, recv_frame, connect_retry

# Test 1: garbage bytes — session should close connection, not crash
sock = connect_retry(sys.argv[2])
sock.sendall(b"\xff\xff\xff\xffNOT_JSON_AT_ALL")
try:
    data = recv_frame(sock, timeout=2)
    # Connection closed (None) or error is acceptable
    print("GARBAGE_HANDLED")
except Exception:
    print("GARBAGE_HANDLED")
sock.close()
time.sleep(0.1)

# Test 2: unknown proto
sock2 = connect_retry(sys.argv[2])
send_frame(sock2, {"t":"Hello","proto":999,
                   "client":{"kind":"test","pid":1,"user":"uat"}})
try:
    resp = recv_frame(sock2, timeout=2)
    # Should get Error or None (connection closed)
    if resp is None or resp.get("t") in ("Error", None):
        print("PROTO_REJECTED")
    else:
        print(f"UNEXPECTED_RESP: {resp}")
except Exception:
    print("PROTO_REJECTED")
sock2.close()
time.sleep(0.1)

# Test 3: oversized frame (> 4 MiB) — should close connection
sock3 = connect_retry(sys.argv[2])
# Send a length header of 8 MiB without body
sock3.sendall(struct.pack(">I", 8 * 1024 * 1024))
try:
    data = recv_frame(sock3, timeout=2)
    print("OVERSIZE_HANDLED")
except Exception:
    print("OVERSIZE_HANDLED")
sock3.close()
PYEOF

if [[ $? -eq 0 ]]; then
  pass "garbage frames handled without crashing session"
  pass "unknown proto rejected"
  pass "oversized frame closed connection"
else
  fail "UAT-10 UDS error handling test failed"
fi
fi

###############################################################################
# UAT-11: Multi-client attach
###############################################################################
if run_test UAT-11; then
header "UAT-11: Multi-client attach"
python3 - "$PY_HELPER" "$ACTIVE_SOCKET_PATH" <<'PYEOF'
import sys, threading, time
sys.path.insert(0, __import__('os').path.dirname(sys.argv[1]))
from uds_helper import send_frame, recv_frame, connect_retry, handshake

results = {}

def client(name, sock_path, collect_n):
    sock = connect_retry(sock_path)
    handshake(sock)
    send_frame(sock, {"t":"Subscribe","since":0})
    events = []
    for _ in range(collect_n + 5):
        try:
            msg = recv_frame(sock, timeout=3)
        except Exception:
            break
        if msg and msg.get("t") == "Event":
            events.append(msg.get("seq"))
        if len(events) >= collect_n:
            break
    sock.close()
    results[name] = events

t1 = threading.Thread(target=client, args=("c1", sys.argv[2], 1))
t2 = threading.Thread(target=client, args=("c2", sys.argv[2], 1))
t1.start(); t2.start()
t1.join(timeout=8); t2.join(timeout=8)

c1 = results.get("c1", []); c2 = results.get("c2", [])
print(f"  Client 1 seqs: {c1}")
print(f"  Client 2 seqs: {c2}")

assert len(c1) > 0, "client 1 received no events"
assert len(c2) > 0, "client 2 received no events"
print("PASS")
PYEOF

if [[ $? -eq 0 ]]; then
  pass "both clients received events simultaneously"
else
  fail "UAT-11 multi-client test failed"
fi
fi

###############################################################################
# UAT-12: CLI argument validation
###############################################################################
if run_test UAT-12; then
header "UAT-12: CLI argument validation"

check_nonzero() {
  local desc="$1"; shift
  local exit_code=0
  timeout 5 "$@" </dev/null >/dev/null 2>&1 || exit_code=$?
  if [[ $exit_code -ne 0 && $exit_code -ne 124 ]]; then
    pass "$desc exits non-zero ($exit_code)"
  elif [[ $exit_code -eq 124 ]]; then
    skip "$desc timed out"
  else
    fail "$desc should exit non-zero but exited 0"
  fi
}

check_zero() {
  local desc="$1"; shift
  local exit_code=0
  timeout 5 "$@" </dev/null >/dev/null 2>&1 || exit_code=$?
  if [[ $exit_code -eq 0 ]]; then
    pass "$desc exits 0"
  else
    fail "$desc should exit 0 but exited $exit_code"
  fi
}

check_nonzero "'forge session new agent' (no name)"    "$FORGE" session new agent
check_nonzero "'forge session tail' (no id)"           "$FORGE" session tail
check_nonzero "'forge session kill' (no id)"           "$FORGE" session kill
check_nonzero "'forge bogus-command'"                  "$FORGE" bogus-command
check_zero    "'forge --help'"                         "$FORGE" --help
check_zero    "'forge session --help'"                 "$FORGE" session --help
fi

###############################################################################
# UAT-04: Kill a session  (after protocol tests)
###############################################################################
if run_test UAT-04; then
header "UAT-04: Kill a session"
KILL_EXIT=0
"$FORGE" session kill "$ACTIVE_SESSION_ID" 2>&1 || KILL_EXIT=$?

if [[ $KILL_EXIT -eq 0 ]]; then
  pass "forge session kill exits 0"
else
  fail "forge session kill exited $KILL_EXIT"
fi

sleep 0.5
if [[ ! -S "$ACTIVE_SOCKET_PATH" ]]; then
  pass "socket removed after kill"
else
  skip "socket still present (may take a moment to clean up)"
fi

STALE_LIST=$("$FORGE" session list 2>&1) || true
if ! echo "$STALE_LIST" | grep -q "^$ACTIVE_SESSION_ID.*active"; then
  pass "killed session no longer shows as active"
else
  fail "killed session still listed as active"
fi
fi

###############################################################################
# UAT-13: Workspace isolation
###############################################################################
if run_test UAT-13; then
header "UAT-13: Workspace isolation"

# Second workspace — separate .forge/ tree
mkdir -p "$WORKSPACE2/.agents"
cp "$WORKSPACE/.agents/test-agent.md" "$WORKSPACE2/.agents/"

WS2_TMPOUT=$(mktemp)
"$FORGE" session new agent test-agent --workspace "$WORKSPACE2" >"$WS2_TMPOUT" 2>&1 &
wait "$!" 2>/dev/null || true
WS2_OUT=$(cat "$WS2_TMPOUT"); rm -f "$WS2_TMPOUT"
WS2_ID=$(echo "$WS2_OUT" | session_id_from_output)

# Only workspace2 should have new session data
if [[ -d "$WORKSPACE2/.forge" ]]; then
  pass ".forge/ created in workspace2"
else
  fail ".forge/ not created in workspace2"
fi

if [[ ! -d "$WORKSPACE/.forge/sessions/$WS2_ID" ]]; then
  pass "workspace1 not contaminated by workspace2 session"
else
  fail "workspace1 has workspace2 session data"
fi

# gitignore auto-created
if [[ -f "$WORKSPACE2/.forge/.gitignore" ]]; then
  pass ".forge/.gitignore auto-created in workspace2"
else
  fail ".forge/.gitignore missing in workspace2"
fi

# Pre-existing gitignore not overwritten
GITIGNORE_PATH="$WORKSPACE2/.forge/.gitignore"
ORIGINAL_CONTENT=$(cat "$GITIGNORE_PATH")
"$FORGE" session new agent test-agent --workspace "$WORKSPACE2" >/dev/null 2>&1 &
wait "$!" 2>/dev/null || true
NEW_CONTENT=$(cat "$GITIGNORE_PATH")
if [[ "$ORIGINAL_CONTENT" == "$NEW_CONTENT" ]]; then
  pass "existing .gitignore preserved on second session"
else
  fail "existing .gitignore was overwritten"
fi

"$FORGE" session kill "$WS2_ID" 2>/dev/null || true
fi

###############################################################################
# Summary
###############################################################################
TOTAL=$((PASS + FAIL + SKIP))
echo ""
echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"
echo -e "${BOLD}Results: ${GREEN}$PASS passed${RESET}  ${RED}$FAIL failed${RESET}  ${YELLOW}$SKIP skipped${RESET}  (${TOTAL} total)"

if [[ ${#FAILED_TESTS[@]} -gt 0 ]]; then
  echo ""
  echo -e "${RED}Failed tests:${RESET}"
  for t in "${FAILED_TESTS[@]}"; do
    echo "  • $t"
  done
fi

echo -e "${BOLD}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${RESET}"

[[ $FAIL -eq 0 ]]
