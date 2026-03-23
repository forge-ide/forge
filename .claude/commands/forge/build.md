Run the appropriate build command for the Forge IDE project.

If the user specifies a target, use the matching command:
- "web" → `npm run compile-web`
- "cli" → `npm run compile-cli`
- "check" or "typecheck" → `npm run compile-check-ts-native`
- default (no target) → `npm run compile`

After running, report whether the build succeeded or summarize any errors.
