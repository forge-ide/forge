// UAT-03 — Ollama status card reachable/unreachable/cache (F-023).
// Plan: docs/testing/phase1-uat.md §UAT-03
//
// The reachable/unreachable transitions require toggling the real Ollama
// daemon. The 10s cache assertion needs a counting HTTP shim on port 11434.
// Both require the real shell via tauri-driver. This spec documents the
// shape and skips until that harness lands.

import { test } from '@playwright/test';

test.describe('UAT-03 — Ollama status card variants', () => {
  test.skip(true, 'requires tauri-driver + Ollama toggle harness — see phase1-uat.md §UAT-03');

  test('reachable → unreachable → reachable transitions', async () => {
    // Steps 1-2, 5-6: see phase1-uat.md
  });

  test('10-second cache debounces two refreshes to one /api/tags call', async () => {
    // Step 4: point OLLAMA_BASE_URL at a counting shim; click Refresh twice; assert count === 1.
  });
});
