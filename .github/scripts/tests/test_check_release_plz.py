"""check-release-plz.py over generated workspaces, and the cases it must refuse (the rc/* release flow).

    test_check_release_plz.py [path to a copy of check-release-plz.py]

A synthetic workspace of six crates with a good release-plz.toml must pass. Then every way of breaking it, applied to EVERY crate in
turn (not one example): the crate's entry dropped, its version_group changed, it dropped from the tagger's changelog_include, its
Cargo.toml given a version of its own, a git_tag_name other than its first-release pattern; plus a stray entry, a duplicate entry, no
tagger, two taggers, a tagger with a git_tag_name. Each must exit 1 and name what is wrong.

--first-release over generated git histories, again for every crate: a crate added since the last release needs its pattern
(required and exact), a crate the last release has must not keep one (exact only), a crate that only moved directory is not new,
the last release is the highest v<major>.<minor>.<patch> (not the last in name order, not a pre-release), and no such tag is exit 2.

The repository's own files must pass. A path argument points it at a deliberately broken copy, to show this test fails.
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


def write_workspace(d, entries, own_version=(), crates=CRATES, folder="crates"):
    with open(os.path.join(d, "Cargo.toml"), "w") as f:
        f.write("[workspace]\nmembers = [" + ", ".join(f'"{folder}/{c}"' for c in crates) + ']\n\n')
        f.write('[workspace.package]\nversion = "0.1.0"\n')
    for name in crates:
        os.makedirs(os.path.join(d, folder, name), exist_ok=True)
        version = 'version = "9.9.9"' if name in own_version else "version.workspace = true"
        with open(os.path.join(d, folder, name, "Cargo.toml"), "w") as f:
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

# --first-release. Each repository is a history (commits of a workspace, some tagged), then the workspace under test at
# HEAD. git runs without the global or system configuration: no signing, and no identity of the machine it runs on.
GIT_ENV = dict(
    os.environ,
    GIT_CONFIG_GLOBAL=os.devnull,
    GIT_CONFIG_NOSYSTEM="1",
    GIT_AUTHOR_NAME="test",
    GIT_AUTHOR_EMAIL="test@invalid",
    GIT_COMMITTER_NAME="test",
    GIT_COMMITTER_EMAIL="test@invalid",
)


def git(d, *args):
    subprocess.run(["git", "-C", d, *args], check=True, capture_output=True, env=GIT_ENV)


def pattern(name):
    return f"{name}-v{{{{ version }}}}"


def with_patterns(names):
    entries = good_entries()
    for e in entries:
        if e["name"] in names:
            e["git_tag_name"] = pattern(e["name"])
    return entries


def run_history(history, entries, mode):
    """history: (crates, folder, tags) per commit, oldest first. HEAD: every crate, under crates/, with `entries`."""
    with tempfile.TemporaryDirectory() as d:
        git(d, "init", "-q")
        for crates, folder, tags in history:
            write_workspace(d, good_entries(), crates=crates, folder=folder)
            git(d, "add", "-A")
            git(d, "commit", "-q", "--allow-empty", "-m", "a commit")
            for tag in tags:
                git(d, "tag", "-a", "-m", tag, tag)
        write_workspace(d, entries)
        args = [sys.executable, "-I", SCRIPT] + ([f"--first-release={mode}"] if mode else []) + [d]
        p = subprocess.run(args, capture_output=True, text=True, env=GIT_ENV)
        return p.returncode, p.stdout + p.stderr


for name in CRATES:
    if name == TAGGER:
        continue
    before = [c for c in CRATES if c != name]
    new = [(before, "crates", ["v0.1.0"])]
    rc, out = run_history(new, good_entries(), "required")
    check(f"{name} added since v0.1.0, no pattern, required: exit 1", rc, 1)
    check(f"{name} added since v0.1.0, no pattern, required: names it", f"{name} is not in v0.1.0" in out, True)
    rc, out = run_history(new, good_entries(), None)
    check(f"{name} added since v0.1.0, no pattern, without --first-release (no tags read): exit 0", rc, 0)
    for mode in ("required", "exact"):
        rc, out = run_history(new, with_patterns({name}), mode)
        check(f"{name} added since v0.1.0 with its pattern, {mode}: exit 0", rc, 0)

    released = [(CRATES, "crates", ["v0.1.0"])]
    rc, out = run_history(released, with_patterns({name}), "exact")
    check(f"{name} in v0.1.0 with a pattern, exact: exit 1", rc, 1)
    check(f"{name} in v0.1.0 with a pattern, exact: names it", f"{name} is in v0.1.0: remove its git_tag_name" in out, True)
    rc, out = run_history(released, with_patterns({name}), "required")
    check(f"{name} in v0.1.0 with a pattern, required (the window until it is removed): exit 0", rc, 0)

    # Any other value is refused, without tags: the workspace's own pattern (which leaves release-plz stopping), another
    # crate's, one that is not a version tag.
    for value in ("v{{ version }}", pattern("crate-x"), f"{name}-{{{{ version }}}}"):
        entries = good_entries()
        next(e for e in entries if e["name"] == name)["git_tag_name"] = value
        rc, out = run(entries)
        check(f"{name} with git_tag_name {value!r}: exit 1", rc, 1)
        check(f"{name} with git_tag_name {value!r}: names it", f"{name}'s git_tag_name is" in out, True)

entries = with_patterns({TAGGER})
rc, out = run(entries)
check("the tagger with a git_tag_name: exit 1", rc, 1)
check("the tagger with a git_tag_name: says it makes the tag", f"{TAGGER} makes the one tag" in out, True)

rc, out = run_history([(CRATES, "old", ["v0.1.0"])], good_entries(), "exact")
check("every crate moved directory since v0.1.0, no pattern, exact (a crate is its name): exit 0", rc, 0)

# The last release is the highest version, not the last tag in name order, and a tag that is not v<major>.<minor>.<patch>
# is not a release.
LATE = "crate-4"
without = [c for c in CRATES if c != LATE]
rc, out = run_history([(CRATES, "crates", ["v0.9.0"]), (without, "crates", ["v0.10.0"])], good_entries(), "required")
check("v0.10.0 lacks the crate, v0.9.0 has it: exit 1", rc, 1)
check("v0.10.0 lacks the crate, v0.9.0 has it: v0.10.0 is the last release", f"{LATE} is not in v0.10.0" in out, True)
rc, out = run_history([(without, "crates", ["v0.9.0"]), (CRATES, "crates", ["v0.10.0"])], good_entries(), "required")
check("v0.9.0 lacks the crate, v0.10.0 has it: exit 0", rc, 0)
history = [(without, "crates", ["v1.0.0-rc.1", "v2.0", "vnext"]), (CRATES, "crates", ["v0.10.0"])]
rc, out = run_history(history, good_entries(), "required")
check("higher tags that are not releases lack the crate, v0.10.0 has it: exit 0", rc, 0)

rc, out = run_history([(CRATES, "crates", ["v1.0", "vnext"])], good_entries(), "required")
check("no release tag, required: exit 2", rc, 2)
check("no release tag, required: says so", "no tag v<major>.<minor>.<patch>" in out, True)

p = subprocess.run([sys.executable, "-I", SCRIPT, "--first-release=sometimes", ROOT], capture_output=True, text=True)
check("an unknown --first-release mode: exit 2", p.returncode, 2)

with tempfile.TemporaryDirectory() as d:
    p = subprocess.run([sys.executable, "-I", SCRIPT, d], capture_output=True, text=True)
    check("no files: exit 2", p.returncode, 2)

p = subprocess.run([sys.executable, "-I", SCRIPT, ROOT], capture_output=True, text=True)
check("the repository's own release-plz.toml passes", p.returncode, 0)
if p.returncode != 0:
    print(p.stdout + p.stderr)

print(f"test_check_release_plz: {passed} passed, {failed} failed")
sys.exit(1 if failed else 0)
