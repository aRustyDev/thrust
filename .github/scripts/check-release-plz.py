#!/usr/bin/env python3
"""Hold release-plz.toml to the workspace (the rc/* release flow, docs/src/architecture/cicd.md "Release candidates").

release-plz has no workspace-level version group, so every crate must be listed in release-plz.toml with the same
version_group, or it leaves the shared version without a word. Nothing in release-plz checks that; this does.

    python3 -I .github/scripts/check-release-plz.py [--first-release required|exact] [<repository root>]

It fails (exit 1, one line per problem) when:
  - a workspace member has no [[package]] entry, or an entry names no member;
  - an entry's version_group differs from the others, or is missing;
  - not exactly one entry has git_tag_enable = true (the one tag and release);
  - that entry's changelog_include is not exactly the other members (the one changelog);
  - a member's Cargo.toml does not inherit the workspace version (version.workspace = true);
  - an entry sets git_tag_name to anything but its first-release pattern, <name>-v{{ version }}, or the tagger sets one.

The first-release pattern is for a crate that is not in the last release. Every crate finds its previous version from the
one tag v<version>; release-plz 0.4.3 then looks for the crate in that tag's workspace and, when it is not there, stops
("cannot find package"), so no release-candidate pull request is made. A pattern that matches no tag makes release-plz
treat the crate as a first release. Once a release contains the crate the pattern must go, or every release lists the
crate's whole history again. With --first-release, the last release is the highest tag v<major>.<minor>.<patch>, and a
crate is in it when its package name is (a crate that moved directory is not new):
  required  every crate that is not in the last release has its pattern. Run by the deps job of ci.yml, so the pull
            request that adds a crate adds its pattern.
  exact     and no crate that is in the last release has one. Run by the release-pr job of release-plz.yml before
            release-plz, the run the stale pattern would spoil; in deps it would fail every pull request, and main's run
            of the release commit itself, from the moment the release is tagged until the pattern is removed.
Exit 2: a file is missing or is not TOML, or --first-release finds no tag v<major>.<minor>.<patch> (a checkout without
tags) or cannot read the tag's manifests.

    python3 -I .github/scripts/check-release-plz.py --release-tag [<repository root>] < releases.json

prints the one tag a release made, for release-plz.yml to start release.yml on, and checks nothing else. The input is
release-plz's `releases` output: one object per crate released, each with the tag its git_tag_name gives. Only the tagger
(the entry with git_tag_enable = true) makes its tag, so a crate on its first-release pattern is listed with a tag
<name>-v<version> that does not exist (release v0.3.0 listed five). The tag is the tagger's, and it must be v<version>.
Exit 1 when the tagger is not released exactly once or its tag is not v<its version>; exit 2 when the input is not a JSON
list of objects or release-plz.toml has not exactly one tagger. None of the checks above runs: by now the tag exists, and
a stale first-release pattern (which `exact` refuses) must not keep a made release from being published.
"""

import argparse
import json
import pathlib
import re
import subprocess
import sys
import tomllib

RELEASE_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")


def stop(message):
    print(f"check-release-plz: {message}", file=sys.stderr)
    sys.exit(2)


def load(path):
    try:
        with open(path, "rb") as f:
            return tomllib.load(f)
    except (OSError, tomllib.TOMLDecodeError) as e:
        stop(f"cannot read {path}: {e}")


def first_release_pattern(name):
    return f"{name}-v{{{{ version }}}}"


def git(root, *args):
    try:
        p = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)
    except OSError as e:
        stop(f"cannot run git: {e}")
    if p.returncode != 0:
        stop(f"git {' '.join(args)} failed in {root}: {p.stderr.strip()}")
    return p.stdout


def last_release(root):
    tags = []
    for tag in git(root, "tag", "--list", "v*").split():
        m = RELEASE_TAG.match(tag)
        if m:
            tags.append((tuple(int(g) for g in m.groups()), tag))
    if not tags:
        stop(f"no tag v<major>.<minor>.<patch> in {root}, so there is no last release to compare with (fetch the tags)")
    return max(tags)[1]


def names_at(root, tag):
    def manifest(path):
        try:
            return tomllib.loads(git(root, "show", f"{tag}:{path}"))
        except tomllib.TOMLDecodeError as e:
            stop(f"{tag}:{path} is not TOML: {e}")

    names = set()
    for rel in manifest("Cargo.toml").get("workspace", {}).get("members", []):
        name = manifest(f"{rel}/Cargo.toml").get("package", {}).get("name")
        if name:
            names.add(name)
    return names


def first_release_problems(root, members, entries, mode):
    tag = last_release(root)
    released = names_at(root, tag)
    patterned = {e.get("name") for e in entries if "git_tag_name" in e}
    found = []
    for name in sorted(set(members) - released - patterned):
        found.append(
            f"{name} is not in {tag}, the last release: release-plz looks for it there and stops (\"cannot find package\"); "
            f'give its [[package]] entry git_tag_name = "{first_release_pattern(name)}" until a release contains it'
        )
    if mode == "exact":
        for name in sorted(patterned & released):
            found.append(
                f"{name} is in {tag}: remove its git_tag_name from release-plz.toml, which was for its first release, "
                "or every release lists its whole history again"
            )
    return tag, found


def problems(root, first_release=None):
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

    for e in entries:
        if "git_tag_name" not in e:
            continue
        name = e.get("name")
        if e.get("git_tag_enable") is True:
            found.append(f"{name} makes the one tag, so its entry must not set git_tag_name (the workspace's is the tag)")
        elif e["git_tag_name"] != first_release_pattern(name):
            found.append(
                f"{name}'s git_tag_name is {e['git_tag_name']!r}; the only one allowed is its first-release pattern "
                f"{first_release_pattern(name)!r}"
            )

    tag = None
    if first_release:
        tag, more = first_release_problems(root, members, entries, first_release)
        found += more
    return tag, found


def release_tag(root, text):
    """The tag of the release in release-plz's `releases` output `text`, and None; or None and why there is none."""
    config = load(pathlib.Path(root) / "release-plz.toml")
    taggers = [e.get("name") for e in config.get("package", []) if e.get("git_tag_enable") is True]
    if len(taggers) != 1:
        stop(f"exactly one [[package]] entry must have git_tag_enable = true, found {len(taggers)}")
    tagger = taggers[0]
    try:
        releases = json.loads(text)
    except json.JSONDecodeError as e:
        stop(f"release-plz's releases are not JSON: {e}")
    if not isinstance(releases, list) or not all(isinstance(r, dict) for r in releases):
        stop("release-plz's releases are not a JSON list of objects")
    mine = [r for r in releases if r.get("package_name") == tagger]
    if len(mine) != 1:
        return None, f"release-plz released {tagger}, the crate that makes the tag, {len(mine)} times, not once"
    tag, version = mine[0].get("tag"), mine[0].get("version")
    if not isinstance(version, str) or not version or tag != f"v{version}":
        return None, f"{tagger}'s tag is {tag!r}, not v<version> of its version {version!r}"
    return tag, None


def main(argv):
    parser = argparse.ArgumentParser(prog="check-release-plz.py")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--first-release", choices=["required", "exact"])
    modes.add_argument("--release-tag", action="store_true")
    parser.add_argument("root", nargs="?", default=".")
    args = parser.parse_args(argv[1:])
    if args.release_tag:
        tag, problem = release_tag(args.root, sys.stdin.read())
        if problem:
            print(f"check-release-plz: {problem}", file=sys.stderr)
            return 1
        print(tag)
        return 0
    tag, found = problems(args.root, args.first_release)
    for p in found:
        print(f"check-release-plz: {p}")
    if found:
        return 1
    done = "release-plz.toml covers every workspace member, in one version group, with one tag"
    if tag:
        done += f"; the first-release patterns are {args.first_release} against {tag}"
    print(f"check-release-plz: {done}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
