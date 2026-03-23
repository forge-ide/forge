Run the appropriate test suite for the Forge IDE project.

Choose based on what the user specifies:
- A file path or module (e.g. "src/vs/platform/ai") → `npm run test-node` scoped to that path using mocha's `--grep` or by running `mocha` directly against the built test files under that path
- "browser" → `npm run test-browser-no-install`
- "extension" or "integration" → `npm run test-extension`
- "build-scripts" → `npm run test-build-scripts`
- "smoke" → `npm run smoketest-no-compile`
- default (no target) → `npm run test-node`

After running, summarize pass/fail counts and any failing test names.
