import { test } from "node:test";
import assert from "node:assert/strict";
import { assertPerformance, assertReviewFlow } from "./measure.mjs";
const scroll = {
  frames: [16, 32, 17],
  progress: 4000,
  maxScroll: 4000,
  finalVisible: true,
  mountedLines: 80,
};
test("accepts median below 400 and a frame exactly at 32", () => {
  assert.doesNotThrow(() => assertPerformance([390, 399, 380, 420, 375], [scroll]));
});
test("rejects median exactly 400, slow frames, incomplete or unrendered scrolls", () => {
  assert.throws(() => assertPerformance([400, 400, 400, 400, 400], [scroll]), /400/);
  assert.throws(
    () => assertPerformance([1, 2, 3, 4, 5], [{ ...scroll, frames: [16, 32.01] }]),
    /32/,
  );
  assert.throws(
    () => assertPerformance([1, 2, 3, 4, 5], [{ ...scroll, finalVisible: false }]),
    /final/i,
  );
  assert.throws(() => assertPerformance([1, 2, 3, 4, 5], [{ ...scroll, progress: 0 }]), /scroll/i);
  assert.throws(
    () => assertPerformance([1, 2, 3, 4, 5], [{ ...scroll, mountedLines: 5000 }]),
    /virtual/i,
  );
  assert.throws(() => assertPerformance([1, 2, 3], [scroll]), /five/i);
});
test("the flow cannot pass without an agent reply and resolved persisted thread", () => {
  const thread = {
    status: "resolved",
    comments: [
      { author: "human", body: "Read config" },
      { author: "agent", body: "Moved to RetryConfig::max_attempts." },
    ],
  };
  assert.doesNotThrow(() => assertReviewFlow(thread, 2));
  assert.throws(
    () => assertReviewFlow({ ...thread, comments: thread.comments.slice(0, 1) }, 2),
    /agent/i,
  );
  assert.throws(() => assertReviewFlow({ ...thread, status: "open" }, 2), /resolved/i);
  assert.throws(() => assertReviewFlow(thread, 1), /round/i);
});
