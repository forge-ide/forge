Run all static validation checks for the Forge IDE project. These must all pass before submitting a PR.

Run the following checks in order and report the result of each:

1. **TypeScript compile** — `npm run compile` (zero new errors required)
2. **Layer checks** — `npm run valid-layers-check` (enforces the platform/workbench/common layer rules)
3. **Cyclic dependencies** — `npm run check-cyclic-dependencies`
4. **Hygiene / precommit** — `node --experimental-strip-types build/hygiene.ts`

Stop and report clearly if any check fails. Do not proceed to the next check after a failure unless the user asks.
