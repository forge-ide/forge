#!/usr/bin/env bash
set -e
MODE="${1:-ci}"
BASE_PKGS=(pkg-config libgtk-3-0 libxkbfile-dev libkrb5-dev libgbm1)
PACKAGE_PKGS=(rpm fakeroot dpkg patchelf execstack snapd squashfs-tools python3-dev python3-pip python3-toml python3-yaml)

sudo apt-get update -qq
if [ "$MODE" = "package" ]; then
  sudo apt-get install -y --no-install-recommends "${BASE_PKGS[@]}" "${PACKAGE_PKGS[@]}"

  # Install snapcraft 4.8.1 via pip.  The apt snapcraft package is a transitional stub
  # that requires the snap store; pip gives us the Python snapcraft that runs without
  # a running snapd daemon.  snapcraft's setup.py calls `git describe`, so we seed a
  # dummy repo and point GIT_DIR at it for the install.
  git init /tmp/pip-git
  git -C /tmp/pip-git config user.email "build@forge"
  git -C /tmp/pip-git config user.name "build"
  git -C /tmp/pip-git commit --allow-empty -m "init"
  git -C /tmp/pip-git tag -a 4.8.1 -m "4.8.1"
  sudo env GIT_DIR=/tmp/pip-git/.git pip3 install "snapcraft==4.8.1"
  rm -rf /tmp/pip-git

  # snapcraft==4.8.1 pins pyyaml==5.3; Ubuntu 22.04's python3-yaml (5.4.1, with C
  # bindings) satisfies the requirement.  Relax the pin then uninstall pip's copy so
  # Python falls back to the apt package (which has the CSafeDumper snapcraft needs).
  sudo python3 -c "
import glob, re
import sysconfig
site = sysconfig.get_path('purelib')
files = glob.glob(site + '/snapcraft-*.dist-info/METADATA')
[open(f,'w').write(re.sub(r'(?i)(pyyaml)==5\.3', r'\1>=5.3', open(f).read())) for f in files]
print('Patched', files)"
  sudo pip3 uninstall -y pyyaml
  python3 -c "from yaml import CSafeDumper; print('pyyaml C bindings OK')"

  # snapcraft data dir: pip installs to /usr/local/share/snapcraft but snapcraft looks
  # for its data in /usr/share/snapcraft.
  sudo ln -sf /usr/local/share/snapcraft /usr/share/snapcraft

  # Fake core22 presence so snapcraft can find the dynamic linker.  Ubuntu 22.04 IS
  # core22, so symlinking current → / is accurate.
  sudo mkdir -p /snap/core22
  sudo ln -sf / /snap/core22/current

  # Patch snapcraft 4.8.1's linker detection for core22 and the x86_64 linker name.
  sudo python3 build/container/patch-snapcraft-linker.py
else
  sudo apt-get install -y --no-install-recommends "${BASE_PKGS[@]}"
fi
