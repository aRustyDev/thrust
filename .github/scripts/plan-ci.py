#!/usr/bin/env python3
"""Which jobs of ci.yml, and which cells of the contract matrix, a change needs. Run by the `changes` job.

    python3 -I .github/scripts/plan-ci.py --changed changed.txt --metadata metadata.json --cells cells.txt
    python3 -I .github/scripts/plan-ci.py --full "<why>" --cells cells.txt

  --changed   the changed paths, one per line (`git diff --name-only <base> <head>`)
  --metadata  `cargo metadata --format-version 1 --no-deps`: the workspace's crates, their directories and their dependencies
  --cells     the contract matrix, one "<target> <pass>" per line (cells.txt, written by plan-cells.sh from scripts/contract-all.sh)
  --full      run everything, for the reason given (no usable base, a release candidate, a merge queue, the nightly run)

It prints, for $GITHUB_OUTPUT:
  jobs=<JSON list of the jobs that must run>   the gate (assert-gate.sh) requires each to succeed, and every other to be skipped
  matrix=<{"include":[{"target":..,"pass":..}]}> and cells=<JSON list of "<target> <pass>">   the cells that must run
  top=<JSON list>   what ci.yml's own gate expects (ADR-0102): the planned jobs that are still jobs of ci.yml itself, then the
                    called workflows that have a planned job
  workflows=<JSON list of the called workflows that have a planned job> and plans=<{"workflow": [its planned jobs]}>   each
                    called workflow's `plan` input, which its `verdict` job judges it by
and, on stderr, why: each path with the rule it matched.

A job is a job of ci.yml itself or of one called workflow (WORKFLOWS, which each stage of the migration fills in). `jobs` is still
every planned job, wherever it lives; `top` names a moved job's workflow instead (a change to docs/ plans the job `docs`, and
`top` the workflow `documentation`, with `plans` = {"documentation": ["docs"]}).

How a path decides (RULES, first match wins; a path that matches no rule needs nothing, as README.md and .agents/ do):
  * a crate's own files (the files under a member's directory, at any depth under crates/: the longest member directory that
    holds the path) change that crate. The crates that depend on it, through any kind of dependency (normal, dev, build), are
    affected too, read from `cargo metadata`, so the graph is Cargo's and not a second list. A group's README.md under
    crates/ that no member holds is read only by the layout guard (deps); any other file of crates/ outside a member runs
    everything.
  * a file a crate's tests read from outside the crate (tools/oracle-v1/, .github/deploy/) runs the jobs that run those tests.
  * a file that only a guard, a job or a pass reads runs that guard's job, or that pass on every target.
  * a file whose readers are not known runs everything: an unknown crate directory, an unlisted script, the toolchain, the
    lockfile, the workflow and actions that run the matrix, and this planner and its gate.

Then, from the changed crates C and the affected crates A (C and everything that depends on it):
  * C not empty: ci-scripts, deps, lint and hermetic (the whole workspace: they build it all anyway).
  * a crate's Cargo.toml: deny.
  * the target of a cell runs its passes when the binary it serves is in A (native-* serve thrust-server, worker-* serve
    thrust-worker), or when the suite itself (thrust-contract-tests) changed.
  * pg-adapter when thrust-adapters-selfhosted is in A; mutants (built on the native server) when thrust-server is in A or the
    suite changed.

Fails open: a target the map below does not know runs on every change (and says so); `--full` runs everything.
"""

import argparse
import json
import re
import sys

JOBS = ("ci-scripts", "deps", "lint", "deny", "hermetic", "pg-adapter", "contract", "mutants", "web", "docs")

# The called workflow each job belongs to (ADR-0102): {workflow: (its jobs)}. A job in none is still a job of ci.yml itself. Every
# migration stage moves its jobs in here; a job may be in one workflow only, and a name here must be one of JOBS (check_workflows).
# ci.yml's `gate` lists these workflows too, as VERDICT_JOBS, and test_plan_ci.py holds the two equal.
WORKFLOWS = {"documentation": ("docs",), "security": ("deny",), "web": ("web",), "lint": ("deps", "lint"), "test": ("ci-scripts", "hermetic", "pg-adapter", "mutants")}

# The binary each target of scripts/contract.sh starts. Everything a cell runs against is built from it.
TARGET_BINARY = {
    "native-memory": "thrust-server",
    "native-pg": "thrust-server",
    "worker-memory": "thrust-worker",
    "worker-cf": "thrust-worker",
}
SUITE = "thrust-contract-tests"
PG_ADAPTER = "thrust-adapters-selfhosted"
MUTANTS_BINARY = "thrust-server"

FULL = "full"
CRATE = "crate"

# (pattern, effect, why). An effect is FULL, CRATE, or {"jobs": [...], "passes": [...]}.
RULES = [
    (r"^(Cargo\.(toml|lock)|rust-toolchain(\.toml)?)$", FULL, "the workspace manifest, the lockfile or the toolchain"),
    (r"^\.config/branding/", {"jobs": ["docs"]}, "the book's palette (read by docs/book.toml only)"),
    (r"^\.(cargo|config)/", FULL, "Cargo's or nextest's configuration"),
    # every workflow file that runs the checks `gate` waits for: today's two, and the ones the migration adds (ADR-0102). test_plan_ci.py
    # lists .github/workflows/ and fails for a file that is neither here nor among the ones that run no check.
    (r"^\.github/workflows/(ci|contract-cell|test|lint|documentation|security|web)\.yml$", FULL, "the workflow that runs the checks"),
    (r"^\.github/actions/setup-rust/", FULL, "the action every Rust job sets up with"),
    (r"^\.github/actions/contract-pass-tools/", FULL, "the action that prepares a contract cell's pass (uv, the app, the browser)"),
    (r"^\.github/actions/verdict/", FULL, "the action every called workflow judges its jobs with (ci.yml's gate requires its output)"),
    (r"^\.github/scripts/(plan-ci\.py|plan-cells\.sh|assert-gate\.sh)$", FULL, "this planner, or the gate that trusts it"),
    (r"^\.github/scripts/toolchain-channel\.sh$", FULL, "the reader of rust-toolchain.toml, which every build's compiler comes from"),
    (r"^scripts/(rs|contract\.sh|contract-all\.sh|worker-dev\.sh|pg-delay-proxy\.py|lib/ports\.sh)$", FULL, "the contract cells' harness"),
    (r"^crates/", CRATE, "a file of crates/: its crate, by the longest member directory that holds it"),
    (r"^\.github/deploy/flagship/", {"jobs": ["deps", "ci-scripts"]}, "the Flagship policy (validated by deps, read by test_flagship.py)"),
    (r"^\.github/deploy/", {"jobs": ["hermetic", "ci-scripts"]}, "the CA root the deploy trusts (read by thrust-adapters-postgres's TLS tests)"),
    (r"^deny\.toml$", {"jobs": ["deny"]}, "cargo-deny's configuration"),
    (r"^\.?(clippy|rustfmt)\.toml$", {"jobs": ["lint"]}, "clippy's or rustfmt's configuration"),
    (r"^release-plz\.toml$", {"jobs": ["deps", "ci-scripts"]}, "release-plz's configuration (check-release-plz.py)"),
    (r"^\.gitignore$", {"jobs": ["deps"]}, "the ignore list (the layout guard checks its lines under crates/)"),
    (r"^tools/oracle-v1/", {"jobs": ["hermetic", "deps"], "passes": ["oracle"]},
     "the v1 oracle: its cells, the OpenAPI diff, and the tests that read its schema and corpus"),
    (r"^tools/compat-record/", {"jobs": ["deps"], "passes": ["e2e", "e2e-mail"]}, "the compat recorder (the acceptance-list guard, scripts/e2e.sh)"),
    (r"^tools/compat-inventory/out-pinned/endpoint-inventory\.csv$", {"jobs": ["deps"]}, "the pinned route inventory (acceptance-list guard)"),
    (r"^\.agents/plans/contrail/research/0003-compat-plane-inventory\.md$", {"jobs": ["deps"]}, "the acceptance list's source table"),
    (r"^tools/(e2e-har|e2e-plane)/", {"passes": ["e2e", "e2e-mail"]}, "a tool of the browser journeys"),
    (r"^e2e/", {"passes": ["e2e", "e2e-mail"]}, "the browser journeys (both suites: journeys/ and journeys-mail/ share e2e/support/)"),
    (r"^web/", {"jobs": ["web"], "passes": ["e2e", "e2e-mail"]}, "the stock app: its build, and the journeys that drive it"),
    (r"^scripts/e2e\.sh$", {"passes": ["e2e", "e2e-mail"]}, "the browser journeys' runner"),
    (r"^scripts/oracle-v1(-reset)?\.sh$", {"passes": ["oracle"]}, "the v1 oracle's runner"),
    (r"^scripts/(mutants\.sh|lib/mutants-verdict\.sh)$", {"jobs": ["mutants", "ci-scripts"]},
     "the mutation controls and their verdicts (test-mutants-verdict.sh reads both)"),
    (r"^scripts/(stage2-contract|check-contract-counts)\.sh$", {"jobs": ["hermetic", "deps"]}, "the contract counts and their guard"),
    (r"^scripts/check-web-diff\.sh$", {"jobs": ["web", "ci-scripts"]}, "the Q-R2 gate of web/"),
    (r"^scripts/check-book\.sh$", {"jobs": ["docs"]}, "the book's check"),
    (r"^scripts/(check-(deps|layout|topology|plane-500|acceptance-list|placement|openapi|migration-lint|edge-config)|render-wrangler)\.sh$",
     {"jobs": ["deps", "ci-scripts"]}, "a guard of the deps job"),
    (r"^scripts/(smoke-deploy|smoke-bootstrap|migrate-ci|deploy-worker|merge-assets|check-asset-retention|edge-opt-in|deploy-target)\.sh$",
     {"jobs": ["ci-scripts"]}, "a deploy script (tested by ci-scripts; run by deploy.yml)"),
    (r"^scripts/flagship\.py$", {"jobs": ["deps", "ci-scripts"]},
     "the Flagship reconciler (its policy check runs in deps, its tests in ci-scripts; flagship.yml runs it)"),
    (r"^scripts/preview-(config|result)\.py$", {"jobs": ["ci-scripts"]},
     "a Preview target's configuration, or the reading of what was deployed (tested by ci-scripts)"),
    (r"^scripts/(verify-all|flake-hunt|measure-argon2|spike-0002-pg|stage2-latency)\.sh$|^scripts/measure-argon2-summary\.py$",
     {}, "a script no job of ci.yml runs"),
    (r"^scripts/", FULL, "a script this planner does not know"),
    (r"^docs/", {"jobs": ["docs"]}, "the book"),
    (r"^AGENTS\.md$", {"jobs": ["docs"]}, "AGENTS.md (the book includes its testing rules)"),
    (r"^\.github/scripts/(lint-workflow\.sh|lint-all-workflows\.sh|check-release-plz\.py)$", {"jobs": ["ci-scripts", "deps"]},
     "a guard the deps job runs"),
    (r"^\.github/(scripts|workflows|actions)/", {"jobs": ["ci-scripts", "deps"]},
     "another workflow, action or script (its tests, and the workflow lint)"),
]
COMPILED = [(re.compile(p), effect, why) for p, effect, why in RULES]

# A group directory's README.md (ADR-0009: crates/<group>/README.md, crates/README.md), when no member holds it.
GROUP_README = re.compile(r"^crates/(.+/)?README\.md$")


def fail(message):
    print(f"plan-ci: {message}", file=sys.stderr)
    sys.exit(2)


def read_lines(path):
    with open(path, encoding="utf-8") as f:
        return [line.strip() for line in f if line.strip()]


def workspace(metadata):
    """{member directory, relative to the workspace root: crate name} and {crate: set of the workspace crates it depends on}."""
    root = metadata.get("workspace_root", "").rstrip("/") + "/"
    members = set(metadata.get("workspace_members", []))
    packages = [p for p in metadata.get("packages", []) if p.get("id") in members]
    if not packages:
        fail("cargo metadata lists no workspace member")
    names = {p["name"] for p in packages}
    dirs, deps = {}, {}
    for p in packages:
        manifest = p["manifest_path"]
        if not manifest.startswith(root) or not manifest.endswith("/Cargo.toml"):
            fail(f"{p['name']}'s manifest {manifest} is not under the workspace root {root}")
        dirs[manifest[len(root):-len("/Cargo.toml")]] = p["name"]
        deps[p["name"]] = {d["name"] for d in p.get("dependencies", []) if d.get("path") and d["name"] in names}
    return dirs, deps


def owner(path, dirs):
    """The member directory that holds path: the longest one that is a prefix of it (a whole path segment), or None."""
    best = None
    for d in dirs:
        if path.startswith(d + "/") and (best is None or len(d) > len(best)):
            best = d
    return best


def affected(changed, deps):
    """The changed crates and every crate that depends on one of them, directly or not."""
    out = set(changed)
    grew = True
    while grew:
        grew = False
        for crate, uses in deps.items():
            if crate not in out and uses & out:
                out.add(crate)
                grew = True
    return out


def check_workflows(workflows=None, jobs=JOBS):
    """Refuses a WORKFLOWS that is not a partition: a name that is not a job, a job in two workflows, a workflow with no job."""
    workflows = WORKFLOWS if workflows is None else workflows
    seen = {}
    for workflow, members in workflows.items():
        if not members:
            fail(f"workflow {workflow} has no job")
        for job in members:
            if job not in jobs:
                fail(f"workflow {workflow} lists {job}, which is not one of the jobs")
            if job in seen:
                fail(f"{job} is in two workflows: {seen[job]} and {workflow}")
            seen[job] = workflow


def slices(planned, workflows=None, jobs=JOBS):
    """(top, called, plans) for a set of planned jobs. `top` is what ci.yml's own gate expects: the planned jobs that are still jobs
    of ci.yml itself, in the order of JOBS, then the called workflows that have a planned job. `plans` is each called workflow's
    own planned jobs, which it is given as its `plan`."""
    workflows = WORKFLOWS if workflows is None else workflows
    check_workflows(workflows, jobs)
    moved = {job for members in workflows.values() for job in members}
    called = [w for w, members in workflows.items() if planned & set(members)]
    plans = {w: [j for j in workflows[w] if j in planned] for w in called}
    top = [j for j in jobs if j in planned and j not in moved] + called
    return top, called, plans


def plan(paths, dirs, deps, cells, full_reason=None):
    """Returns (jobs, selected cells, notes)."""
    notes = []
    jobs, passes, changed = set(), set(), set()
    full = full_reason
    for path in paths:
        for pattern, effect, why in COMPILED:
            if not pattern.search(path):
                continue
            if effect == FULL:
                full = full or f"{path}: {why}"
                notes.append(f"{path}: {why} -> everything")
            elif effect == CRATE:
                top = owner(path, dirs)
                if top is None and GROUP_README.search(path):
                    jobs.add("deps")
                    notes.append(f"{path}: a group's README, inside no crate (the layout guard checks it names its crates) -> deps")
                elif top is None:
                    full = full or f"{path}: inside no crate of the workspace"
                    notes.append(f"{path}: inside no crate of the workspace -> everything")
                else:
                    crate = dirs[top]
                    changed.add(crate)
                    if path == f"{top}/Cargo.toml":
                        jobs.add("deny")
                    notes.append(f"{path}: crate {crate}")
            else:
                jobs.update(effect.get("jobs", []))
                passes.update(effect.get("passes", []))
                notes.append(f"{path}: {why} -> {', '.join(effect.get('jobs', []) + effect.get('passes', [])) or 'nothing'}")
            break
        else:
            notes.append(f"{path}: no job reads it")

    if full:
        return set(JOBS), list(cells), [f"everything: {full}"] + notes

    hit = affected(changed, deps)
    if changed:
        jobs.update(("ci-scripts", "deps", "lint", "hermetic"))
        notes.append(f"changed crates: {', '.join(sorted(changed))}; affected: {', '.join(sorted(hit))}")
    if PG_ADAPTER in hit:
        jobs.add("pg-adapter")
    if MUTANTS_BINARY in hit or SUITE in changed:
        jobs.add("mutants")

    selected = []
    for cell in cells:
        target, cell_pass = cell.split(" ")
        binary = TARGET_BINARY.get(target)
        if binary is None:
            notes.append(f"cell {cell}: target {target} is not in plan-ci.py's TARGET_BINARY, so it runs on every change")
            selected.append(cell)
        elif binary in hit or SUITE in changed or cell_pass in passes:
            selected.append(cell)
    if selected:
        jobs.add("contract")
    return jobs, selected, notes


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--changed")
    parser.add_argument("--metadata")
    parser.add_argument("--cells", required=True)
    parser.add_argument("--full")
    args = parser.parse_args()

    cells = read_lines(args.cells)
    if not cells:
        fail(f"{args.cells} lists no cell")
    for cell in cells:
        if len(cell.split(" ")) != 2:
            fail(f"'{cell}' in {args.cells} is not '<target> <pass>'")

    if args.full:
        paths, dirs, deps = [], {}, {}
    else:
        if not args.changed or not args.metadata:
            fail("--changed and --metadata are needed unless --full is given")
        paths = read_lines(args.changed)
        with open(args.metadata, encoding="utf-8") as f:
            dirs, deps = workspace(json.load(f))
    full = args.full or (None if paths else "the change list is empty")

    jobs, selected, notes = plan(paths, dirs, deps, cells, full)
    for note in notes:
        print(note, file=sys.stderr)
    ordered = [j for j in JOBS if j in jobs]
    top, called, plans = slices(set(ordered))  # before the first line is printed: a map that is not a partition prints nothing
    matrix = {"include": [{"target": c.split(" ")[0], "pass": c.split(" ")[1]} for c in selected]}
    print("jobs=" + json.dumps(ordered, separators=(",", ":")))
    print("matrix=" + json.dumps(matrix, separators=(",", ":")))
    print("cells=" + json.dumps(selected, separators=(",", ":")))
    print("top=" + json.dumps(top, separators=(",", ":")))
    print("workflows=" + json.dumps(called, separators=(",", ":")))
    print("plans=" + json.dumps(plans, separators=(",", ":")))
    print(f"plan-ci: {len(ordered)} jobs, {len(selected)} of {len(cells)} cells", file=sys.stderr)


if __name__ == "__main__":
    main()
