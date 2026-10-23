import { expect, it } from "vitest";
import { CLIENT_PACKAGE } from "./index";

it("exports the package name", () => {
  expect(CLIENT_PACKAGE).toBe("@forge/client");
});
