import js from "@eslint/js";
import solid from "eslint-plugin-solid/configs/typescript";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "**/dist/",
      "**/node_modules/",
      "target/",
      // Session worktrees are copies of the repo with their own checks.
      ".worktrees/",
      "apps/desktop/src-tauri/",
      "packages/client/src/generated/",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.strict,
  {
    files: ["**/*.{ts,tsx}"],
    languageOptions: {
      globals: globals.browser,
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
    },
  },
  {
    files: ["packages/ui/**/*.{ts,tsx}"],
    ...solid,
  },
  {
    // Components go through packages/ui/src/host/, never Tauri directly.
    files: ["packages/ui/src/**/*.{ts,tsx}"],
    ignores: ["packages/ui/src/host/**"],
    rules: {
      "no-restricted-imports": ["error", { patterns: ["@tauri-apps/*"] }],
    },
  },
);
