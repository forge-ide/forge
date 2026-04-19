# @forge/ipc

The TypeScript IPC type surface. Re-exports the `ts-rs`-generated identifier and enum types from `forge-core` so the Solid app and any other web-side consumer speak the same wire format the Rust crates do — without hand-maintaining a parallel set of definitions. The generated files live in `src/generated/` and are produced by the Rust `ts-rs` derives during the workspace test run; this package only re-exports them and provides the public type entrypoint.

## Role in the workspace

- Depended on by: `app`. Future web packages that touch the IPC boundary should depend on this rather than re-deriving types.
- Depends on: nothing at runtime (types only). Dev: `typescript`.

## Key types / entry points

- `src/index.ts` — public re-exports: `AgentId`, `AgentInstanceId`, `ApprovalScope`, `CompactTrigger`, `MessageId`, `ProviderId`, `RosterScope`, `SessionId`, `SessionPersistence`, `SessionState`, `ToolCallId`, `WorkspaceId`.
- `src/generated/*.ts` — `ts-rs`-generated, auto-regenerated; do not edit by hand. Add new identifier or enum types by adding `#[derive(TS)]` in `forge-core` and regenerating.
- `package.json` `exports` — `.` (the curated re-exports) and `./generated/*` (raw access if a consumer needs an unexported type).
- Scripts: `typecheck` / `build` both run `tsc --noEmit`; the package has no compiled output.

## Further reading

- [IPC contracts](../../../docs/architecture/ipc-contracts.md)
- [Crate architecture — `forge-core` (id types)](../../../docs/architecture/crate-architecture.md#31-forge-core)
- [Crate architecture — `forge-ipc` (wire framing)](../../../docs/architecture/crate-architecture.md#37-forge-fs-forge-lsp-forge-term-forge-ipc-forge-cli-forge-shell)
