# Sample repos

One small project per v0.1 language, for the manual smoke checklist (`docs/smoke.md`). Each has
one failing test: it calls `slugify`, which doesn't exist yet. The smoke task asks an agent to
add the function and a unit test of its own, so the failing test passes.

`just smoke-repo <rust|typescript|python|go> <dir>` copies one into `<dir>` as a fresh git repo
on `main` with one commit, ready for `forge project open`.

| Repo | Test command |
| --- | --- |
| `rust` | `cargo test` |
| `typescript` | `node --test` (Node 24 runs the `.ts` files directly) |
| `python` | `python3 -m unittest` |
| `go` | `go test ./...` |
