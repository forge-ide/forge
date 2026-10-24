import assert from "node:assert/strict";
import { test } from "node:test";

import * as text from "../src/text.ts";

test("title capitalizes the first letter", () => {
  assert.equal(text.title("forge"), "Forge");
});

test("slugify lowercases and joins words with hyphens", () => {
  const slugify = (text as Record<string, unknown>).slugify;
  assert.equal(typeof slugify, "function", "src/text.ts exports no slugify");
  assert.equal((slugify as (s: string) => string)("Hello, World!"), "hello-world");
});
