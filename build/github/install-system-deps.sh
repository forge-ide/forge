#!/usr/bin/env bash
set -e
MODE="${1:-ci}"
BASE_PKGS="pkg-config libgtk-3-0 libxkbfile-dev libkrb5-dev libgbm1"
PACKAGE_PKGS="rpm fakeroot dpkg snapcraft"

sudo apt-get update -qq
if [ "$MODE" = "package" ]; then
  sudo apt-get install -y --no-install-recommends $BASE_PKGS $PACKAGE_PKGS
else
  sudo apt-get install -y --no-install-recommends $BASE_PKGS
fi
