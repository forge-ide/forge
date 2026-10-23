import { expect, it } from "vitest";
import { EDITOR_PACKAGE } from "./index";

it("exports the package name", () => {
  expect(EDITOR_PACKAGE).toBe("@forge/editor");
});
