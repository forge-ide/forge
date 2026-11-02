import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { recordPerformance } from "./report.mjs";
test("retains raw samples and browser identity when exact performance limits fail", async () => {
  const root = await mkdtemp("/tmp/forge-report-");
  const result = {
    platform: "test platform",
    browserVersion: "test browser",
    openings: [400, 400, 400, 400, 400],
    scrolls: [
      { frames: [16, 33], progress: 100, maxScroll: 100, finalVisible: true, mountedLines: 50 },
    ],
  };
  try {
    await assert.rejects(recordPerformance(result, root), /400/);
    const saved = JSON.parse(await readFile(join(root, "measurements.json"), "utf8"));
    assert.deepEqual(saved.openings, result.openings);
    assert.deepEqual(saved.scrolls[0].frames, [16, 33]);
    assert.equal(saved.browserVersion, "test browser");
    assert.equal(saved.passed, false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
