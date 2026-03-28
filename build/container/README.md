# Container Build

Hermetic Linux builds via Podman. Produces tarball, `.deb`, `.rpm`, and `.snap` artifacts without requiring system packages, matching glibc, or npm dependencies on the host.

## Prerequisites

- Podman with rootless mode configured
- QEMU user-space emulation (`qemu-user-static`) if building arm64 on an x64 host

## Usage

```bash
# x64 tarball (default)
./build/container/build.sh

# arm64 tarball
./build/container/build.sh --arch arm64

# Multiple package formats
./build/container/build.sh --formats tarball,deb,rpm,snap

# Snap package only
./build/container/build.sh --formats snap

# Custom output directory
./build/container/build.sh --output ./release

# Build image only, skip the build run
./build/container/build.sh --image-only

# Force clean image rebuild
./build/container/build.sh --no-cache
```

Artifacts land in `./dist/` by default.

### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--arch` | `x64` | Target architecture: `x64` or `arm64` |
| `--formats` | `tarball` | Comma-separated: `tarball`, `deb`, `rpm`, `snap` |
| `--output` | `./dist` | Host directory to copy artifacts into |
| `--memory` | `16g` | Container memory limit |
| `--no-cache` | off | Disable Podman layer cache |
| `--image-only` | off | Build the image but don't run the build |

## How it works

1. **Image build** — Podman builds `forge-build-<arch>` from `Containerfile`. npm install layers are ordered before source COPY so they survive source-only changes.
2. **Build run** — Source is compiled and packaged inside the container. `BUILD_SOURCEVERSION` is injected so the VS Code build system doesn't need a `.git` directory.
3. **Artifact extraction** — `podman cp` pulls artifacts to the host; the container is removed.

arm64 builds on x64 use QEMU emulation via `--platform linux/arm64` — no sysroots needed, but expect 3–5× slower build times.

## Snap builds

Snap packages are built using `snapcraft` running in `--destructive-mode` inside the container. Destructive mode lets snapcraft build directly in the current environment rather than spinning up an inner Multipass or LXD VM — which is not possible inside a container. The Containerfile installs `snapcraft` and targets the `core22` base.

Snap artifacts land in `./dist/snap/` (a `.snap` file alongside build metadata).

> **Note:** Snap builds require more memory than other formats. The default `--memory 16g` limit covers typical builds; reduce only if your host is constrained and expect occasional OOM failures.

## Files

| File | Purpose |
|------|---------|
| `Containerfile` | Ubuntu 22.04 image with system deps, Node.js, snapcraft, and layered npm installs |
| `.containerignore` | Excludes `.git`, `node_modules`, and build outputs from the COPY context |
| `build.sh` | Entry point — builds image, runs build, extracts artifacts |
