"""plan-ci.py: which jobs and cells a change needs. Run by ci-scripts.

    test_plan_ci.py [path to a copy of plan-ci.py]

1. Generated crate graphs (300 random DAGs, seeded): every crate changed alone, and random pairs, against an independent
   reverse-reachability written here (a DFS over the reversed edges, not plan-ci.py's fixpoint). The jobs and the cells must be
   exactly what the affected binaries and crates call for. Each graph gets a generated layout (ADR-0009): flat, grouped one or
   two levels down, with directories that share a prefix (`v1`, `v1x`) and a member inside another member's directory, so a
   path must reach the longest member directory that holds it.
2. Union: the plan of a list of paths is the union of the plans of its parts (random subsets of a corpus with one path per
   rule), so no path can hide another.
3. Every rule is reached first by its own sample path (no rule shadowed by an earlier one), and matches a file tracked here.
4. Fail-open: an empty list, an unknown crate directory, a file of crates/ outside a crate, an unknown script, a target the
   planner does not know. A group's README.md, outside every crate, runs only the deps job (the layout guard reads it).
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


ALL_JOBS = {"ci-scripts", "deps", "lint", "deny", "hermetic", "pg-adapter", "contract", "mutants", "web", "docs"}
CELLS = [
    "native-memory main", "native-memory oracle", "native-pg main", "native-pg oracle",
    "worker-memory main", "worker-cf main", "worker-cf oracle", "worker-cf e2e", "worker-cf e2e-mail",
]
SERVER, WORKER, SUITE, PG = "thrust-server", "thrust-worker", "thrust-contract-tests", "thrust-adapters-selfhosted"
BINARY = {"native-memory": SERVER, "native-pg": SERVER, "worker-memory": WORKER, "worker-cf": WORKER}


def flat(deps):
    """The layout before ADR-0009: every crate at crates/<name>."""
    return {n: f"crates/{n}" for n in deps}


def metadata(deps, root="/w", layout=None):
    """cargo metadata (--no-deps) for crates at the layout's directories with these path dependencies, plus one registry
    dependency each. The default layout is this repository's (ADR_0009), and flat for a crate it does not place."""
    layout = layout or {n: ADR_0009.get(n, f"crates/{n}") for n in deps}
    packages = []
    for name, uses in deps.items():
        kinds = ["normal", "dev", "build"]
        packages.append({
            "name": name,
            "id": f"path+file://{root}/{layout[name]}#0.1.0",
            "manifest_path": f"{root}/{layout[name]}/Cargo.toml",
            "dependencies": [{"name": u, "path": f"{root}/{layout[u]}", "kind": kinds[i % 3] if kinds[i % 3] != "normal" else None}
                             for i, u in enumerate(sorted(uses))] + [{"name": "serde", "kind": None}],
        })
    return {"workspace_root": root, "workspace_members": [p["id"] for p in packages], "packages": packages}


def run_plan(paths, deps, cells=CELLS, layout=None):
    dirs, d = pc.workspace(metadata(deps, layout=layout))
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


def random_layout(rng, names):
    """A layout for these crates: some flat, the rest in groups one or two levels down; two directories where one is a prefix of
    the other's name (crates/api/v1, crates/api/v1x); and, half the time, a crate inside another crate's directory (ADR-0009
    forbids it, but the planner must still give a file to the inner one)."""
    groups = ["adapters", "api", "core", "testing/suites", "testing/fixtures"]
    layout = {n: f"crates/{n}" if rng.random() < 0.3 else f"crates/{rng.choice(groups)}/{n}" for n in names}
    a, b = rng.sample(names, 2)
    layout[a], layout[b] = "crates/api/v1", "crates/api/v1x"
    rest = [n for n in names if n not in (a, b)]
    if len(rest) >= 2 and rng.random() < 0.5:
        outer, inner = rng.sample(rest, 2)
        layout[inner] = f"{layout[outer]}/nested"
    return layout


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
    if "thrust-preview-provisioner" in hit:
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
    layout = flat(deps) if g % 3 == 0 else random_layout(rng, names)
    cases = [[n] for n in names] + [rng.sample(names, 2) for _ in range(3)]
    for changed in cases:
        paths = [f"{layout[c]}/src/lib.rs" for c in changed]
        check(f"graph {g}: {changed} changed", run_plan(paths, deps, layout=layout), want_for_crates(set(changed), deps))
    # a manifest also runs deny; a test file of the crate is the crate
    n = rng.choice(names)
    jobs, cells = run_plan([f"{layout[n]}/Cargo.toml"], deps, layout=layout)
    wj, wc = want_for_crates({n}, deps)
    check(f"graph {g}: {n}'s Cargo.toml at {layout[n]}", (jobs, cells), (wj | {"deny"}, wc))
    check(f"graph {g}: {n}'s test file at {layout[n]}", run_plan([f"{layout[n]}/tests/t.rs"], deps, layout=layout),
          want_for_crates({n}, deps))

# ---- 2. union -----------------------------------------------------------------------------------------------------------------
REAL_GRAPH = {
    "thrust-contracts": set(), "thrust-ports": {"thrust-contracts"}, "thrust-core": {"thrust-contracts", "thrust-ports"},
    "thrust-adapters-cloudflare": {"thrust-contracts"}, PG: {"thrust-contracts"}, SUITE: {"thrust-contracts"},
    SERVER: {"thrust-core", PG}, WORKER: {"thrust-core", "thrust-adapters-cloudflare"}, "thrust-editor-doc": set(),
}
# REAL_GRAPH's crates where ADR-0009 puts them, which is where this repository keeps them (section 6 checks it)
ADR_0009 = {
    "thrust-contracts": "crates/core/types", "thrust-ports": "crates/core/ports", "thrust-core": "crates/core/usecases",
    "thrust-adapters-cloudflare": "crates/adapters/cloudflare", PG: "crates/adapters/selfhosted", SUITE: "crates/testing/suites/api",
    SERVER: "crates/server", WORKER: "crates/worker", "thrust-editor-doc": "crates/editor/doc",
}
# one path per rule, in the rules' order; `None` where the rule's sample is a crate path (below)
SAMPLES = [
    "Cargo.lock", ".config/branding/colors.css", ".config/nextest.toml", ".github/workflows/ci.yml", ".github/actions/setup-rust/action.yml",
    ".github/actions/contract-pass-tools/action.yml", ".github/actions/verdict/action.yml", ".github/scripts/assert-gate.sh",
    ".github/scripts/toolchain-channel.sh", "scripts/contract.sh", "crates/core/usecases/src/lib.rs", "infra/preview/provisioner-bootstrap.sql",
    ".github/deploy/flagship/policy.json", ".github/deploy/ca.pem", "deny.toml", "clippy.toml", "release-plz.toml", ".gitignore", "tools/oracle-v1/corpus/a.json",
    "tools/compat-record/record.py", "tools/compat-inventory/out-pinned/endpoint-inventory.csv",
    ".agents/plans/contrail/research/0003-compat-plane-inventory.md", "tools/e2e-har/scrub.py", "e2e/journeys/m1.spec.ts",
    "web/apps/web/app.tsx", "scripts/e2e.sh", "scripts/oracle-v1.sh", "scripts/mutants.sh", "scripts/stage2-contract.sh",
    "scripts/check-web-diff.sh", "scripts/check-book.sh", "scripts/check-topology.sh", "scripts/smoke-deploy.sh", "scripts/flagship.py", "scripts/preview-config.py", "infra/edge/access.tf", "scripts/samply-wrapper.sh", "scripts/verify-all.sh",
    "scripts/new-thing.sh", "docs/src/intro.md", "AGENTS.md", ".github/scripts/lint-workflow.sh", ".github/workflows/deploy.yml",
]
NOTHING = ["README.md", "CHANGELOG.md", ".agents/plans/x.md", "docs-notes.md", "CLAUDE.md", "crates-AGENTS.md", "probes/send-shape/run.sh", "tools/spike-0006/x", ".github/CODEOWNERS"]
check("one sample per rule", len(SAMPLES), len(pc.RULES))
corpus = SAMPLES + NOTHING + ["crates/adapters/cloudflare/src/lib.rs", "crates/server/Cargo.toml", "crates/testing/suites/api/src/lib.rs"]
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
for path in ["crates/gone/src/lib.rs", "crates/adapters/notes.txt", "crates/README.txt", "scripts/new-thing.sh",
             "Cargo.lock", "rust-toolchain.toml", ".github/scripts/plan-ci.py", "scripts/worker-dev.sh",
             ".github/actions/contract-pass-tools/action.yml", ".github/actions/verdict/action.yml", "scripts/lib/ports.sh",
             # a directory that is no crate but shares a prefix with one (crates/core/types): a whole path segment must match
             "crates/core/types-gone/src/lib.rs"]:
    check(f"{path} runs everything", run_plan([path], REAL_GRAPH), EVERYTHING)
for path in ["crates/README.md", "crates/adapters/README.md", "crates/testing/suites/README.md"]:
    check(f"{path}, a group's README inside no crate, runs only deps (the layout guard)", run_plan([path], REAL_GRAPH),
          ({"deps"}, set()))
check(".gitignore runs only deps (the layout guard reads its lines under crates/)", run_plan([".gitignore"], REAL_GRAPH),
      ({"deps"}, set()))
check("a crate's own README.md is the crate", run_plan(["crates/core/usecases/README.md"], REAL_GRAPH),
      run_plan(["crates/core/usecases/src/lib.rs"], REAL_GRAPH))
jobs, cells = run_plan(["README.md"], REAL_GRAPH, CELLS + ["worker-new main"])
check("a target the planner does not know runs even on a change that needs nothing", (jobs, cells), ({"contract"}, {"worker-new main"}))
jobs, cells = run_plan(["crates/adapters/cloudflare/src/lib.rs"], REAL_GRAPH, CELLS + ["worker-new main"])
check("a target the planner does not know runs with a crate change", "worker-new main" in cells, True)

# ---- 5. the named cases ----------------------------------------------------------------------------------------------------------
check("G-30: web/ alone runs the web job and both browser journey cells (the suites share the app, and the mail suite's page is the app's)",
      run_plan(["web/apps/web/app.tsx"], REAL_GRAPH), ({"web", "contract"}, {"worker-cf e2e", "worker-cf e2e-mail"}))
check("e2e/ alone runs only the journeys' cells, both suites'", run_plan(["e2e/journeys/m1.spec.ts"], REAL_GRAPH),
      ({"contract"}, {"worker-cf e2e", "worker-cf e2e-mail"}))
check("a journey of the mail suite alone runs the same two cells (e2e/support/ is shared, so the suites are not told apart)",
      run_plan(["e2e/journeys-mail/m2-sign-in-by-code.spec.ts"], REAL_GRAPH), ({"contract"}, {"worker-cf e2e", "worker-cf e2e-mail"}))
check("a change of the Worker's own crate runs the mail journey's cell with the Worker's other passes",
      "worker-cf e2e-mail" in run_plan(["crates/adapters/cloudflare/src/lib.rs"], REAL_GRAPH)[1], True)
check("a deploy script runs ci-scripts and no cell", run_plan(["scripts/smoke-deploy.sh"], REAL_GRAPH), ({"ci-scripts"}, set()))
check("preview infrastructure runs shell and Postgres checks", run_plan(["infra/preview/provisioner-bootstrap.sql"], REAL_GRAPH),
      ({"ci-scripts", "pg-adapter"}, set()))
check("Cloudflare infrastructure runs shell and Postgres checks", run_plan(["infra/cloudflare/preview-access.py"], REAL_GRAPH),
      ({"ci-scripts", "pg-adapter"}, set()))
check("the edge's Cloudflare root runs ci-scripts and no cell", run_plan(["infra/edge/access.tf"], REAL_GRAPH), ({"ci-scripts"}, set()))
check("the edge's token script runs ci-scripts and no cell", run_plan(["infra/edge/service-token.py"], REAL_GRAPH), ({"ci-scripts"}, set()))
preview_graph = dict(REAL_GRAPH, **{"thrust-preview-provisioner": {"thrust-adapters-cloudflare"}})
preview_layout = dict(ADR_0009, **{"thrust-preview-provisioner": "crates/preview-provisioner"})
preview_jobs, _ = run_plan(["crates/preview-provisioner/src/lib.rs"], preview_graph, layout=preview_layout)
check("preview Worker change runs the Postgres check", "pg-adapter" in preview_jobs, True)
check("another workflow runs ci-scripts and deps, no cell", run_plan([".github/workflows/flake-hunt.yml"], REAL_GRAPH),
      ({"ci-scripts", "deps"}, set()))
check("the book runs only the docs job", run_plan(["docs/src/intro.md"], REAL_GRAPH), ({"docs"}, set()))
check("AGENTS.md, which the book includes, runs the docs job", run_plan(["AGENTS.md"], REAL_GRAPH), ({"docs"}, set()))
check("the book's palette runs the docs job", run_plan([".config/branding/colors.css"], REAL_GRAPH), ({"docs"}, set()))
check("README.md and the plans run nothing", run_plan(["README.md", ".agents/plans/x.md"], REAL_GRAPH), (set(), set()))
check("the oracle's files run its cells and the tests that read them",
      run_plan(["tools/oracle-v1/schema/openapi.v1.json"], REAL_GRAPH),
      ({"hermetic", "deps", "contract"}, {"native-memory oracle", "native-pg oracle", "worker-cf oracle"}))
check("the deploy's CA root runs the TLS tests (hermetic) and ci-scripts", run_plan([".github/deploy/ca.pem"], REAL_GRAPH),
      ({"hermetic", "ci-scripts"}, set()))
jobs, cells = run_plan(["crates/adapters/cloudflare/src/lib.rs"], REAL_GRAPH)
check("a Worker-only crate runs the worker cells only", cells, {c for c in CELLS if c.startswith("worker-")})
check("a Worker-only crate runs no mutants and no pg-adapter", jobs & {"mutants", "pg-adapter"}, set())
jobs, cells = run_plan(["crates/adapters/selfhosted/src/lib.rs"], REAL_GRAPH)
check("the self-hosted adapter runs the native cells, pg-adapter and mutants",
      (cells, {"pg-adapter", "mutants"} <= jobs), ({c for c in CELLS if c.startswith("native-")}, True))
jobs, cells = run_plan(["crates/testing/suites/api/src/lib.rs"], REAL_GRAPH)
check("the suite runs every cell and the mutants", (cells, "mutants" in jobs), (set(CELLS), True))
# The mutation controls' verdicts and the sharing out of their rows are read by mutants.sh (the job) and by test-mutants-verdict.sh and
# test-mutants-shards.sh (ci-scripts), which also read mutants.sh.
for path in ["scripts/mutants.sh", "scripts/lib/mutants-verdict.sh", "scripts/lib/mutants-shard.sh"]:
    jobs, cells = run_plan([path], REAL_GRAPH)
    check(f"{path} runs the mutants job and its test, and no cell", ({"mutants", "ci-scripts"} <= jobs, cells), (True, set()))
jobs, cells = run_plan(["crates/editor/doc/src/lib.rs"], REAL_GRAPH)
check("a crate no binary depends on runs no cell", (cells, "contract" in jobs, "hermetic" in jobs), (set(), False, True))
# ADR-0009's layout plans exactly as the flat one before it did: moving a crate changed no plan.
check("ADR_0009 places every crate of REAL_GRAPH", sorted(ADR_0009), sorted(REAL_GRAPH))
for n in sorted(REAL_GRAPH):
    for f in ("src/lib.rs", "Cargo.toml", "tests/t.rs"):
        check(f"ADR-0009: {n}'s {f} at {ADR_0009[n]} plans as at crates/{n}",
              run_plan([f"{ADR_0009[n]}/{f}"], REAL_GRAPH), run_plan([f"crates/{n}/{f}"], REAL_GRAPH, layout=flat(REAL_GRAPH)))

# ---- 6. this repository ----------------------------------------------------------------------------------------------------------
md = subprocess.run(["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline", "--manifest-path",
                     os.path.join(ROOT, "Cargo.toml")], capture_output=True, text=True)
check("cargo metadata runs here", md.returncode, 0)
if md.returncode == 0:
    dirs, real = pc.workspace(json.loads(md.stdout))
    check("every member directory is under crates/", sorted(d for d in dirs if not d.startswith("crates/")), [])
    check("every member's own file is that member's", sorted(d for d in dirs if pc.owner(f"{d}/src/lib.rs", dirs) != d), [])
    dir_of = {n: d for d, n in dirs.items()}
    check("REAL_GRAPH's crates are where ADR_0009 puts them", {n: dir_of.get(n) for n in ADR_0009}, ADR_0009)
    for name in sorted({pc.SUITE, pc.PG_ADAPTER, pc.MUTANTS_BINARY, *pc.TARGET_BINARY.values()}):
        check(f"{name}, named by the planner, is a crate of the workspace", name in real, True)
    for n in sorted(real):
        jobs, sel, _ = pc.plan([f"{dir_of[n]}/src/lib.rs"], dirs, real, CELLS)
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
def cli(*args, changed=None, cells="\n".join(CELLS) + "\n", md_obj=None, script=None):
    with tempfile.TemporaryDirectory() as d:
        argv = [sys.executable, "-I", script or SCRIPT, "--cells", os.path.join(d, "cells.txt")]
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
check("cli: cells", json.loads(out.get("cells", "null")), ["worker-cf e2e", "worker-cf e2e-mail"])
check("cli: matrix", json.loads(out.get("matrix", "null")),
      {"include": [{"target": "worker-cf", "pass": "e2e"}, {"target": "worker-cf", "pass": "e2e-mail"}]})
rc, out, err = cli(changed="README.md\n")
check("cli: README.md: no job, an empty matrix", (rc, out.get("jobs"), out.get("matrix"), out.get("cells")), (0, "[]", '{"include":[]}', "[]"))
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

# ---- 8. the called workflows (ADR-0102) ------------------------------------------------------------------------------------------
def refused(fn):
    """The exit code of the planner's `fail`, or None when it did not refuse."""
    try:
        fn()
    except SystemExit as e:
        return e.code
    return None


def independent_slices(planned, workflows, jobs):
    """slices() written another way: where each job lives first, then the groups."""
    home = {j: w for w, members in workflows.items() for j in members}
    top = [j for j in jobs if j in planned and j not in home] + [w for w in workflows if set(workflows[w]) & planned]
    called = [w for w in workflows if set(workflows[w]) & planned]
    plans = {w: sorted(j for j in planned if home.get(j) == w) for w in called}
    return top, called, plans


JOBS_LIST = list(pc.JOBS)
# What WORKFLOWS holds today. Each stage of the migration edits this line, so the stage's change to the planner shows in this file,
# and a mutant that empties the map is killed by it.
EXPECTED_WORKFLOWS = {"documentation": ("docs",), "security": ("deny",), "web": ("web",), "lint": ("deps", "lint"),
                      "test": ("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants")}
check("WORKFLOWS is a partition of the jobs", refused(lambda: pc.check_workflows()), None)
check("WORKFLOWS holds the workflows moved so far, and their jobs", pc.WORKFLOWS, EXPECTED_WORKFLOWS)


def normal(slices):
    """slices() with each plan sorted, to compare with independent_slices()."""
    top, called, plans = slices
    return top, called, {w: sorted(j) for w, j in plans.items()}


for _ in range(200):
    planned = set(rng.sample(JOBS_LIST, rng.randint(0, len(JOBS_LIST))))
    check(f"no workflow: top is the planned jobs in order, no workflow, no plan ({sorted(planned)})", pc.slices(planned, {}),
          ([j for j in JOBS_LIST if j in planned], [], {}))
    check(f"the real map: top, the called workflows and their plans ({sorted(planned)})", normal(pc.slices(planned)),
          independent_slices(planned, EXPECTED_WORKFLOWS, JOBS_LIST))
# generated partitions: each job stays in ci.yml or goes into one workflow; the planned jobs are random. The workflows' names here
# are not job names, so that a name in `top` is one thing or the other (the real clash, a workflow `web` holding a job `web`, is below)
NAMES = ["wf-test", "wf-lint", "wf-documentation", "wf-security", "wf-web"]
for g in range(300):
    chosen = rng.sample(NAMES, rng.randint(0, len(NAMES)))
    workflows = {w: [] for w in chosen}
    for job in JOBS_LIST:
        if chosen and rng.random() < 0.7:
            workflows[rng.choice(chosen)].append(job)
    workflows = {w: tuple(m) for w, m in workflows.items() if m}
    planned = set(rng.sample(JOBS_LIST, rng.randint(0, len(JOBS_LIST))))
    top, called, plans = pc.slices(planned, workflows)
    wtop, wcalled, wplans = independent_slices(planned, workflows, JOBS_LIST)
    check(f"partition {g}: top", top, wtop)
    check(f"partition {g}: the called workflows", called, wcalled)
    check(f"partition {g}: each plan holds its own planned jobs", {w: sorted(j) for w, j in plans.items()}, wplans)
    # every planned job is judged exactly once: by ci.yml's gate as itself, or by the `verdict` job of its one workflow
    inside = [j for w in plans for j in plans[w]]
    check(f"partition {g}: no planned job is judged twice or not at all", sorted(top[:len(top) - len(called)] + inside), sorted(planned))
    check(f"partition {g}: a called workflow with no planned job is not asked for", sorted(set(top) & set(workflows) - set(called)), [])
# the real clash of names: the workflow `web` holds the job `web`, and `lint` the job `lint` while `deny` stays in ci.yml
check("a workflow and its own job share a name: top has the calling job once", pc.slices({"web", "deny"}, {"web": ("web",)}),
      (["deny", "web"], ["web"], {"web": ["web"]}))
check("a workflow whose job is not planned is not asked for, though an inline job has its name",
      pc.slices({"lint"}, {"lint": ("deny",)}), (["lint"], [], {}))
# a map that is not a partition is refused (exit 2), never half-used
check("a job in two workflows is refused", refused(lambda: pc.check_workflows({"test": ("lint", "docs"), "web": ("docs",)})), 2)
check("a name that is not a job is refused", refused(lambda: pc.check_workflows({"test": ("hermetic", "nonesuch")})), 2)
check("a workflow with no job is refused", refused(lambda: pc.check_workflows({"test": ()})), 2)
check("slices refuses a map that is not a partition", refused(lambda: pc.slices({"lint"}, {"test": ("lint",), "web": ("lint",)})), 2)

# ---- 9. the workflow files (ADR-0102) ---------------------------------------------------------------------------------------------
# The files that run the checks `gate` waits for, today and as the migration adds them; and the ones that run no check, because they
# have triggers of their own (the deploy and release flows, the schedules).
CHECK_WORKFLOWS = ["ci", "contract-cell", "test", "lint", "documentation", "security", "web"]
NO_CHECK_WORKFLOWS = ["deploy", "e2e", "flagship", "flake-hunt", "release", "release-plz", "cloudflare-infrastructure", "edge-infrastructure", "deploy-preview-provisioner", "enable-preview-provisioner", "preview-reconcile", "preview-migration-source"]
for name in CHECK_WORKFLOWS:
    check(f".github/workflows/{name}.yml runs everything", run_plan([f".github/workflows/{name}.yml"], REAL_GRAPH), EVERYTHING)
for name in NO_CHECK_WORKFLOWS:
    check(f".github/workflows/{name}.yml runs only its helper tests and the workflow lint",
          run_plan([f".github/workflows/{name}.yml"], REAL_GRAPH), ({"ci-scripts", "deps"}, set()))
check("a workflow named like a check without being one (testing.yml) is not run-everything",
      run_plan([".github/workflows/testing.yml"], REAL_GRAPH), ({"ci-scripts", "deps"}, set()))
check("the names of the two lists are disjoint", sorted(set(CHECK_WORKFLOWS) & set(NO_CHECK_WORKFLOWS)), [])
workflow_dir = os.path.join(ROOT, ".github", "workflows")
real_files = sorted(f[:-4] for f in os.listdir(workflow_dir) if f.endswith(".yml"))
check("this repository has workflow files", len(real_files) >= 8, True)
check("every workflow file of this repository is one that runs the checks or one that runs none: add a new file to a list here, "
      "and to the planner's rule when it runs the checks", [n for n in real_files if n not in CHECK_WORKFLOWS + NO_CHECK_WORKFLOWS], [])
check("every file that runs the checks is planned as such, whatever the lists say",
      [n for n in real_files if n in CHECK_WORKFLOWS and run_plan([f".github/workflows/{n}.yml"], REAL_GRAPH) != EVERYTHING], [])

# The two indexes are read by check-ci-index.sh, so a change to one runs that guard (deps) and its test (ci-scripts), which is what the
# generic rule for .github/ gives; this keeps a README from ever being a file that runs nothing.
for index in (".github/workflows/README.md", ".github/actions/README.md", ".github/scripts/check-ci-index.sh"):
    check(f"{index} runs the index guard (deps) and its test (ci-scripts)", run_plan([index], REAL_GRAPH), ({"ci-scripts", "deps"}, set()))

# ---- 10. the command line's new lines --------------------------------------------------------------------------------------------
rc, out, _ = cli(changed="web/x.tsx\n")
check("cli: a change to web/ plans the web workflow, and the test workflow with the job contract alone (the browser journeys' cells)",
      (json.loads(out.get("top", "null")), json.loads(out.get("workflows", "null")), json.loads(out.get("plans", "null"))),
      (["web", "test"], ["web", "test"], {"web": ["web"], "test": ["contract"]}))
check("cli: and `jobs` still names the jobs `contract` and `web`, which now live in workflows",
      json.loads(out.get("jobs", "null")), ["contract", "web"])
rc, out, _ = cli(changed="e2e/journeys/m1.spec.ts\n")
check("cli: a change to e2e/ alone plans the test workflow with the job contract alone, and no web workflow (the app is not rebuilt)",
      (json.loads(out.get("top", "null")), json.loads(out.get("workflows", "null")), json.loads(out.get("plans", "null"))),
      (["test"], ["test"], {"test": ["contract"]}))
check("cli: the cells themselves are still planned, for the test workflow to be given: `cells` and `matrix` name them",
      (json.loads(out.get("cells", "null")), len(json.loads(out.get("matrix", "null"))["include"])), (["worker-cf e2e", "worker-cf e2e-mail"], 2))
rc, out, _ = cli(changed="scripts/check-web-diff.sh\n")
check("cli: the Q-R2 gate's script plans the workflow web, and its own test (the job ci-scripts of the workflow test)",
      (json.loads(out.get("top", "null")), json.loads(out.get("plans", "null"))), (["web", "test"], {"web": ["web"], "test": ["ci-scripts"]}))
rc, out, _ = cli(changed="scripts/smoke-deploy.sh\n")
check("cli: a deploy script plans the workflow test with the job ci-scripts alone",
      (json.loads(out.get("top", "null")), json.loads(out.get("plans", "null")), json.loads(out.get("jobs", "null"))),
      (["test"], {"test": ["ci-scripts"]}, ["ci-scripts"]))
rc, out, _ = cli(changed=".github/deploy/ca.pem\n")
check("cli: the deploy's CA root plans the workflow test with hermetic and ci-scripts, in the workflow's order",
      json.loads(out.get("plans", "null")), {"test": ["ci-scripts", "hermetic"]})
rc, out, _ = cli(changed="README.md\n")
check("cli: README.md: nothing to expect of ci.yml", (out.get("top"), out.get("workflows"), out.get("plans")), ("[]", "[]", "{}"))
rc, out, _ = cli("--full", "a probe")
check("cli: --full: top is every job still in ci.yml, then every called workflow",
      json.loads(out.get("top", "null")), independent_slices(set(JOBS_LIST), EXPECTED_WORKFLOWS, JOBS_LIST)[0])
check("cli: --full: jobs is still every job, moved or not", json.loads(out.get("jobs", "null")), JOBS_LIST)
# the real map, through the real command line
rc, out, _ = cli(changed="docs/src/intro.md\n")
check("cli: the book alone plans the documentation workflow and no job of ci.yml",
      (json.loads(out.get("top", "null")), json.loads(out.get("workflows", "null")), json.loads(out.get("plans", "null"))),
      (["documentation"], ["documentation"], {"documentation": ["docs"]}))
check("cli: the book alone: jobs still names the job `docs`", json.loads(out.get("jobs", "null")), ["docs"])
rc, out, _ = cli(changed="deny.toml\n")
check("cli: deny.toml alone plans the security workflow and no job of ci.yml",
      (json.loads(out.get("top", "null")), json.loads(out.get("workflows", "null")), json.loads(out.get("plans", "null"))),
      (["security"], ["security"], {"security": ["deny"]}))
check("cli: deny.toml alone: jobs still names the job `deny`", json.loads(out.get("jobs", "null")), ["deny"])
rc, out, _ = cli(changed="crates/server/Cargo.toml\n")
check("cli: a crate's Cargo.toml plans the security workflow too, the lint workflow with both its jobs, and the test workflow, after the jobs still in ci.yml",
      (json.loads(out.get("top", "null"))[-3:], json.loads(out.get("plans", "null")).get("security"),
       json.loads(out.get("plans", "null")).get("lint")), (["security", "lint", "test"], ["deny"], ["deps", "lint"]))
# a workflow of two jobs can have a partial plan: only the job the change needs, and `verdict` expects the other to be skipped
rc, out, _ = cli(changed="clippy.toml\n")
check("cli: clippy.toml plans the lint workflow with the job `lint` alone",
      (json.loads(out.get("top", "null")), json.loads(out.get("plans", "null")), json.loads(out.get("jobs", "null"))),
      (["lint"], {"lint": ["lint"]}, ["lint"]))
rc, out, _ = cli(changed="scripts/check-layout.sh\n")
check("cli: a guard of deps plans the lint workflow with the job `deps` alone, and the job `ci-scripts` (its test) of the test workflow",
      (json.loads(out.get("top", "null")), json.loads(out.get("plans", "null"))), (["lint", "test"], {"lint": ["deps"], "test": ["ci-scripts"]}))
rc, out, _ = cli(changed="clippy.toml\nscripts/check-layout.sh\n")
check("cli: both together plan the two jobs of lint, in the order of the workflow (deps first)",
      json.loads(out.get("plans", "null")).get("lint"), ["deps", "lint"])
rc, out, _ = cli(changed="docs/src/intro.md\ndeny.toml\n")
check("cli: the book and deny.toml: both workflows, each with its own job",
      (json.loads(out.get("top", "null")), json.loads(out.get("plans", "null"))),
      (["documentation", "security"], {"documentation": ["docs"], "security": ["deny"]}))
rc, out, _ = cli(changed=".github/actions/verdict/action.yml\n")
check("cli: a change to the verdict action runs everything, so every called workflow is called",
      json.loads(out.get("workflows", "null")), ["documentation", "security", "web", "lint", "test"])
rc, out, _ = cli(changed="docs/src/intro.md\nweb/x.tsx\n")
check("cli: the book and web/: the workflows, in the planner's order (no job is left in ci.yml)",
      json.loads(out.get("top", "null")), ["documentation", "web", "test"])
# Run the same command line on copies of the planner whose WORKFLOWS holds other maps, so that `plans` and the refusals are shown
# against more than the one workflow of the real map.
MAP_LINE = r"\nWORKFLOWS = \{.*\}\n"
with open(SCRIPT, encoding="utf-8") as f:
    source = f.read()
check("the WORKFLOWS line is found once in the planner, to be replaced in the copies", len(re.findall(MAP_LINE, source)), 1)


def with_map(text):
    """The planner's source with its WORKFLOWS line replaced."""
    return re.sub(MAP_LINE, lambda _: f"\nWORKFLOWS = {text}\n", source, count=1)


with tempfile.TemporaryDirectory() as d:
    moved = os.path.join(d, "plan-ci-with-workflows.py")
    with open(moved, "w", encoding="utf-8") as f:
        f.write(with_map('{"wf-test": ("hermetic", "pg-adapter"), "wf-docs": ("docs",)}'))
    rc, out, _ = cli("--full", "a probe", script=moved)
    check("cli, two workflows: --full: top is the jobs left in ci.yml, then the two workflows",
          json.loads(out.get("top", "null")), [j for j in JOBS_LIST if j not in ("hermetic", "pg-adapter", "docs")] + ["wf-test", "wf-docs"])
    check("cli, two workflows: --full: both are called", json.loads(out.get("workflows", "null")), ["wf-test", "wf-docs"])
    check("cli, two workflows: --full: each is given its jobs as its plan", json.loads(out.get("plans", "null")),
          {"wf-test": ["hermetic", "pg-adapter"], "wf-docs": ["docs"]})
    check("cli, two workflows: --full: jobs still lists every planned job, inside a workflow or not", json.loads(out.get("jobs", "null")), JOBS_LIST)
    rc, out, _ = cli(changed="docs/src/intro.md\n", script=moved)
    check("cli, two workflows: the book alone: only the docs workflow", (json.loads(out.get("top", "null")), json.loads(out.get("workflows", "null")),
          json.loads(out.get("plans", "null"))), (["wf-docs"], ["wf-docs"], {"wf-docs": ["docs"]}))
    rc, out, _ = cli(changed="README.md\n", script=moved)
    check("cli, two workflows: a change that needs nothing calls nothing", (out.get("top"), out.get("workflows"), out.get("plans")), ("[]", "[]", "{}"))
    bad_map = os.path.join(d, "plan-ci-bad-map.py")
    with open(bad_map, "w", encoding="utf-8") as f:
        f.write(with_map('{"a": ("docs",), "b": ("docs",)}'))
    rc, out, err = cli("--full", "a probe", script=bad_map)
    check("cli: a map with a job in two workflows is refused (exit 2), says which, and prints nothing", (rc, out, "docs is in two workflows" in err),
          (2, {}, True))

print(f"passed={passed} bad={failed}")
sys.exit(1 if failed else 0)
