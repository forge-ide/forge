# forge-oci

Container lifecycle for agent isolation. Reserved scaffold in Phase 1 — the crate compiles into the workspace as a placeholder so the eventual `ContainerRuntime` trait and its `podman` / `docker` shells can land without a workspace-wide reshuffle. The Phase 1 binary does not run any containerised workloads; the architecture doc describes the planned API and runtime detection.

## Role in the workspace

- Depended on by: nothing yet; future agent isolation paths in `forge-agents` / `forge-session` will consume it.
- Depends on: nothing (intentionally empty until implementation begins).

## Key types / entry points

- _None yet._ The planned `ContainerRuntime` trait, `PodmanRuntime` / `DockerRuntime` implementations, OCI-spec generation via `oci-spec-rs`, and the first-run install banner are described in the architecture doc.

## Further reading

- [Crate architecture — `forge-oci`](../../docs/architecture/crate-architecture.md#36-forge-oci)
- [Isolation model](../../docs/architecture/isolation-model.md)
