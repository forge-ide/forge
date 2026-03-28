"""Patch snapcraft 4.8.1 linker detection for core22 compatibility.

snapcraft 4.8.1 expects the dynamic linker to be named ld-<version>.so
(e.g. ld-2.35.so), but on x86_64 the actual linker is ld-linux-x86-64.so.2.
This script applies three patches:

1. Adds "core22": "2.99" to the known linker version map.  A deliberately
   high version is used so snapcraft doesn't reject prebuilt native modules
   that target newer glibc; classic snaps use the host linker anyway.
2. Widens the regex in elf.py to also match ld-linux-*.so.N names.
3. Widens the regex in file_utils.py to also match ld-linux-*.so.N names
   and updates the group extraction accordingly.
"""

import pathlib
import sys
import sysconfig

SNAPCRAFT = pathlib.Path(sysconfig.get_path("purelib")) / "snapcraft"


def patch_project_options():
    """Add core22 to _LINKER_VERSION_FOR_BASE."""
    f = SNAPCRAFT / "project" / "_project_options.py"
    old = '"core20": "2.31", "core18": "2.27", "core": "2.23"'
    # Use a high version so snapcraft doesn't reject prebuilt native modules
    # that target newer glibc.  Classic snaps use the host linker anyway.
    new = '"core22": "2.99", "core20": "2.31", "core18": "2.27", "core": "2.23"'
    t = f.read_text()
    if old not in t:
        print(f"WARNING: expected string not found in {f}", file=sys.stderr)
        return False
    f.write_text(t.replace(old, new))
    return True


def patch_elf():
    """Widen _get_dynamic_linker regex to accept ld-linux-x86-64.so.2."""
    f = SNAPCRAFT / "internal" / "elf.py"
    old = r'(?P<dynamic_linker>ld-[\d.]+.so)$'
    new = r'(?P<dynamic_linker>ld-[\d.]+\.so|ld-linux-[\w-]+\.so\.[\d.]+)$'
    t = f.read_text()
    if old not in t:
        print(f"WARNING: expected regex not found in {f}", file=sys.stderr)
        return False
    f.write_text(t.replace(old, new))
    return True


def patch_file_utils():
    """Widen get_linker_version_from_file regex and group extraction."""
    f = SNAPCRAFT / "file_utils.py"
    t = f.read_text()

    old_re = r'ld-(?P<linker_version>[\d.]+).so$'
    new_re = r'ld-(?:(?P<lv1>[\d.]+)\.so|linux-[\w-]+\.so\.(?P<lv2>[\d.]+))$'
    if old_re not in t:
        print(f"WARNING: expected regex not found in {f}", file=sys.stderr)
        return False
    t = t.replace(old_re, new_re)

    old_group = 'm.group("linker_version")'
    new_group = '(m.group("lv1") or m.group("lv2"))'
    if old_group not in t:
        print(f"WARNING: expected group call not found in {f}", file=sys.stderr)
        return False
    t = t.replace(old_group, new_group)

    f.write_text(t)
    return True


if __name__ == "__main__":
    ok = all([patch_project_options(), patch_elf(), patch_file_utils()])
    if ok:
        print("Patched snapcraft linker detection for core22")
    else:
        sys.exit(1)
