// UAT-01c — Real-Ollama chat round-trip.
// Plan: docs/testing/phase1-uat.md §UAT-01c
//
// Provider wiring landed in F-038. This spec stays skipped at the Playwright
// layer — the round-trip is exercised at the Rust integration level by
// `crates/forge-session/tests/provider_selection.rs::ollama_round_trip_against_local_qwen`
// (`#[ignore]`-gated). A real-shell Playwright variant requires the
// tauri-driver harness that other 01-series specs are also waiting on.

import { test } from '@playwright/test';

test.describe('UAT-01c — real-Ollama chat round-trip', () => {
  test.skip(
    true,
    'covered at the Rust integration layer; real-shell Playwright variant blocked on tauri-driver — see phase1-uat.md §UAT-01c',
  );

  test('placeholder — rerun UAT-01a against a real Ollama-backed session', async () => {
    // no-op
  });
});
