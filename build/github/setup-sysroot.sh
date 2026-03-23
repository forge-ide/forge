#!/usr/bin/env bash
# Downloads glibc-2.28 sysroots for cross-compilation and writes compiler
# environment variables to $GITHUB_ENV for downstream GitHub Actions steps.
#
# Usage: bash build/github/setup-sysroot.sh <ARCH>
#   ARCH: x64 or arm64
#
# NOTE: The Chromium clang toolchain and libcxx headers/objects are NOT
# downloaded here. System GCC from the sysroot is used instead. Clang
# toolchain setup is deferred to a later production-release milestone.

set -e

ARCH="${1:-x64}"

if [ "$ARCH" != "x64" ] && [ "$ARCH" != "arm64" ]; then
  echo "Error: unsupported ARCH '$ARCH'. Expected x64 or arm64." >&2
  exit 1
fi

# Map x64 -> amd64 for the sysroot download helper (which uses Debian arch names)
if [ "$ARCH" = "x64" ]; then
  SYSROOT_ARCH="amd64"
else
  SYSROOT_ARCH="arm64"
fi

VSCODE_CLIENT_SYSROOT_DIR="$PWD/.build/sysroots/glibc-2.28-gcc-10.5.0"
VSCODE_REMOTE_SYSROOT_DIR="$PWD/.build/sysroots/glibc-2.28-gcc-8.5.0"

# Download client sysroot (glibc-2.28 / gcc-10.5.0) — used for native modules
if [ -d "$VSCODE_CLIENT_SYSROOT_DIR" ]; then
  echo "Using cached client sysroot: $VSCODE_CLIENT_SYSROOT_DIR"
else
  echo "Downloading client sysroot (glibc-2.28-gcc-10.5.0) for $ARCH ..."
  SYSROOT_ARCH="$SYSROOT_ARCH" \
  VSCODE_SYSROOT_DIR="$VSCODE_CLIENT_SYSROOT_DIR" \
    node -e 'import { getVSCodeSysroot } from "./build/linux/debian/install-sysroot.ts"; (async () => { await getVSCodeSysroot(process.env["SYSROOT_ARCH"]); })()'
fi

# Download remote sysroot (glibc-2.28 / gcc-8.5.0) — used for remote extension host modules
if [ -d "$VSCODE_REMOTE_SYSROOT_DIR" ]; then
  echo "Using cached remote sysroot: $VSCODE_REMOTE_SYSROOT_DIR"
else
  echo "Downloading remote sysroot (glibc-2.28-gcc-8.5.0) for $ARCH ..."
  SYSROOT_ARCH="$SYSROOT_ARCH" \
  VSCODE_SYSROOT_DIR="$VSCODE_REMOTE_SYSROOT_DIR" \
  VSCODE_SYSROOT_PREFIX="-glibc-2.28-gcc-8.5.0" \
    node -e 'import { getVSCodeSysroot } from "./build/linux/debian/install-sysroot.ts"; (async () => { await getVSCodeSysroot(process.env["SYSROOT_ARCH"]); })()'
fi

# Write compiler environment variables to $GITHUB_ENV so downstream steps
# inherit them automatically. System GCC from the sysroot is used (no clang).

if [ "$ARCH" = "x64" ]; then
  TRIPLE="x86_64-linux-gnu"
  CLIENT_SYSROOT="$VSCODE_CLIENT_SYSROOT_DIR/$TRIPLE/$TRIPLE/sysroot"
  REMOTE_SYSROOT="$VSCODE_REMOTE_SYSROOT_DIR/$TRIPLE/$TRIPLE/sysroot"

  # Client native module compiler (system GCC from client sysroot)
  CC_VAL="$VSCODE_CLIENT_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-gcc"
  CXX_VAL="$VSCODE_CLIENT_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-g++"
  CXXFLAGS_VAL="--sysroot=$CLIENT_SYSROOT"
  LDFLAGS_VAL="--sysroot=$CLIENT_SYSROOT -L$CLIENT_SYSROOT/usr/lib/$TRIPLE -L$CLIENT_SYSROOT/lib/$TRIPLE"

  # Remote extension host compiler (gcc-8.5.0 sysroot)
  REMOTE_CC_VAL="$VSCODE_REMOTE_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-gcc"
  REMOTE_CXX_VAL="$VSCODE_REMOTE_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-g++"
  REMOTE_CXXFLAGS_VAL="--sysroot=$REMOTE_SYSROOT"
  REMOTE_LDFLAGS_VAL="--sysroot=$REMOTE_SYSROOT -L$REMOTE_SYSROOT/usr/lib/$TRIPLE -L$REMOTE_SYSROOT/lib/$TRIPLE"

elif [ "$ARCH" = "arm64" ]; then
  TRIPLE="aarch64-linux-gnu"
  CLIENT_SYSROOT="$VSCODE_CLIENT_SYSROOT_DIR/$TRIPLE/$TRIPLE/sysroot"
  REMOTE_SYSROOT="$VSCODE_REMOTE_SYSROOT_DIR/$TRIPLE/$TRIPLE/sysroot"

  # Client native module compiler
  CC_VAL="$VSCODE_CLIENT_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-gcc"
  CXX_VAL="$VSCODE_CLIENT_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-g++"
  CXXFLAGS_VAL="--sysroot=$CLIENT_SYSROOT"
  LDFLAGS_VAL="--sysroot=$CLIENT_SYSROOT -L$CLIENT_SYSROOT/usr/lib/$TRIPLE -L$CLIENT_SYSROOT/lib/$TRIPLE"

  # Remote extension host compiler
  REMOTE_CC_VAL="$VSCODE_REMOTE_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-gcc"
  REMOTE_CXX_VAL="$VSCODE_REMOTE_SYSROOT_DIR/$TRIPLE/bin/$TRIPLE-g++"
  REMOTE_CXXFLAGS_VAL="--sysroot=$REMOTE_SYSROOT"
  REMOTE_LDFLAGS_VAL="--sysroot=$REMOTE_SYSROOT -L$REMOTE_SYSROOT/usr/lib/$TRIPLE -L$REMOTE_SYSROOT/lib/$TRIPLE"
fi

# Emit sysroot dir paths so downstream steps can reference them directly
# (e.g. the RPM packaging step needs VSCODE_CLIENT_SYSROOT_DIR for the strip binary)
cat >> "$GITHUB_ENV" <<EOF
VSCODE_CLIENT_SYSROOT_DIR=$VSCODE_CLIENT_SYSROOT_DIR
VSCODE_REMOTE_SYSROOT_DIR=$VSCODE_REMOTE_SYSROOT_DIR
CC=$CC_VAL
CXX=$CXX_VAL
CXXFLAGS=$CXXFLAGS_VAL
LDFLAGS=$LDFLAGS_VAL
VSCODE_REMOTE_CC=$REMOTE_CC_VAL
VSCODE_REMOTE_CXX=$REMOTE_CXX_VAL
VSCODE_REMOTE_CXXFLAGS=$REMOTE_CXXFLAGS_VAL
VSCODE_REMOTE_LDFLAGS=$REMOTE_LDFLAGS_VAL
EOF

echo "Compiler environment variables written to \$GITHUB_ENV."
