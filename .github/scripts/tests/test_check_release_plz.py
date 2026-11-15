"""check-release-plz.py over generated workspaces, and the cases it must refuse (the rc/* release flow).

    test_check_release_plz.py [path to a copy of check-release-plz.py]

A synthetic workspace of six crates with a good release-plz.toml must pass. Then every way of breaking it, applied to EVERY crate in
turn (not one example): the crate's entry dropped, its version_group changed, it dropped from the tagger's changelog_include, its
Cargo.toml given a version of its own; plus a stray entry, a duplicate entry, no tagger, two taggers. Each must exit 1 and name what
is wrong. The repository's own files must pass. A path argument points it at a deliberately broken copy, to show this test fails.
"""

import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "..", "..", "..")
SCRIPT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "check-release-plz.py")

passed = 0
failed = 0


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


def toml_value(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, list):
        return "[" + ", ".join(toml_value(x) for x in v) + "]"
    return '"' + str(v) + '"'


CRATES = [f"crate-{i}" for i in range(6)]
TAGGER = "crate-2"


def good_entries():
    entries = []
    for name in CRATES:
        e = {"name": name, "version_group": "g"}
        if name == TAGGER:
            e.update(git_tag_enable=True, changelog_include=[c for c in CRATES if c != TAGGER])
        entries.append(e)
    return entries


def write_workspace(d, entries, own_version=()):
    with open(os.path.join(d, "Cargo.toml"), "w") as f:
        f.write("[workspace]\nmembers = [" + ", ".join(f'"crates/{c}"' for c in CRATES) + ']\n\n')
        f.write('[workspace.package]\nversion = "0.1.0"\n')
    for name in CRATES:
        os.makedirs(os.path.join(d, "crates", name), exist_ok=True)
        version = 'version = "9.9.9"' if name in own_version else "version.workspace = true"
        with open(os.path.join(d, "crates", name, "Cargo.toml"), "w") as f:
            f.write(f'[package]\nname = "{name}"\n{version}\n')
    with open(os.path.join(d, "release-plz.toml"), "w") as f:
        f.write("[workspace]\ngit_only = true\n")
        for e in entries:
            f.write("\n[[package]]\n")
            for k, v in e.items():
                f.write(f"{k} = {toml_value(v)}\n")


def run(entries, own_version=()):
    with tempfile.TemporaryDirectory() as d:
        write_workspace(d, entries, own_version)
        p = subprocess.run([sys.executable, "-I", SCRIPT, d], capture_output=True, text=True)
        return p.returncode, p.stdout + p.stderr


rc, out = run(good_entries())
check("the good workspace passes", rc, 0)

for name in CRATES:
    entries = [e for e in good_entries() if e["name"] != name]
    if name == TAGGER:
        rc, out = run(entries)
        check(f"{name} (the tagger) dropped: exit 1", rc, 1)
        check(f"{name} (the tagger) dropped: no tagger left", "found 0" in out, True)
    else:
        rc, out = run(entries)
        check(f"{name} dropped: exit 1", rc, 1)
        check(f"{name} dropped: names it", f"{name} is a workspace member with no [[package]] entry" in out, True)

    entries = good_entries()
    next(e for e in entries if e["name"] == name)["version_group"] = "other"
    rc, out = run(entries)
    check(f"{name} in another version group: exit 1", rc, 1)
    check(f"{name} in another version group: says so", "same version_group" in out, True)

    entries = good_entries()
    del next(e for e in entries if e["name"] == name)["version_group"]
    rc, out = run(entries)
    check(f"{name} with no version group: exit 1", rc, 1)

    rc, out = run(good_entries(), own_version={name})
    check(f"{name} with a version of its own: exit 1", rc, 1)
    check(f"{name} with a version of its own: names it", f"{name} does not inherit the workspace version" in out, True)

    if name != TAGGER:
        entries = good_entries()
        tagger = next(e for e in entries if e["name"] == TAGGER)
        tagger["changelog_include"] = [c for c in tagger["changelog_include"] if c != name]
        rc, out = run(entries)
        check(f"{name} missing from changelog_include: exit 1", rc, 1)
        check(f"{name} missing from changelog_include: names it", f"{name} is missing from {TAGGER}'s changelog_include" in out, True)

        entries = good_entries()
        next(e for e in entries if e["name"] == name)["git_tag_enable"] = True
        rc, out = run(entries)
        check(f"{name} as a second tagger: exit 1", rc, 1)
        check(f"{name} as a second tagger: says found 2", "found 2" in out, True)

entries = good_entries()
for e in entries:
    del e["version_group"]
rc, out = run(entries)
check("no entry has a version group (all equal, all missing): exit 1", rc, 1)

rc, out = run(good_entries() + [{"name": "stray", "version_group": "g"}])
check("a stray entry: exit 1", rc, 1)
check("a stray entry: names it", "entry for stray, which is not a workspace member" in out, True)

rc, out = run(good_entries() + [{"name": "crate-0", "version_group": "g"}])
check("a duplicate entry: exit 1", rc, 1)
check("a duplicate entry: says so", "more than one [[package]] entry for crate-0" in out, True)

entries = good_entries()
next(e for e in entries if e["name"] == TAGGER)["changelog_include"].append("stray")
rc, out = run(entries)
check("changelog_include names a non-member: exit 1", rc, 1)

with tempfile.TemporaryDirectory() as d:
    p = subprocess.run([sys.executable, "-I", SCRIPT, d], capture_output=True, text=True)
    check("no files: exit 2", p.returncode, 2)

p = subprocess.run([sys.executable, "-I", SCRIPT, ROOT], capture_output=True, text=True)
check("the repository's own release-plz.toml passes", p.returncode, 0)
if p.returncode != 0:
    print(p.stdout + p.stderr)

print(f"test_check_release_plz: {passed} passed, {failed} failed")
sys.exit(1 if failed else 0)
