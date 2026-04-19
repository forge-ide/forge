# forge-fs

Scoped filesystem operations with path validation. Every Forge write or edit goes through this crate; the session dispatcher refuses direct writes so the rules here — glob-based `allowed_paths` enforcement, symlink rejection, byte-size limits, and unified diff/preview generation — apply uniformly. Reads are also validated and capped so an attacker-controlled path or runaway file size cannot allocate an unbounded buffer.

## Role in the workspace

- Depended on by: `forge-session` (tool dispatcher), and any future tool layers that need bounded FS access.
- Depends on: `glob`, `sha2`, `hex`, `similar` (diff), `tempfile`, `thiserror`. No Forge-internal deps.

## Key types / entry points

- `read_file(path, allowed_paths, limits) -> ReadResult` — canonicalises, glob-checks, size-checks, then reads. Returns content, byte count, and SHA-256.
- `ReadResult { content, bytes, sha256 }` — read response shape.
- `write(...)` and `write_preview(...)` (re-exported from `mutate`) — guarded write + dry-run preview.
- `edit(...)` and `edit_preview(...)` (re-exported from `mutate`) — guarded edit + dry-run preview using `similar` for diffs.
- `ApprovalPreview` (re-exported) — the unified-diff preview surfaced to approval prompts.
- `Limits` — configurable byte ceilings (e.g. `max_read_bytes`).
- `FsError` — typed error covering `Io`, `TooLarge`, `PathDenied`, `SymlinkDenied`, `ParentMissing`.
- `canonicalize_no_symlink` (crate-private) — symlink-safe canonicalisation shared by write and edit paths.

## Further reading

- [Crate architecture — `forge-fs`](../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
