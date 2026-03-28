#!/usr/bin/env bash
set -euo pipefail

ARCH="x64"
FORMATS="tarball"
OUTPUT="./dist"
NO_CACHE=""
IMAGE_ONLY=false
MEMORY="16g"

usage() {
  cat <<EOF
Usage: $0 [OPTIONS]

Options:
  --arch ARCH       x64 (default) or arm64
  --formats FMTS    Comma-separated: tarball,rpm,deb,snap (default: tarball)
  --output DIR      Artifact destination (default: ./dist)
  --memory MEM      Container memory limit (default: 16g)
  --no-cache        Disable Podman layer cache
  --image-only      Build image only, don't run build
EOF
  exit 1
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --arch)    ARCH="$2";    shift 2 ;;
    --formats) FORMATS="$2"; shift 2 ;;
    --output)  OUTPUT="$2";  shift 2 ;;
    --memory)  MEMORY="$2";  shift 2 ;;
    --no-cache)   NO_CACHE="--no-cache"; shift ;;
    --image-only) IMAGE_ONLY=true;       shift ;;
    *) echo "Unknown option: $1" >&2; usage ;;
  esac
done

case "$ARCH" in
  x64)   PLATFORM="linux/amd64" ;;
  arm64) PLATFORM="linux/arm64" ;;
  *)     echo "Invalid arch: $ARCH (must be x64 or arm64)" >&2; exit 1 ;;
esac

NODE_VERSION=$(cat .nvmrc)
BUILD_COMMIT=$(git rev-parse HEAD)
IMAGE="forge-build-${ARCH}"
CONTAINER="forge-build-${ARCH}-$$"

echo "==> Building image: ${IMAGE} (${PLATFORM})"

# shellcheck disable=SC2086
podman build \
  --platform "${PLATFORM}" \
  --build-arg "VSCODE_ARCH=${ARCH}" \
  --build-arg "NODE_VERSION=${NODE_VERSION}" \
  --build-arg "BUILD_COMMIT=${BUILD_COMMIT}" \
  ${NO_CACHE} \
  -t "${IMAGE}" \
  -f build/container/Containerfile \
  .

if [[ "${IMAGE_ONLY}" == true ]]; then
  echo "==> Image built: ${IMAGE}"
  exit 0
fi

# Assemble the build command
BUILD_CMDS="npm run compile-build && \
  npm run compile-extensions-build && \
  npm run download-builtin-extensions && \
  npm run gulp core-ci && \
  npm run gulp vscode-linux-${ARCH}-min-ci"

# Append package commands for each requested format
IFS=',' read -ra FMT_LIST <<< "${FORMATS}"
for fmt in "${FMT_LIST[@]}"; do
  fmt="${fmt// /}"
  case "$fmt" in
    tarball)
      BUILD_CMDS="${BUILD_CMDS} && tar -czf .build/forge-linux-${ARCH}.tar.gz -C .. Forge-linux-${ARCH}"
      ;;
    rpm)
      BUILD_CMDS="${BUILD_CMDS} && npm run gulp vscode-linux-${ARCH}-prepare-rpm && npm run gulp vscode-linux-${ARCH}-build-rpm"
      ;;
    deb)
      BUILD_CMDS="${BUILD_CMDS} && npm run gulp vscode-linux-${ARCH}-prepare-deb && npm run gulp vscode-linux-${ARCH}-build-deb"
      ;;
    snap)
      BUILD_CMDS="${BUILD_CMDS} && npm run gulp vscode-linux-${ARCH}-prepare-snap && npm run gulp vscode-linux-${ARCH}-build-snap"
      ;;
    *)
      echo "Unknown format: $fmt (must be tarball, rpm, deb, or snap)" >&2
      exit 1
      ;;
  esac
done

echo "==> Running build in container: ${CONTAINER}"

podman run \
  --name "${CONTAINER}" \
  --platform "${PLATFORM}" \
  --memory "${MEMORY}" \
  -e "BUILD_SOURCEVERSION=${BUILD_COMMIT}" \
  -e "VSCODE_ARCH=${ARCH}" \
  "${IMAGE}" \
  bash -c "${BUILD_CMDS}"

# Extract artifacts from the stopped container
mkdir -p "${OUTPUT}"
echo "==> Extracting artifacts to ${OUTPUT}"

IFS=',' read -ra FMT_LIST <<< "${FORMATS}"
for fmt in "${FMT_LIST[@]}"; do
  fmt="${fmt// /}"
  case "$fmt" in
    tarball)
      podman cp "${CONTAINER}:/workspace/forge/.build/forge-linux-${ARCH}.tar.gz" "${OUTPUT}/"
      ;;
    rpm)
      mkdir -p "${OUTPUT}/rpm"
      podman cp "${CONTAINER}:/workspace/forge/.build/linux/rpm/." "${OUTPUT}/rpm/"
      ;;
    deb)
      mkdir -p "${OUTPUT}/deb"
      podman cp "${CONTAINER}:/workspace/forge/.build/linux/deb/." "${OUTPUT}/deb/"
      ;;
    snap)
      mkdir -p "${OUTPUT}/snap"
      podman cp "${CONTAINER}:/workspace/forge/.build/linux/snap/." "${OUTPUT}/snap/"
      ;;
  esac
done

podman rm "${CONTAINER}"
echo "==> Done. Artifacts in ${OUTPUT}"
