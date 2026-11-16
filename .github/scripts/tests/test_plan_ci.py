"""plan-ci.py: which jobs and cells a change needs. Run by ci-scripts.

    test_plan_ci.py [path to a copy of plan-ci.py]

1. Generated crate graphs (300 random DAGs, seeded): every crate changed alone, and random pairs, against an independent
   reverse-reachability written here (a DFS over the reversed edges, not plan-ci.py's fixpoint). The jobs and the cells must be
   exactly what the affected binaries and crates call for.
2. Union: the plan of a list of paths is the union of the plans of its parts (random subsets of a corpus with one path per
   rule), so no path can hide another.
3. Every rule is reached first by its own sample path (no rule shadowed by an earlier one), and matches a file tracked here.
4. Fail-open: an empty list, an unknown crate directory, a file of crates/ outside a crate, an unknown script, a target the
   planner does not know.
5. The cases the plan and the gaps name (G-30: web/ alone runs the browser journeys; a deploy script runs no cell; docs run
   nothing).
6. This repository: Cargo's real graph, every target of scripts/contract-all.sh known, the named crates real, and no script the
   planner keeps out of the cells is called by what runs a cell.
7. The command line: its output, --full, and the inputs it refuses (exit 2).
A path argument points it at a deliberately broken copy, to show this test fails.
"""

import importlib.util
import json
import os
import random
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
SCRIPT = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, "..", "plan-ci.py"))

spec = importlib.util.spec_from_file_location("plan_ci", SCRIPT)
pc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pc)

passed = 0
failed = 0


def check(label, got, want):
    global passed, failed
    if got == want:
        passed += 1
    else:
        failed += 1
        print(f"BAD  {label}: got {got!r}, wanted {want!r}")


ALL_JOBS = {"ci-scripts", "deps", "lint", "deny", "hermetic", "pg-adapter", "contract", "mutants", "web"}
CELLS = [
    "native-memory main", "native-memory oracle", "native-pg main", "native-pg oracle",
    "worker-memory main", "worker-cf main", "worker-cf oracle", "worker-cf e2e",
]
SERVER, WORKER, SUITE, PG = "thrust-server", "thrust-worker", "thrust-contract-tests", "thrust-adapters-selfhosted"
BINARY = {"native-memory": SERVER, "native-pg": SERVER, "worker-memory": WORKER, "worker-cf": WORKER}


def metadata(deps, root="/w"):
    """cargo metadata (--no-deps) for crates/<name> crates with these path dependencies, plus one registry dependency each."""
    packages = []
    for name, uses in deps.items():
        kinds = ["normal", "dev", "build"]
        packages.append({
            "name": name,
            "id": f"path+file://{root}/crates/{name}#0.1.0",
            "manifest_path": f"{root}/crates/{name}/Cargo.toml",
            "dependencies": [{"name": u, "path": f"{root}/crates/{u}", "kind": kinds[i % 3] if kinds[i % 3] != "normal" else None}
                             for i, u in enumerate(sorted(uses))] + [{"name": "serde", "kind": None}],
        })
    return {"workspace_root": root, "workspace_members": [p["id"] for p in packages], "packages": packages}


def run_plan(paths, deps, cells=CELLS):
    dirs, d = pc.workspace(metadata(deps))
    jobs, selected, _ = pc.plan(paths, dirs, d, cells)
    return set(jobs), set(selected)


# ---- 1. generated graphs, against an independent reverse reachability ------------------------------------------------------------
def random_graph(rng):
    names = [SERVER, WORKER, SUITE, PG] + [f"thrust-c{i}" for i in range(rng.randint(2, 9))]
    rng.shuffle(names)  # the order is the topological order: a crate depends only on crates before it
    deps = {}
    for i, n in enumerate(names):
        deps[n] = {m for m in names[:i] if rng.random() < 0.3}
    return deps


def reached_from(changed, deps):
    reverse = {n: set() for n in deps}
    for n, uses in deps.items():
        for u in uses:
            reverse[u].add(n)
    seen = set()

    def dfs(n):
        if n in seen:
            return
        seen.add(n)
        for m in reverse[n]:
            dfs(m)

    for c in changed:
        dfs(c)
    return seen


def want_for_crates(changed, deps):
    hit = reached_from(changed, deps)
    jobs = {"ci-scripts", "deps", "lint", "hermetic"}
    if PG in hit:
        jobs.add("pg-adapter")
    if SERVER in hit or SUITE in changed:
        jobs.add("mutants")
    cells = {c for c in CELLS if BINARY[c.split(" ")[0]] in hit or SUITE in changed}
    if cells:
        jobs.add("contract")
    return jobs, cells


rng = random.Random(20261116)
for g in range(300):
    deps = random_graph(rng)
    names = sorted(deps)
    cases = [[n] for n in names] + [rng.sample(names, 2) for _ in range(3)]
    for changed in cases:
        paths = [f"crates/{c}/src/lib.rs" for c in changed]
        check(f"graph {g}: {changed} changed", run_plan(paths, deps), want_for_crates(set(changed), deps))
    # a manifest also runs deny; a test file of the crate is the crate
    n = rng.choice(names)
    jobs, cells = run_plan([f"crates/{n}/Cargo.toml"], deps)
    wj, wc = want_for_crates({n}, deps)
    check(f"graph {g}: {n}'s Cargo.toml", (jobs, cells), (wj | {"deny"}, wc))
    check(f"graph {g}: {n}'s test file", run_plan([f"crates/{n}/tests/t.rs"], deps), want_for_crates({n}, deps))

# ---- 2. union -----------------------------------------------------------------------------------------------------------------
REAL_GRAPH = {
    "thrust-contracts": set(), "thrust-ports": {"thrust-contracts"}, "thrust-core": {"thrust-contracts", "thrust-ports"},
    "thrust-adapters-cloudflare": {"thrust-contracts"}, PG: {"thrust-contracts"}, SUITE: {"thrust-contracts"},
    SERVER: {"thrust-core", PG}, WORKER: {"thrust-core", "thrust-adapters-cloudflare"}, "thrust-editor-doc": set(),
}
# one path per rule, in the rules' order; `None` where the rule's sample is a crate path (below)
SAMPLES = [
    "Cargo.lock", ".config/branding/colors.css", ".config/nextest.toml", ".github/workflows/ci.yml", ".github/actions/setup-rust/action.yml",
    ".github/scripts/assert-gate.sh", ".github/scripts/toolchain-channel.sh", "scripts/contract.sh", "crates/thrust-core/src/lib.rs", "crates/README.md",
    ".github/deploy/ca.pem", "deny.toml", "clippy.toml", "release-plz.toml", "tools/oracle-v1/corpus/a.json",
    "tools/compat-record/record.py", "tools/compat-inventory/out-pinned/endpoint-inventory.csv",
    ".agents/plans/contrail/research/0003-compat-plane-inventory.md", "tools/e2e-har/scrub.py", "e2e/journeys/m1.spec.ts",
    "web/apps/web/app.tsx", "scripts/e2e.sh", "scripts/oracle-v1.sh", "scripts/mutants.sh", "scripts/stage2-contract.sh",
    "scripts/check-web-diff.sh", "scripts/check-topology.sh", "scripts/smoke-deploy.sh", "scripts/verify-all.sh",
    "scripts/new-thing.sh", ".github/scripts/lint-workflow.sh", ".github/workflows/deploy.yml",
]
NOTHING = ["docs/src/intro.md", "README.md", "CHANGELOG.md", "probes/send-shape/run.sh", "tools/spike-0006/x", ".github/CODEOWNERS"]
check("one sample per rule", len(SAMPLES), len(pc.RULES))
corpus = SAMPLES + NOTHING + ["crates/thrust-adapters-cloudflare/src/lib.rs", "crates/thrust-server/Cargo.toml", "crates/thrust-contract-tests/src/lib.rs"]
for i in range(400):
    a = rng.sample(corpus, rng.randint(1, 4))
    b = rng.sample(corpus, rng.randint(1, 4))
    ja, ca = run_plan(a, REAL_GRAPH)
    jb, cb = run_plan(b, REAL_GRAPH)
    check(f"union {i}: {a} + {b}", run_plan(a + b, REAL_GRAPH), (ja | jb, ca | cb))

# ---- 3. every rule first reached by its own sample, and matching a tracked file ----------------------------------------------------
for i, path in enumerate(SAMPLES):
    first = next((k for k, (p, _, _) in enumerate(pc.COMPILED) if p.search(path)), None)
    check(f"rule {i} ({pc.RULES[i][0]}) is the first match of {path}", first, i)
for path in NOTHING:
    check(f"{path} matches no rule", [p.pattern for p, _, _ in pc.COMPILED if p.search(path)], [])
tracked = subprocess.run(["git", "-C", ROOT, "ls-files"], capture_output=True, text=True, check=True).stdout.split("\n")
check("git ls-files lists this repository", len(tracked) > 100, True)
for pattern, _, why in pc.RULES:
    if pattern in (r"^crates/", r"^scripts/"):
        continue  # the catch-alls: they exist for files that are not there yet
    check(f"the rule for {why} ({pattern}) matches a tracked file", any(re.search(pattern, f) for f in tracked), True)

# ---- 4. fail open --------------------------------------------------------------------------------------------------------------
EVERYTHING = (ALL_JOBS, set(CELLS))
for path in ["crates/thrust-gone/src/lib.rs", "crates/README.md", "scripts/new-thing.sh", "Cargo.lock", "rust-toolchain.toml",
             ".github/scripts/plan-ci.py", "scripts/worker-dev.sh"]:
    check(f"{path} runs everything", run_plan([path], REAL_GRAPH), EVERYTHING)
jobs, cells = run_plan(["docs/a.md"], REAL_GRAPH, CELLS + ["worker-new main"])
check("a target the planner does not know runs even on docs", (jobs, cells), ({"contract"}, {"worker-new main"}))
jobs, cells = run_plan(["crates/thrust-adapters-cloudflare/src/lib.rs"], REAL_GRAPH, CELLS + ["worker-new main"])
check("a target the planner does not know runs with a crate change", "worker-new main" in cells, True)

# ---- 5. the named cases ----------------------------------------------------------------------------------------------------------
check("G-30: web/ alone runs the web job and the browser journeys' cell",
      run_plan(["web/apps/web/app.tsx"], REAL_GRAPH), ({"web", "contract"}, {"worker-cf e2e"}))
check("e2e/ alone runs only the journeys' cell", run_plan(["e2e/journeys/m1.spec.ts"], REAL_GRAPH), ({"contract"}, {"worker-cf e2e"}))
check("a deploy script runs ci-scripts and no cell", run_plan(["scripts/smoke-deploy.sh"], REAL_GRAPH), ({"ci-scripts"}, set()))
check("another workflow runs ci-scripts and deps, no cell", run_plan([".github/workflows/flake-hunt.yml"], REAL_GRAPH),
      ({"ci-scripts", "deps"}, set()))
check("docs run nothing", run_plan(["docs/src/intro.md", "README.md"], REAL_GRAPH), (set(), set()))
check("the oracle's files run its cells and the tests that read them",
      run_plan(["tools/oracle-v1/schema/openapi.v1.json"], REAL_GRAPH),
      ({"hermetic", "deps", "contract"}, {"native-memory oracle", "native-pg oracle", "worker-cf oracle"}))
check("the deploy's CA root runs the TLS tests (hermetic) and ci-scripts", run_plan([".github/deploy/ca.pem"], REAL_GRAPH),
      ({"hermetic", "ci-scripts"}, set()))
jobs, cells = run_plan(["crates/thrust-adapters-cloudflare/src/lib.rs"], REAL_GRAPH)
check("a Worker-only crate runs the worker cells only", cells, {c for c in CELLS if c.startswith("worker-")})
check("a Worker-only crate runs no mutants and no pg-adapter", jobs & {"mutants", "pg-adapter"}, set())
jobs, cells = run_plan(["crates/thrust-adapters-selfhosted/src/lib.rs"], REAL_GRAPH)
check("the self-hosted adapter runs the native cells, pg-adapter and mutants",
      (cells, {"pg-adapter", "mutants"} <= jobs), ({c for c in CELLS if c.startswith("native-")}, True))
jobs, cells = run_plan(["crates/thrust-contract-tests/src/lib.rs"], REAL_GRAPH)
check("the suite runs every cell and the mutants", (cells, "mutants" in jobs), (set(CELLS), True))
jobs, cells = run_plan(["crates/thrust-editor-doc/src/lib.rs"], REAL_GRAPH)
check("a crate no binary depends on runs no cell", (cells, "contract" in jobs, "hermetic" in jobs), (set(), False, True))

# ---- 6. this repository ----------------------------------------------------------------------------------------------------------
md = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline", "--manifest-path",
                     os.path.join(ROOT, "Cargo.toml")], capture_output=True, text=True)
check("cargo metadata runs here", md.returncode, 0)
if md.returncode == 0:
    dirs, real = pc.workspace(json.loads(md.stdout))
    check("every crate directory is crates/<name>", sorted(dirs), sorted(f"crates/{n}" for n in real))
    for name in sorted({pc.SUITE, pc.PG_ADAPTER, pc.MUTANTS_BINARY, *pc.TARGET_BINARY.values()}):
        check(f"{name}, named by the planner, is a crate of the workspace", name in real, True)
    for n in sorted(real):
        jobs, sel, _ = pc.plan([f"crates/{n}/src/lib.rs"], dirs, real, CELLS)
        check(f"this repository: {n} changed", (set(jobs), set(sel)), want_for_crates({n}, real))
with open(os.path.join(ROOT, "scripts", "contract-all.sh"), encoding="utf-8") as f:
    targets = {line.split()[1] for line in f if re.match(r"^run [a-z0-9-]+ [a-z0-9-]+\s*$", line)}
check("scripts/contract-all.sh has targets", len(targets) >= 4, True)
for t in sorted(targets):
    check(f"target {t} of scripts/contract-all.sh is known to the planner", t in pc.TARGET_BINARY, True)
# What runs a cell must call no script that the planner keeps out of the cells.
# (Comment lines are left out: a comment that names a script does not call it.)
HARNESS = [".github/workflows/contract-cell.yml", "scripts/contract.sh", "scripts/worker-dev.sh", "scripts/e2e.sh",
           "scripts/oracle-v1.sh", "scripts/oracle-v1-reset.sh"]
harness_text = ""
for h in HARNESS:
    with open(os.path.join(ROOT, h), encoding="utf-8") as f:
        harness_text += "".join(line for line in f if not line.lstrip().startswith("#"))
for script in sorted(f for f in tracked if f.startswith("scripts/") and "/" not in f[len("scripts/"):]):
    jobs, cells = run_plan([script], REAL_GRAPH)
    if cells:
        continue
    name = os.path.basename(script)
    check(f"{script} runs no cell, so nothing that runs a cell calls it", re.search(re.escape(name) + r"\b", harness_text) is None, True)

# ---- 7. the command line ---------------------------------------------------------------------------------------------------------
def cli(*args, changed=None, cells="\n".join(CELLS) + "\n", md_obj=None):
    with tempfile.TemporaryDirectory() as d:
        argv = [sys.executable, "-I", SCRIPT, "--cells", os.path.join(d, "cells.txt")]
        with open(os.path.join(d, "cells.txt"), "w") as f:
            f.write(cells)
        if changed is not None:
            with open(os.path.join(d, "changed.txt"), "w") as f:
                f.write(changed)
            with open(os.path.join(d, "md.json"), "w") as f:
                json.dump(md_obj or metadata(REAL_GRAPH), f)
            argv += ["--changed", os.path.join(d, "changed.txt"), "--metadata", os.path.join(d, "md.json")]
        p = subprocess.run(argv + list(args), capture_output=True, text=True)
        out = dict(line.split("=", 1) for line in p.stdout.splitlines() if "=" in line)
        return p.returncode, out, p.stderr


rc, out, _ = cli(changed="web/x.tsx\n")
check("cli: exit 0", rc, 0)
check("cli: jobs, in the gate's order", json.loads(out.get("jobs", "null")), ["contract", "web"])
check("cli: cells", json.loads(out.get("cells", "null")), ["worker-cf e2e"])
check("cli: matrix", json.loads(out.get("matrix", "null")), {"include": [{"target": "worker-cf", "pass": "e2e"}]})
rc, out, err = cli(changed="docs/a.md\n")
check("cli: docs: no job, an empty matrix", (rc, out.get("jobs"), out.get("matrix"), out.get("cells")), (0, "[]", '{"include":[]}', "[]"))
rc, out, err = cli(changed="")
check("cli: an empty change list runs everything", (rc, set(json.loads(out.get("jobs", "[]"))), len(json.loads(out.get("cells", "[]")))),
      (0, ALL_JOBS, len(CELLS)))
check("cli: and says why", "the change list is empty" in err, True)
rc, out, err = cli("--full", "a release candidate")
check("cli: --full runs everything", (rc, set(json.loads(out.get("jobs", "[]"))), json.loads(out.get("cells", "[]"))),
      (0, ALL_JOBS, CELLS))
check("cli: --full says why", "a release candidate" in err, True)
check("cli: no --changed and no --full is refused", cli()[0], 2)
check("cli: an empty cell list is refused", cli("--full", "x", cells="")[0], 2)
check("cli: a cell that is not '<target> <pass>' is refused", cli("--full", "x", cells="native-memory\n")[0], 2)
check("cli: metadata with no member is refused", cli(changed="a\n", md_obj={"workspace_root": "/w", "workspace_members": [], "packages": []})[0], 2)
bad = metadata(REAL_GRAPH)
bad["packages"][0]["manifest_path"] = "/elsewhere/x/Cargo.toml"
check("cli: a crate outside the workspace root is refused", cli(changed="a\n", md_obj=bad)[0], 2)

print(f"passed={passed} bad={failed}")
sys.exit(1 if failed else 0)
