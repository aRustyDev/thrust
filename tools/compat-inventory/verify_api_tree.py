# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Prove, with plain file reads (no git process), that `apps/api` in a scanned tree is the tree the oracle pin records.

oracle/PIN carries `apps_api_tree=<git tree sha>`. This recomputes the git tree object id of `<root>/apps/api` from the working
files (blob = sha1("blob <n>\\0" + bytes), tree = sha1("tree <n>\\0" + sorted entries), modes 100644 / 100755 / 120000) and
compares. Untracked noise (__pycache__, *.pyc, caches, virtualenvs) is excluded; if the tree has other untracked files the hash
will not match and the result is reported as `match: false`, never silently accepted.

Usage:  uv run verify_api_tree.py --root <plane tree root> [--out file.json]
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
from pathlib import Path

SKIP_NAMES = {"__pycache__", ".pytest_cache", ".ruff_cache", ".mypy_cache", ".venv", "venv", ".DS_Store", "node_modules", ".coverage", "htmlcov"}


def blob_sha(p: Path) -> str:
    if p.is_symlink():
        data = os.readlink(p).encode()
    else:
        data = p.read_bytes()
    return hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()


def tree_sha(d: Path) -> str | None:
    entries = []
    for name in os.listdir(d):
        if name in SKIP_NAMES or name.endswith(".pyc"):
            continue
        p = d / name
        if p.is_symlink():
            entries.append((name, "120000", blob_sha(p), False))
        elif p.is_dir():
            sub = tree_sha(p)
            if sub is not None:  # git does not store empty trees
                entries.append((name, "40000", sub, True))
        elif p.is_file():
            mode = "100755" if p.stat().st_mode & stat.S_IXUSR else "100644"
            entries.append((name, mode, blob_sha(p), False))
    if not entries:
        return None
    entries.sort(key=lambda e: e[0] + ("/" if e[3] else ""))  # git orders directories as if their name ended in "/"
    body = b"".join(f"{mode} {name}".encode() + b"\0" + bytes.fromhex(sha) for name, mode, sha, _ in entries)
    return hashlib.sha1(b"tree %d\0" % len(body) + body).hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", required=True, type=Path)
    ap.add_argument("--out", type=Path)
    a = ap.parse_args()
    root = a.root.resolve()
    pin = {}
    f = root / "oracle" / "PIN"
    if f.is_file():
        for line in f.read_text().splitlines():
            if "=" in line and not line.startswith("#"):
                k, v = line.split("=", 1)
                pin[k.strip()] = v.strip()
    got = tree_sha(root / "apps" / "api")
    res = {"root": str(root), "computed_apps_api_tree": got, "pin_apps_api_tree": pin.get("apps_api_tree"),
           "match": bool(pin.get("apps_api_tree")) and got == pin.get("apps_api_tree")}
    print(json.dumps(res, indent=1))
    if a.out:
        a.out.write_text(json.dumps(res, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
