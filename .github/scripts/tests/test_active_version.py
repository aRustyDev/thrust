"""active-version.py against the shapes wrangler is expected to print, and against everything it must not guess about (issue #49).

    test_active_version.py [path to a copy of active-version.py]

The shapes are the documented ones and are UNVERIFIED against a real account until the first staging deploy; what this test fixes is that
every other shape lands in `unknown`, never in `none` or `single`, because a deploy that cannot say what it would roll back to must not
roll back by itself. A path argument points it at a deliberately broken copy, to show the test fails.
"""

import importlib.util
import itertools
import json
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "active-version.py")

spec = importlib.util.spec_from_file_location("active_version", SCRIPT)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

passed = 0
failed = 0


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


def dep(version_ids_percent, created):
    return {
        "id": "dep-" + created,
        "source": "wrangler",
        "strategy": "percentage",
        "created_on": created,
        "versions": [{"version_id": v, "percentage": p} for v, p in version_ids_percent],
        "annotations": {"workers/triggered_by": "deploy"},
    }


def ver(vid, tag=None, number=1):
    annotations = {"workers/triggered_by": "upload"}
    if tag is not None:
        annotations["workers/tag"] = tag
    return {"id": vid, "number": number, "metadata": {"created_on": "2026-11-14T00:00:00Z"}, "annotations": annotations}


def run(deployments, versions):
    d = deployments if isinstance(deployments, str) else json.dumps(deployments)
    v = versions if isinstance(versions, str) else json.dumps(versions)
    return mod.classify(d, v)


SHA_A = "a" * 40
SHA_B = "b" * 40
SHA_C = "c" * 40

# ---- the three states a healthy Worker is in -----------------------------------------------------------------------------------
check("no deployment is `none`", run([], [])["state"], "none")
check("no deployment, in an envelope, is `none`", run({"result": {"deployments": []}}, [])["state"], "none")

got = run([dep([("v1", 100)], "2026-11-14T10:00:00Z")], [ver("v1", SHA_A)])
check("one deployment, one version, with its tag", (got["state"], got["version_id"], got["tag"]), ("single", "v1", SHA_A))

got = run([dep([("v1", 100)], "2026-11-14T10:00:00Z")], [ver("v0", SHA_B), ver("v1")])
check("a version that has no tag", (got["state"], got["version_id"], got["tag"]), ("single", "v1", ""))

got = run([dep([("v1", 100)], "2026-11-14T10:00:00Z")], [ver("v0", SHA_B)])
check("a live version that is not among the versions listed", (got["state"], got["version_id"], got["tag"]), ("single", "v1", ""))

got = run([dep([("v1", 100)], "2026-11-14T10:00:00Z")], "not json at all")
check("a versions listing that does not parse still gives the live version", (got["state"], got["version_id"], got["tag"]), ("single", "v1", ""))

# ---- the newest deployment is the live one, whatever order the CLI printed them in --------------------------------------------
history = [
    dep([("v1", 100)], "2026-11-14T08:00:00Z"),
    dep([("v2", 100)], "2026-11-14T09:00:00Z"),
    dep([("v3", 100)], "2026-11-14T10:00:00Z"),
]
versions = [ver("v1", SHA_A), ver("v2", SHA_B), ver("v3", SHA_C)]
for order in itertools.permutations(history):
    got = run(list(order), versions)
    check("every order of three deployments: the newest is live", (got["state"], got["version_id"], got["tag"]), ("single", "v3", SHA_C))
for order in itertools.permutations(versions):
    got = run(history, list(order))
    check("every order of the versions: the tag is the live version's", got["tag"], SHA_C)

# ---- a version with 0% of traffic (an override) is not "taking traffic" ----------------------------------------------------------
got = run([dep([("v2", 100), ("v1", 0)], "2026-11-14T10:00:00Z")], [ver("v2", SHA_B), ver("v1", SHA_A)])
check("a 0% version beside the live one is ignored", (got["state"], got["version_id"]), ("single", "v2"))

# ---- what it must NOT call single or none ------------------------------------------------------------------------------------------
got = run([dep([("v1", 60), ("v2", 40)], "2026-11-14T10:00:00Z")], [ver("v1", SHA_A), ver("v2", SHA_B)])
check("a gradual deployment is `split`", (got["state"], got["version_id"], got["tag"]), ("split", "", ""))

got = run([dep([("v1", 50)], "2026-11-14T10:00:00Z")], [ver("v1", SHA_A)])
check("one version with 50% is `unknown`", got["state"], "unknown")

got = run([dep([("v1", 0)], "2026-11-14T10:00:00Z")], [ver("v1", SHA_A)])
check("a deployment with no traffic anywhere is `unknown`", got["state"], "unknown")

check("empty output is `unknown`", run("", [])["state"], "unknown")
check("plain text is `unknown`", run("Worker not found", [])["state"], "unknown")
check("an object that is not an envelope is `unknown`", run({"errors": [{"code": 10007}]}, [])["state"], "unknown")
check("a list of non-objects is `unknown`", run([1, 2, 3], [])["state"], "unknown")
check("a deployment without created_on is `unknown`", run([{"versions": [{"version_id": "v1", "percentage": 100}]}], [])["state"], "unknown")
check("a deployment without versions is `unknown`", run([{"created_on": "2026-11-14T10:00:00Z"}], [])["state"], "unknown")
check("a version without an id is `unknown`", run([dep([("", 100)], "2026-11-14T10:00:00Z")], [])["state"], "unknown")

# ---- wrangler's banner and log lines around the JSON ------------------------------------------------------------------------------
noisy = " ⛅️ wrangler 4.151.2\n-------------------\n" + json.dumps([dep([("v1", 100)], "2026-11-14T10:00:00Z")], indent=2) + "\n"
got = run(noisy, [ver("v1", SHA_A)])
check("a banner before the JSON is tolerated", (got["state"], got["tag"]), ("single", SHA_A))
got = run([dep([("v1", 100)], "2026-11-14T10:00:00Z")], " ⛅️ wrangler\n" + json.dumps([ver("v1", SHA_B)]) + "\n")
check("a banner before the versions is tolerated", got["tag"], SHA_B)

# ---- what is printed ends up in $GITHUB_OUTPUT, so it must not be able to carry a newline or a space -----------------------------
for evil in ("abc\nstate=none", "abc def", "abc;rm -rf /", "$(id)", "a`b`"):
    got = run([dep([("v1", 100)], "2026-11-14T10:00:00Z")], [ver("v1", evil)])
    check(f"a tag with {evil!r} in it is refused", got["state"], "unknown")
    got = run([dep([(evil, 100)], "2026-11-14T10:00:00Z")], [])
    check(f"a version id with {evil!r} in it is refused", got["state"], "unknown")

# ---- the command line --------------------------------------------------------------------------------------------------------------
with tempfile.TemporaryDirectory() as d:
    dp = os.path.join(d, "deployments.json")
    vp = os.path.join(d, "versions.json")
    with open(dp, "w") as f:
        json.dump([dep([("v1", 100)], "2026-11-14T10:00:00Z")], f)
    with open(vp, "w") as f:
        json.dump([ver("v1", SHA_A)], f)
    p = subprocess.run([sys.executable, "-I", SCRIPT, dp, vp], capture_output=True, text=True)
    check("the command prints key=value lines", p.stdout.splitlines()[:3], ["state=single", "version_id=v1", f"tag={SHA_A}"])
    check("the command exits 0 for any classification", p.returncode, 0)
    p = subprocess.run([sys.executable, "-I", SCRIPT, os.path.join(d, "nope.json"), vp], capture_output=True, text=True)
    check("a file that is not there is exit 2", p.returncode, 2)
    p = subprocess.run([sys.executable, "-I", SCRIPT], capture_output=True, text=True)
    check("no arguments is exit 2", p.returncode, 2)

print(f"passed={passed} bad={failed}")
sys.exit(1 if failed else 0)
