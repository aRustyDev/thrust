"""scripts/merge-assets.sh over generated release trees, and the cases it must refuse (issue #49, ADR-0011).

    test_merge_assets.py [path to a copy of merge-assets.sh]

For 200 seeded pairs of trees (a new release and the previous one, with names that overlap, names with spaces and unicode, deep paths, the
same path with different bytes, files only one of them has) the output directory must be, by a rule written here and not read from the
script: every file of either release, and where both have a path, the NEW release's bytes. The count it prints must agree. Then the named
cases: the first deploy, a missing index.html on either side, a symlink, a name with a newline, an output directory that is not empty,
and the file limit. A path argument points it at a deliberately broken copy, to show this test fails.
"""

import os
import random
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "..", "..", "scripts", "merge-assets.sh")

passed = 0
failed = 0


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


def write_tree(root, files):
    for rel, data in files.items():
        path = os.path.join(root, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "wb") as f:
            f.write(data)


def read_tree(root):
    out = {}
    for dirpath, _dirs, names in os.walk(root):
        for n in names:
            p = os.path.join(dirpath, n)
            with open(p, "rb") as f:
                out[os.path.relpath(p, root)] = f.read()
    return out


def merge(new, prev, out, env=None):
    e = dict(os.environ)
    if env:
        e.update(env)
    p = subprocess.run(["/bin/sh", SCRIPT, new, prev, out], capture_output=True, text=True, env=e)
    return p.returncode, p.stdout, p.stderr


NAME_POOL = [
    "assets/app-{h}.js", "assets/chunk-{h}.js", "assets/vendor-{h}.js", "assets/style-{h}.css", "assets/logo-{h}.svg",
    "assets/nested/deep/er/file-{h}.js", "assets/with space-{h}.js", "assets/ünï-{h}.js", "icons/icon-{h}.png",
    "sw.js", "manifest.json", "workbox-9f2f79cf.js", "favicon/favicon.ico",
]


def random_release(rng, hashes):
    files = {"index.html": b"<html>" + bytes([rng.randrange(256)]) + b"</html>"}
    for pattern in NAME_POOL:
        if rng.random() < 0.55:
            h = rng.choice(hashes)
            files[pattern.format(h=h)] = bytes([rng.randrange(256)]) * rng.randrange(1, 40)
    return files


# ---- generated pairs -----------------------------------------------------------------------------------------------------------------
for seed in range(200):
    rng = random.Random(seed)
    hashes = ["a1", "b2", "c3", "d4"]
    new = random_release(rng, hashes)
    prev = random_release(rng, hashes)
    with tempfile.TemporaryDirectory() as d:
        nd, pd, od = os.path.join(d, "new"), os.path.join(d, "prev"), os.path.join(d, "out")
        write_tree(nd, new)
        write_tree(pd, prev)
        rc, stdout, stderr = merge(nd, pd, od)
        want = dict(prev)
        want.update(new)  # a path both have is the new release's
        check(f"seed {seed}: exit", rc, 0)
        if rc != 0:
            print(stderr)
            continue
        got = read_tree(od)
        check(f"seed {seed}: the files are every file of either release, the new release's bytes where both have a path", got, want)
        retained = len(set(prev) - set(new))
        check(f"seed {seed}: the summary", stdout.strip(), f"merge-assets: new={len(new)} retained={retained} total={len(want)}")
        check(f"seed {seed}: the new release's directory is untouched", read_tree(nd), new)
        check(f"seed {seed}: the previous release's directory is untouched", read_tree(pd), prev)

# ---- the first deploy: no previous release ---------------------------------------------------------------------------------------------
with tempfile.TemporaryDirectory() as d:
    new = {"index.html": b"<html>", "assets/app-a1.js": b"js"}
    nd, od = os.path.join(d, "new"), os.path.join(d, "out")
    write_tree(nd, new)
    rc, stdout, stderr = merge(nd, "-", od)
    check("first deploy: exit", rc, 0)
    check("first deploy: the output is the new release", read_tree(od), new)
    check("first deploy: nothing retained", stdout.strip(), "merge-assets: new=2 retained=0 total=2")

# ---- refusals ----------------------------------------------------------------------------------------------------------------------------
def refused(label, setup, want_rc, fragment, env=None):
    with tempfile.TemporaryDirectory() as d:
        nd, pd, od = os.path.join(d, "new"), os.path.join(d, "prev"), os.path.join(d, "out")
        extra = setup(d, nd, pd, od)
        rc, stdout, stderr = merge(nd, extra.get("prev", pd), od, env)
        check(f"{label}: exit", rc, want_rc)
        check(f"{label}: says why", fragment in stderr, True)
        if want_rc == 1:
            check(f"{label}: leaves no output behind", os.path.exists(od), False)


def s_new_no_index(d, nd, pd, od):
    write_tree(nd, {"assets/app-a1.js": b"js"})
    write_tree(pd, {"index.html": b"<html>"})
    return {}


def s_prev_no_index(d, nd, pd, od):
    write_tree(nd, {"index.html": b"<html>"})
    write_tree(pd, {"assets/app-a1.js": b"js"})
    return {}


def s_symlink_prev(d, nd, pd, od):
    write_tree(nd, {"index.html": b"<html>"})
    write_tree(pd, {"index.html": b"<html>"})
    os.symlink("/etc/passwd", os.path.join(pd, "assets-link"))
    return {}


def s_symlink_new(d, nd, pd, od):
    write_tree(nd, {"index.html": b"<html>"})
    write_tree(pd, {"index.html": b"<html>"})
    os.symlink("/etc/passwd", os.path.join(nd, "assets-link"))
    return {}


def s_newline_name(d, nd, pd, od):
    write_tree(nd, {"index.html": b"<html>"})
    write_tree(pd, {"index.html": b"<html>", "assets/bad\nname.js": b"js"})
    return {}


def s_over_limit(d, nd, pd, od):
    write_tree(nd, {"index.html": b"<html>", "a.js": b"1", "b.js": b"2"})
    write_tree(pd, {"index.html": b"<html>", "c.js": b"3", "d.js": b"4", "e.js": b"5"})  # 3 + 3 retained = 6 files, one over a limit of 5
    return {}


refused("a new release without index.html", s_new_no_index, 1, "no index.html")
refused("a previous release without index.html", s_prev_no_index, 1, "not a web build")
refused("a symlink in the previous release", s_symlink_prev, 1, "has a symlink")
refused("a symlink in the new release", s_symlink_new, 1, "has a symlink")
refused("a file name with a newline", s_newline_name, 1, "newline")
refused("more files than the limit", s_over_limit, 1, "more than the 5 that Workers static assets take", {"MERGE_ASSETS_LIMIT": "5"})

with tempfile.TemporaryDirectory() as d:
    nd, pd, od = os.path.join(d, "new"), os.path.join(d, "prev"), os.path.join(d, "out")
    write_tree(nd, {"index.html": b"<html>"})
    write_tree(pd, {"index.html": b"<html>"})
    write_tree(od, {"leftover.txt": b"from an earlier run"})
    rc, _o, e = merge(nd, pd, od)
    check("an output directory that is not empty: usage error", rc, 2)
    check("an output directory that is not empty: says why", "already has something in it" in e, True)
    check("an output directory that is not empty: is left alone", read_tree(od), {"leftover.txt": b"from an earlier run"})
    rc, _o, e = merge(os.path.join(d, "nope"), pd, os.path.join(d, "out2"))
    check("a new directory that is not there: usage error", rc, 2)
    rc, _o, e = merge(nd, os.path.join(d, "nope"), os.path.join(d, "out3"))
    check("a previous directory that is not there: usage error", rc, 2)
    p = subprocess.run(["/bin/sh", SCRIPT], capture_output=True, text=True)
    check("no arguments: usage error", p.returncode, 2)
    rc, _o, e = merge(nd, pd, os.path.join(d, "out4"), {"MERGE_ASSETS_LIMIT": "many"})
    check("a limit that is not a number: usage error", rc, 2)

print(f"passed={passed} bad={failed}")
sys.exit(1 if failed else 0)
