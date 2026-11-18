#!/usr/bin/env python3
"""Build an untrusted migration artifact from an exact Git commit, without running PR code.

Usage: migration-artifact.py --repo <checkout> --commit <full commit SHA>
The caller must separately prove that the SHA is the current authorized PR head. The output is
data, not an executable or an approval to run SQL. In particular, transaction control is not
validated here. Errors never produce a partial artifact on stdout.
"""

import argparse
import hashlib
import json
import re
import subprocess
import sys

DIRECTORY = "crates/adapters/postgres/migrations/"
NAME = re.compile(r"[0-9]{4}_[a-z0-9_]+\.sql\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
MAX_FILES = 256
MAX_FILE_BYTES = 512 * 1024
MAX_TOTAL_BYTES = 4 * 1024 * 1024
MAX_TREE_BYTES = 128 * 1024


def git(repo, args, limit):
    """Read at most limit bytes from git, including when a hostile tree is oversized."""
    proc = subprocess.Popen(["git", "-C", repo, *args], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
    try:
        data = proc.stdout.read(limit + 1)
        if len(data) > limit:
            proc.kill()
            proc.wait()
            raise ValueError("git output exceeds its limit")
        if proc.wait(timeout=30) != 0:
            raise ValueError("git could not read the requested object")
        return data
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
        proc.stdout.close()


def build(repo, commit):
    if not SHA.fullmatch(commit):
        raise ValueError("commit must be a full lowercase SHA-1 commit id")
    resolved = git(repo, ["rev-parse", "--verify", f"{commit}^{{commit}}"], 64).decode("ascii").strip()
    if resolved != commit:
        raise ValueError("commit must name a commit object exactly")
    tree = git(repo, ["ls-tree", "-rz", "--full-tree", commit, "--", DIRECTORY], MAX_TREE_BYTES)
    records = tree.split(b"\0")
    if records[-1:] != [b""]:
        raise ValueError("git tree output is incomplete")
    entries = []
    seen_versions = set()
    total = 0
    for record in records[:-1]:
        metadata, path = record.split(b"\t", 1)
        mode, kind, oid = metadata.decode("ascii").split(" ")
        path = path.decode("utf-8")
        name = path.removeprefix(DIRECTORY)
        if not path.startswith(DIRECTORY) or not NAME.fullmatch(name) or mode != "100644" or kind != "blob" or not SHA.fullmatch(oid):
            raise ValueError("migration tree contains an invalid file, path, mode or object")
        if name[:4] in seen_versions:
            raise ValueError("migration version numbers must be unique")
        seen_versions.add(name[:4])
        size_text = git(repo, ["cat-file", "-s", oid], 32).decode("ascii").strip()
        if not size_text.isdecimal() or int(size_text) > MAX_FILE_BYTES:
            raise ValueError("migration file exceeds its size limit")
        size = int(size_text)
        total += size
        if total > MAX_TOTAL_BYTES or len(entries) >= MAX_FILES:
            raise ValueError("migration artifact exceeds its size or count limit")
        blob = git(repo, ["cat-file", "blob", oid], MAX_FILE_BYTES)
        if len(blob) != size or b"\x00" in blob:
            raise ValueError("migration blob is incomplete or contains a NUL byte")
        entries.append({"name": name, "sha256": hashlib.sha256(blob).hexdigest(), "sql": blob.decode("utf-8")})
    if not entries:
        raise ValueError("migration tree is empty")
    entries.sort(key=lambda entry: entry["name"])
    return {"version": 1, "commit": commit, "files": entries}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--commit", required=True)
    args = parser.parse_args()
    try:
        artifact = build(args.repo, args.commit)
    except (OSError, UnicodeError, ValueError, subprocess.TimeoutExpired) as error:
        print(f"migration-artifact: {error}", file=sys.stderr)
        return 1
    json.dump(artifact, sys.stdout, ensure_ascii=True, separators=(",", ":"))
    print()
    return 0


if __name__ == "__main__":
    sys.exit(main())
