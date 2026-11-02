import assert from "node:assert/strict";
export function assertPerformance(openings, scrolls) {
  assert.equal(openings.length, 5, "Exactly five openings are required");
  assert(
    openings.every((n) => Number.isFinite(n) && n >= 0),
    "Opening samples must be finite",
  );
  const median = [...openings].sort((a, b) => a - b)[2];
  assert(median < 400, `First-file median ${median} ms must be <400 ms`);
  assert(scrolls.length > 0, "Scroll measurements are required");
  for (const scroll of scrolls) {
    assert(
      scroll.frames.length >= 2 && scroll.frames.every((n) => Number.isFinite(n) && n > 0),
      "Missing or invalid rAF frames",
    );
    const max = Math.max(...scroll.frames);
    assert(max <= 32, `Maximum frame ${max} ms must be <=32 ms`);
    assert(
      scroll.maxScroll > 0 && scroll.progress >= scroll.maxScroll - 1,
      "Incomplete scroll progress",
    );
    assert(scroll.finalVisible, "The final code line must be visible");
    assert(
      scroll.mountedLines > 0 && scroll.mountedLines < 200,
      "Code viewport must remain virtualized",
    );
  }
  return { median, maximumFrame: Math.max(...scrolls.flatMap((s) => s.frames)) };
}
export function assertReviewFlow(thread, round) {
  assert.equal(thread.status, "resolved", "Thread must be persisted resolved");
  assert(
    thread.comments.some(
      (c) => c.author === "agent" && c.body === "Moved to RetryConfig::max_attempts.",
    ),
    "Expected the fake agent MCP reply",
  );
  assert(round >= 2, "Request changes must advance the review round");
}
// Executed inside either real browser. It has no closure over the Node harness.
export async function openingProbe({ title, timeoutMs = 10000 }) {
  const button = [...document.querySelectorAll(".session-row")].find(
    (el) => el.querySelector("span")?.textContent === title,
  );
  if (!button) throw new Error(`Session not visible: ${title}`);
  const start = performance.now();
  button.click();
  return await new Promise((resolve, reject) => {
    let observed = false;
    function tick() {
      const line = [...document.querySelectorAll(".cm-line")].find((el) => {
        const rect = el.getBoundingClientRect();
        return (
          el.textContent?.trim() &&
          !el.classList.contains("diff-header") &&
          rect.width > 0 &&
          rect.height > 0 &&
          rect.top < innerHeight &&
          rect.bottom > 0
        );
      });
      if (line && observed) {
        resolve(performance.now() - start);
        return;
      }
      observed = !!line;
      if (performance.now() - start > timeoutMs) {
        reject(new Error("First code did not become visible"));
        return;
      }
      requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);
  });
}
export async function scrollingProbe({ lastLine, timeoutMs = 15000 }) {
  const scroller = document.querySelector(".cm-scroller");
  if (!scroller) throw new Error("Missing code scroller");
  scroller.scrollTop = 0;
  // Start at the settled top; every frame from the first scroll movement to its final paint is retained.
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const frames = [];
  let previous = performance.now(),
    start = previous,
    maximumMounted = 0,
    settled = 0;
  return await new Promise((resolve, reject) => {
    function tick(now) {
      frames.push(now - previous);
      previous = now;
      const maxScroll = scroller.scrollHeight - scroller.clientHeight;
      maximumMounted = Math.max(maximumMounted, document.querySelectorAll(".cm-line").length);
      const remaining = maxScroll - scroller.scrollTop;
      scroller.scrollTop = Math.min(
        maxScroll,
        scroller.scrollTop + Math.max(50, scroller.clientHeight / 2),
      );
      const last = [...document.querySelectorAll(".cm-line")].find((el) =>
        el.textContent?.includes(lastLine),
      );
      const rect = last?.getBoundingClientRect(),
        container = scroller.getBoundingClientRect();
      const finalVisible =
        !!rect && rect.bottom <= container.bottom + 1 && rect.top >= container.top - 1;
      if (remaining <= 1 && finalVisible) settled++;
      else settled = 0;
      if (settled >= 2) {
        resolve({
          frames,
          progress: scroller.scrollTop,
          maxScroll,
          finalVisible,
          mountedLines: maximumMounted,
        });
        return;
      }
      if (now - start > timeoutMs) {
        reject(new Error("Incomplete scroll: final line did not become visible"));
        return;
      }
      requestAnimationFrame(tick);
    }
    requestAnimationFrame(tick);
  });
}
