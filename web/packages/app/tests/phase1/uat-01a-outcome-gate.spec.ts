// UAT-01a — Outcome gate (MockProvider).
// Plan: docs/testing/phase1-uat.md §UAT-01a
//
// This UAT exercises the full UI → session → tool call → approval → chat
// pipeline end-to-end against a real `forged` daemon driven by MockProvider.
// Running it against the Vite dev harness requires a companion bash fixture
// that spawns `forged` with FORGE_MOCK_SEQUENCE_FILE, captures the socket
// path, and proxies Tauri `invoke` calls to the real UDS. That fixture is
// not in this scaffold — it will be added in a follow-up.
//
// For now, this spec documents the step sequence and stays skipped.

import { test } from '@playwright/test';

test.describe('UAT-01a — outcome gate (MockProvider)', () => {
  test.skip(true, 'requires forged bridge fixture — see phase1-uat.md §UAT-01a');

  test('end-to-end: launch → chat → tool call → approve → result', async () => {
    // 1. Seed session via CLI with `FORGE_MOCK_SEQUENCE_FILE` pointing at a
    //    scripted three-turn conversation (text, fs.read tool call, continuation).
    // 2. Launch app, navigate to /, confirm the session card appears.
    // 3. Click the card; confirm Session window mounts.
    // 4. Send "read the file"; assert streaming cursor + composer disabled.
    // 5. Send "go ahead"; assert tool call card renders awaiting-approval.
    // 6. Focus card; press `A`; assert completed status + result preview.
    // 7. Assert composer re-enables on AssistantMessage(final).
  });
});
