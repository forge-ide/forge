import { expect, it } from "vitest";
import { mountDiff, diffRows } from "./diff";
const hunks = [
  {
    id: "h1",
    oldStart: 3,
    oldLines: 2,
    newStart: 3,
    newLines: 2,
    lines: [" same", "-old", "+<img src=x onerror=alert(1)>\u202e"],
    turnId: "turn",
  },
];
it("maps deletions only to old lines and insertions only to head lines", () => {
  expect(diffRows(hunks).map((r) => [r.old, r.new])).toEqual([
    [undefined, undefined],
    [3, 3],
    [4, undefined],
    [undefined, 4],
  ]);
});
it("renders markup as text, exposes bidi, selects new lines and destroys its view", () => {
  const parent = document.createElement("div");
  document.body.append(parent);
  let range: unknown;
  let why = "";
  const diff = mountDiff(parent, hunks, {
    onSelect: (r) => (range = r),
    onRevert: () => {},
    onWhy: (turn) => (why = turn),
    onCopy: () => {},
  });
  expect(parent.querySelector("img")).toBeNull();
  expect(parent.textContent).toContain("⟦U+202E⟧");
  const line = parent.querySelector<HTMLButtonElement>('button[data-line="4"]');
  line?.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
  expect(range).toEqual({ startLine: 4, endLine: 4 });
  parent.querySelector<HTMLButtonElement>('button[aria-label="Show why"]')?.click();
  expect(why).toBe("turn");
  diff.destroy();
  expect(parent.children.length).toBe(0);
  parent.remove();
});
it("highlights the selected head range and copies the displayed header and visible identifiers", () => {
  const parent = document.createElement("div");
  document.body.append(parent);
  let copied = "";
  const diff = mountDiff(
    parent,
    [
      {
        ...hunks[0],
        id: "visible",
        oldStart: 1,
        oldLines: 0,
        newStart: 1,
        newLines: 1,
        lines: ["+const isAdmin\u034f = true;"],
      },
    ],
    { onSelect: () => {}, onRevert: () => {}, onWhy: () => {}, onCopy: (text) => (copied = text) },
  );
  parent
    .querySelector('button[data-line="1"]')
    ?.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
  expect(parent.querySelector(".diff-selected")).not.toBeNull();
  parent.querySelector<HTMLButtonElement>('button[aria-label="Copy"]')?.click();
  expect(copied).toBe("@@ -1,0 +1,1 @@\n+const isAdmin⟦U+034F⟧ = true;");
  diff.destroy();
  parent.remove();
});
it("keeps gutter targets mounted while a drag updates the selected range", () => {
  const parent = document.createElement("div");
  document.body.append(parent);
  const diff = mountDiff(parent, hunks, {
    onSelect: () => {},
    onRevert: () => {},
    onWhy: () => {},
    onCopy: () => {},
  });
  const line = parent.querySelector('button[data-line="4"]');
  line?.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true }));
  expect(parent.querySelector('button[data-line="4"]')).toBe(line);
  diff.destroy();
  parent.remove();
});
