// UAT-01c — Real-Ollama chat round-trip (BLOCKED).
// Plan: docs/testing/phase1-uat.md §UAT-01c
//
// BLOCKED: `crates/forge-session/src/main.rs:38-45` hardcodes MockProvider.
// Session-level OllamaProvider wiring is not yet in place. This spec stays
// skipped; open a follow-up ticket to select the provider in `forged` based
// on session meta.

import { test } from '@playwright/test';

test.describe('UAT-01c — real-Ollama chat round-trip', () => {
  test.skip(true, 'blocked — forged hardcodes MockProvider; see phase1-uat.md §UAT-01c');

  test('placeholder — rerun UAT-01a against a real Ollama-backed session', async () => {
    // no-op
  });
});
