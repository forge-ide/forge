Start the Forge IDE in watch/development mode.

Choose based on what the user specifies:
- "client" → `npm run watch-client`
- "extensions" → `npm run watch-extensions`
- default (no target) → `npm run watch` (runs client transpile + client + extensions in parallel)

Note: these are long-running processes. Use run_in_background so the terminal stays free.
After starting, confirm the watch process is running and remind the user to launch the app with `./scripts/code.sh`.
