import { expect, it } from "vitest";
import { visibleText } from "./text";
it("exposes controls and bidi without changing replacement characters or markup text", () => {
  expect(visibleText("a\u202eb\r\u001b\u0000\ufffd<img>")).toBe(
    "a⟦U+202E⟧b⟦U+000D⟧⟦U+001B⟧⟦U+0000⟧�<img>",
  );
  expect(visibleText("a\tb\nc")).toBe("a\tb\nc");
});
it("exposes all default-ignorable code points while preserving ordinary accents and replacement text", () => {
  expect(visibleText("isAdmin\u034f\ufe0f\u3164\u{e0100}\u2028\u2029")).toBe(
    "isAdmin⟦U+034F⟧⟦U+FE0F⟧⟦U+3164⟧⟦U+E0100⟧⟦U+2028⟧⟦U+2029⟧",
  );
  expect(visibleText("é e\u0301\t\n�")).toBe("é e\u0301\t\n�");
});
