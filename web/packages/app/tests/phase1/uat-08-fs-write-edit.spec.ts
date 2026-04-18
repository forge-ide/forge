// UAT-08 — fs.write / fs.edit through the GUI (F-028 + F-029 via F-027).
// Plan: docs/testing/phase1-uat.md §UAT-08
//
// Needs a real `forged` daemon with `FORGE_MOCK_SEQUENCE_FILE` scripted to
// emit write/edit tool calls, plus a tempdir workspace so we can inspect
// disk state post-approval. That requires a forged bridge fixture which is
// not in this scaffold — a follow-up item.

import { test } from '@playwright/test';

test.describe('UAT-08 — fs.write / fs.edit through the GUI', () => {
  test.skip(true, 'requires forged bridge fixture — see phase1-uat.md §UAT-08');

  test('path denial: approval prompt renders, then invoke errors', async () => {
    // Step 1: approve once; assert FsError::PathDenied event; assert /etc/passwd unchanged.
  });

  test('fs.write inside workspace: file bytes match', async () => {
    // Step 2
  });

  test('fs.edit applies unified diff', async () => {
    // Step 3
  });

  test('malformed patch surfaces error event; file unchanged', async () => {
    // Step 4
  });

  test('nonexistent target: fs.edit refuses to create', async () => {
    // Step 5
  });
});
