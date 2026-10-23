import { describe, expect, it } from "vitest";
import { appTitle } from "./title";

describe("appTitle", () => {
  it("names the app", () => {
    expect(appTitle()).toBe("Forge");
  });
});
