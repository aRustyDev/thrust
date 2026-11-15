#!/usr/bin/env python3
"""Hold release-plz.toml to the workspace (the rc/* release flow, docs/src/architecture/cicd.md "Release candidates").

release-plz has no workspace-level version group, so every crate must be listed in release-plz.toml with the same
version_group, or it leaves the shared version without a word. Nothing in release-plz checks that; this does.

    python3 -I .github/scripts/check-release-plz.py [<repository root>]

It fails (exit 1, one line per problem) when:
  - a workspace member has no [[package]] entry, or an entry names no member;
  - an entry's version_group differs from the others, or is missing;
  - not exactly one entry has git_tag_enable = true (the one tag and release);
  - that entry's changelog_include is not exactly the other members (the one changelog);
  - a member's Cargo.toml does not inherit the workspace version (version.workspace = true).
Exit 2: a file is missing or is not TOML.
"""

import pathlib
import sys
import tomllib


def load(path):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        print(f"check-release-plz: cannot read {path}: {e}", file=sys.stderr)
        sys.exit(2)


def problems(root):
    root = pathlib.Path(root)
    workspace = load(root / "Cargo.toml")
    config = load(root / "release-plz.toml")
    found = []

    members = {}
    for rel in workspace.get("workspace", {}).get("members", []):
        manifest = load(root / rel / "Cargo.toml")
        package = manifest.get("package", {})
        name = package.get("name")
        if not name:
            found.append(f"{rel}/Cargo.toml has no package name")
            continue
        members[name] = rel
        if package.get("version") != {"workspace": True}:
            found.append(f"{name} does not inherit the workspace version (version.workspace = true)")
    if not members:
        found.append("Cargo.toml lists no workspace members")

    entries = config.get("package", [])
    names = [e.get("name") for e in entries]
    for name in sorted(set(members) - set(names)):
        found.append(f"{name} is a workspace member with no [[package]] entry in release-plz.toml")
    for name in sorted(set(names) - set(members)):
        found.append(f"release-plz.toml has a [[package]] entry for {name}, which is not a workspace member")
    for name in sorted({n for n in names if names.count(n) > 1}):
        found.append(f"release-plz.toml has more than one [[package]] entry for {name}")

    groups = {e.get("version_group") for e in entries}
    if len(groups) != 1 or None in groups:
        found.append(f"every [[package]] entry must have the same version_group, found {sorted(map(str, groups))}")

    taggers = [e for e in entries if e.get("git_tag_enable") is True]
    if len(taggers) != 1:
        found.append(f"exactly one [[package]] entry must have git_tag_enable = true, found {len(taggers)}")
    else:
        tagger = taggers[0]
        want = set(members) - {tagger.get("name")}
        got = set(tagger.get("changelog_include", []))
        for name in sorted(want - got):
            found.append(f"{name} is missing from {tagger.get('name')}'s changelog_include")
        for name in sorted(got - want):
            found.append(f"{tagger.get('name')}'s changelog_include names {name}, which is not another workspace member")
    return found


def main(argv):
    if len(argv) > 2:
        print("usage: check-release-plz.py [<repository root>]", file=sys.stderr)
        return 2
    found = problems(argv[1] if len(argv) == 2 else ".")
    for p in found:
        print(f"check-release-plz: {p}")
    if found:
        return 1
    print("check-release-plz: release-plz.toml covers every workspace member, in one version group, with one tag")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
