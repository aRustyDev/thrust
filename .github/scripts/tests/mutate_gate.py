"""Hand mutants of plan-ci.py, assert-gate.sh and the workflow files that call each other: one bug planted in a copy at a time, and
the tests must fail for it. Run by ci-scripts, because a gate whose tests cannot fail is not a gate (AGENTS.md rule 8).

    python3 -I .github/scripts/tests/mutate_gate.py [--jobs N] [--only TEXT] [--list]

  a plan-ci.py mutant must fail test_plan_ci.py, run with the copy as its argument, or test_workflow_wiring.py, run on a copy of the
  tree that holds it;
  an assert-gate.sh mutant must fail test-ci-scripts.sh, run under dash (Ubuntu's sh) in a copy of the tree it reads, stopping
  at the first failing case (STOP_AT_FIRST_BAD), since one answer is all it wants;
  a ci.yml, called-workflow (documentation.yml, security.yml, ...) or verdict-action mutant must fail test_workflow_wiring.py, run on a
  copy of the tree that holds it.

It refuses to answer, and says why, when its own answer could not be trusted:
  * the tests must pass on the unmutated files first, or every mutant would be "killed" by whatever broke them;
  * the text a mutant replaces must be found exactly once, or the mutant tests nothing (the first driver, kept in a session's
    scratchpad, counted such a mutant as a "noop" and went on, and one of its 34 targets text the planner has since reworded);
  * a mutant must still compile (python's `compile`, `dash -n`), or the tests would fail for a syntax error;
  * the two EQUIVALENT mutants, which change no behaviour, must SURVIVE: a driver that kills them is killing everything.

Exit status: 0 every mutant killed and every equivalent one survived; 1 a mutant survived, was broken, or an equivalent one died;
2 the tests do not pass on the unmutated files.
"""

import argparse
import concurrent.futures
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.abspath(os.path.join(HERE, ".."))
ROOT = os.path.abspath(os.path.join(SCRIPTS, "..", ".."))
PLAN, GATE = "plan-ci.py", "assert-gate.sh"
CI, DOC, SEC, WEB, LNT, TST, BLD = "ci.yml", "documentation.yml", "security.yml", "web.yml", "lint.yml", "test.yml", "build.yml"  # ci.yml, and each called workflow
ACT = "verdict/action.yml"  # the action every called workflow's `verdict` job calls
DEEP = "deep.yml"  # not a called workflow: the nightly run of the randomized tests, which nothing waits for
NAT = "build-native/action.yml"  # the steps the native jobs share (linux in build.yml, macos in build-extended.yml)
BEXT = "build-extended.yml"  # not a called workflow: nightly and before a release, never in the gate
TOOLS = "contract-pass-tools/action.yml"  # what a pass needs on the runner; the e2e cells are the longest job of every full run
SHELL = shutil.which("dash") or "sh"
TIMEOUT = 900
# where each target lives, relative to the repository root
TARGET_PATH = {
    PLAN: ".github/scripts/plan-ci.py",
    GATE: ".github/scripts/assert-gate.sh",
    CI: ".github/workflows/ci.yml",
    DOC: ".github/workflows/documentation.yml",
    SEC: ".github/workflows/security.yml",
    WEB: ".github/workflows/web.yml",
    LNT: ".github/workflows/lint.yml",
    TST: ".github/workflows/test.yml",
    BLD: ".github/workflows/build.yml",
    ACT: ".github/actions/verdict/action.yml",
    DEEP: ".github/workflows/deep.yml",
    TOOLS: ".github/actions/contract-pass-tools/action.yml",
    NAT: ".github/actions/build-native/action.yml",
    BEXT: ".github/workflows/build-extended.yml",
}
# The workflows that tests judge as files: a mutant of one of these must fail test_workflow_wiring.py.
WIRED = (CI, DOC, SEC, WEB, LNT, TST, BLD, ACT, DEEP, TOOLS, NAT, BEXT)

# The called workflows, each with its file, its jobs (in the order the file has them), and the text of ci.yml's `gate` needs that names its
# call and what that becomes without it. A stage of the migration adds a row, and every mutant below that is written for "a called
# workflow" is made for it; a row with several jobs gets the mutants only several jobs can have.
CALLS = [
    (DOC, "documentation", ("docs",), "web, documentation, security, build]", "web, security, build]"),
    (SEC, "security", ("deny",), "documentation, security, build]", "documentation, build]"),
    (WEB, "web", ("web",), "lint, web, documentation", "lint, documentation"),
    (LNT, "lint", ("deps", "lint", "bash"), "test, lint, web", "test, web"),
    (TST, "test", ("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants", "workers"), "changes, test, lint", "changes, lint"),
    (BLD, "build", ("linux", "wasm"), "security, build]", "security]"),
]
CELL_JOB = "contract"  # the job that runs the contract cells; the workflow that holds it gets the mutants of the cells (call_mutants)


def call_mutants():
    """The mutants of each called workflow's call in ci.yml and of its own file: the same bugs, planted once per workflow."""
    out = []
    for target, w, jobs, needs_old, needs_new in CALLS:
        job = jobs[0]
        jobs_text = " ".join(jobs)
        needs_text = job if len(jobs) == 1 else "[" + ", ".join(jobs) + "]"
        plan_text = "[" + ",".join(f'"{j}"' for j in jobs) + "]"
        out += [
            (CI, f"the {w} call runs on the planner's `jobs`", f"if: contains(fromJSON(needs.changes.outputs.top || '[]'), '{w}')",
             f"if: contains(fromJSON(needs.changes.outputs.jobs || '[]'), '{w}')"),
            (CI, f"the gate does not need the {w} call", needs_old, needs_new),
            (CI, f"the {w} call passes a fixed plan, not the planner's",
             f"plan: ${{{{ toJSON(fromJSON(needs.changes.outputs.plans || '{{}}').{w}) }}}}", f"plan: '{plan_text}'"),
            (CI, f"the {w} call is of another workflow's file", f"uses: ./.github/workflows/{w}.yml", "uses: ./.github/workflows/contract-cell.yml"),
            # the newline first: a job's own step may have an `if: always()` too (hermetic's upload), indented deeper
            (target, f"{w}: the verdict job does not run when a job failed", "\n    if: always()\n", "\n    if: success()\n"),
            (target, f"{w}: the verdict job judges no plan", "          plan: ${{ inputs.plan }}\n", "          plan: '[]'\n"),
            (target, f"{w}: the verdict job knows no job of the workflow", f"          jobs: {jobs_text}\n", "          jobs: verdict\n"),
            (target, f"{w}: the verdict job does not wait for the job", f"    needs: {needs_text}\n", "    needs: []\n"),
            (target, f"{w}: the verdict job exposes nothing of the call", "    outputs:\n      verdict: ${{ steps.judge.outputs.verdict }}\n", ""),
            (target, f"{w}: the verdict job calls another action", "uses: ./.github/actions/verdict", "uses: ./.github/actions/setup-rust"),
            (target, f"{w}: the verdict job does not check the repository out",
             re.compile(r"(    steps:\n)      - uses: actions/checkout@[^\n]*\n        with:\n          persist-credentials: false\n(      - id: judge)"), r"\1\2"),
            (target, f"{w}: the verdict job has a command of its own", f"          jobs: {jobs_text}\n",
             f"          jobs: {jobs_text}\n      - run: echo hi\n"),
            (target, f"{w}: the workflow does not expose the verdict", "value: ${{ jobs.verdict.outputs.verdict }}", "value: pass"),
            (target, f"{w}: the workflow sets a workflow-level concurrency", "permissions: {}\n\njobs:", f"permissions: {{}}\n\nconcurrency:\n  group: {w}\n\njobs:"),
            (target, f"{w}: the workflow cannot run alone", "  workflow_dispatch:\n", "  push:\n"),
            (target, f"{w}: the workflow reads the event name", f"          jobs: {jobs_text}\n",
             f"          jobs: {jobs_text}\n          event: ${{{{ github.event_name }}}}\n"),
        ]
        for j in jobs:  # what is true of each job of the workflow
            out += [
                (CI, f"the moved job {j} is left in ci.yml as well", "  gate:\n    name: gate",
                 f"  {j}:\n    runs-on: ubuntu-24.04\n    steps:\n      - run: echo\n\n  gate:\n    name: gate"),
                (target, f"{w}: the job {j} runs whatever the plan", f"if: contains(fromJSON(inputs.plan), '{j}')", "if: true"),
            ]
        if len(jobs) > 1:  # what only a workflow of several jobs can get wrong
            out += [
                (target, f"{w}: the verdict job waits for the first job only", f"    needs: {needs_text}\n", f"    needs: {jobs[0]}\n"),
                (target, f"{w}: the verdict job knows the first job only", f"          jobs: {jobs_text}\n", f"          jobs: {jobs[0]}\n"),
                (target, f"{w}: the verdict job knows the jobs in a list that is not the workflow's", f"          jobs: {jobs_text}\n",
                 f"          jobs: {jobs_text} verdict\n"),
            ]
        if CELL_JOB in jobs:  # what only the workflow that runs the contract cells can get wrong (and the call, and the gate, around it)
            out += [
                (CI, f"the {w} call passes no matrix", "      matrix: ${{ needs.changes.outputs.matrix || '{\"include\":[]}' }}\n", ""),
                (CI, f"the {w} call passes no cells", "      cells: ${{ needs.changes.outputs.cells || '[]' }}\n", ""),
                (CI, f"the {w} call passes the cells where the matrix goes", "matrix: ${{ needs.changes.outputs.matrix || '{\"include\":[]}' }}",
                 "matrix: ${{ needs.changes.outputs.cells || '[]' }}"),
                (CI, "the gate still downloads the cell markers", "      - name: Assert the planned set ran and passed\n",
                 "      - name: Collect the per-cell pass markers\n        uses: actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2\n"
                 "        with:\n          pattern: cell-ok-*\n          path: reported\n          merge-multiple: true\n"
                 "      - name: Assert the planned set ran and passed\n"),
                (CI, "the gate is still given the planned cells", "          VERDICT_JOBS: documentation security web lint test build\n",
                 "          VERDICT_JOBS: documentation security web lint test build\n          PLANNED: ${{ needs.changes.outputs.cells || '[]' }}\n"),
                (target, f"{w}: the verdict call passes no cell job", "          cell-job: contract\n", ""),
                (target, f"{w}: the verdict call passes no cells", "          cells: ${{ inputs.cells }}\n", ""),
                (target, f"{w}: the verdict call passes other cells than it was given", "cells: ${{ inputs.cells }}\n", "cells: '[]'\n"),
                (target, f"{w}: the verdict call names another job as the cell job", "cell-job: contract\n", "cell-job: mutants\n"),
                (target, f"{w}: the contract matrix is a fixed one, not the one it was given", "matrix: ${{ fromJSON(inputs.matrix) }}\n",
                 'matrix: {"include":[{"target":"native-memory","pass":"main"}]}\n'),
                (target, f"{w}: a failed cell stops the others", "      fail-fast: false\n      matrix: ${{ fromJSON(inputs.matrix) }}\n",
                 "      fail-fast: true\n      matrix: ${{ fromJSON(inputs.matrix) }}\n"),
                (target, f"{w}: contract calls another workflow", "    uses: ./.github/workflows/contract-cell.yml\n", "    uses: ./.github/workflows/web.yml\n"),
                (target, f"{w}: the matrix is not asked of the caller",
                 re.compile(r"      matrix:\n        description: The contract matrix, [^\n]*\n        type: string\n        required: true\n"), ""),
                (target, f"{w}: the cells are optional for the caller",
                 re.compile(r"(      cells:\n        description: JSON array of the \"<target> <pass>\" cells that are planned \(the planner's `cells`\)\n        type: string\n)"
                            r"        required: true"), r"\1        required: false"),
            ]
    return out

# (file, label, the text to replace, its replacement). The text, or a compiled regex whose replacement may use \1, must match once
# in the file.
MUTANTS = [
    # --- plan-ci.py: which jobs and cells a change needs
    (PLAN, "only direct dependents are affected", "if crate not in out and uses & out:", "if crate not in out and uses & set(changed):"),
    (PLAN, "dev and build dependencies are ignored", 'if d.get("path") and d["name"] in names}',
     'if d.get("path") and d["name"] in names and d.get("kind") is None}'),
    (PLAN, "pg-adapter only when the adapter itself changed", "if PG_ADAPTER in hit:", "if PG_ADAPTER in changed:"),
    (PLAN, "the suite does not run mutants", "if MUTANTS_BINARY in hit or SUITE in changed:", "if MUTANTS_BINARY in hit:"),
    (PLAN, "the suite does not run the cells", "elif binary in hit or SUITE in changed or cell_pass in passes:",
     "elif binary in hit or cell_pass in passes:"),
    (PLAN, "a pass's own inputs do not run it", "elif binary in hit or SUITE in changed or cell_pass in passes:",
     "elif binary in hit or SUITE in changed:"),
    (PLAN, "an unknown target is dropped", "            selected.append(cell)\n        elif", "            pass\n        elif"),
    (PLAN, "a file of crates/ outside a crate runs nothing", 'full = full or f"{path}: inside no crate of the workspace"', "pass"),
    (PLAN, "every matching rule applies, not the first", "            break\n        else:", "        else:"),
    (PLAN, "a crate manifest does not run deny", '                        jobs.add("deny")', "                        pass"),
    # The mutants of a rule's effect find the rule by its pattern and change the effect, so a rule that gains a pass or a job (the
    # second e2e suite added `e2e-mail` to four of them, an hour after the first run of this driver) does not break them; a rule
    # that is renamed or removed does, which is the signal.
    (PLAN, "web/ does not run the journeys", re.compile(r'(\(r"\^web/", \{"jobs": \["web"\]), "passes": \[[^\]]*\]\}'), r"\1}"),
    (PLAN, "the CA root does not run hermetic", re.compile(r'(\(r"\^\\\.github/deploy/", \{"jobs": \[)"hermetic", '), r"\1"),
    (PLAN, "a crate change does not run lint", 'jobs.update(("ci-scripts", "deps", "lint", "hermetic"))',
     'jobs.update(("ci-scripts", "deps", "hermetic"))'),
    (PLAN, "an empty change list runs nothing", 'full = args.full or (None if paths else "the change list is empty")', "full = args.full"),
    (PLAN, "cells but no contract job", '    if selected:\n        jobs.add("contract")', '    if False:\n        jobs.add("contract")'),
    (PLAN, "native-pg is mapped to the Worker", '"native-pg": "thrust-server"', '"native-pg": "thrust-worker"'),
    (PLAN, "e2e.sh runs no cell", re.compile(r'(\(r"\^scripts/e2e\\\.sh\$", )\{"passes": \[[^\]]*\]\}'), r"\1{}"),
    (PLAN, "an unknown script runs nothing", '(r"^scripts/", FULL, "a script this planner does not know")',
     '(r"^scripts/", {}, "a script this planner does not know")'),
    (PLAN, "the oracle's files run no hermetic", re.compile(r'(\(r"\^tools/oracle-v1/", \{"jobs": \[)"hermetic", '), r"\1"),
    (PLAN, "--full runs only the matrix", "        return set(JOBS), list(cells),", '        return {"contract"}, list(cells),'),
    # --- plan-ci.py: the workflow files that run the checks, and the called workflows (ADR-0102)
    (PLAN, "a workflow that runs the checks, web.yml, is not run-everything", "|documentation|security|web|build)\\.yml$", "|documentation|security|build)\\.yml$"),
    (PLAN, "a workflow that runs the checks, documentation.yml, is not run-everything", "|lint|documentation|security|web|build)", "|lint|security|web|build)"),
    (PLAN, "a workflow that runs the checks, lint.yml, is not run-everything", "|test|lint|documentation", "|test|documentation"),
    (PLAN, "a workflow that runs the checks, test.yml, is not run-everything", "|contract-cell|test|lint", "|contract-cell|lint"),
    (PLAN, "a workflow that runs the checks, security.yml, is not run-everything", "|documentation|security|web|build)", "|documentation|web|build)"),
    (PLAN, "a workflow that runs the checks, build.yml, is not run-everything", "|documentation|security|web|build)", "|documentation|security|web)"),
    (PLAN, "the verdict action is not run-everything", re.compile(r'(\(r"\^\\\.github/actions/verdict/", )FULL'), r"\1{}"),
    (PLAN, "a job may be in two workflows", "            if job in seen:", "            if False:"),
    (PLAN, "a workflow may list a name that is not a job", "            if job not in jobs:", "            if False:"),
    (PLAN, "a workflow may have no job", "        if not members:", "        if False:"),
    (PLAN, "a called workflow with a planned job is not asked for", "called = [w for w, members in workflows.items() if planned & set(members)]",
     "called = [w for w, members in workflows.items() if False]"),
    (PLAN, "top forgets the jobs that stay in ci.yml", "top = [j for j in jobs if j in planned and j not in moved] + called", "top = called"),
    (PLAN, "top lists a moved job as well as its workflow", "if j in planned and j not in moved]", "if j in planned]"),
    (PLAN, "a plan holds jobs that were not planned", "plans = {w: [j for j in workflows[w] if j in planned] for w in called}",
     "plans = {w: list(workflows[w]) for w in called}"),
    (PLAN, "the planner does not print top", '    print("top=" + json.dumps(top, separators=(",", ":")))', "    pass"),
    (PLAN, "the planner prints no plans", '    print("plans=" + json.dumps(plans, separators=(",", ":")))', '    print("plans=" + json.dumps({}, separators=(",", ":")))'),
    # --- assert-gate.sh: what the gate, and a callee's verdict job, accept
    (GATE, "an expected job may be skipped", '[ "$result" = success ] || fail "$job was expected',
     '[ "$result" = success ] || [ "$result" = skipped ] || fail "$job was expected'),
    (GATE, "an unexpected job may run", '[ "$result" = skipped ] || fail "$job was not expected', 'true || fail "$job was not expected'),
    (GATE, "an unknown expected job is accepted", "*) fail \"the planner expects '$job', which is not a job the gate knows\" ;;", "*) ;;"),
    (GATE, "an expected job need not be a need", 'grep -q "^$job " needs.txt || fail', "true || fail"),
    (GATE, "an unknown need is accepted", "*) fail \"'$job' is a need of the gate but not a job the planner knows\" ;;", "*) ;;"),
    (GATE, "the planner job is not checked", '[ "$changes_result" = "success" ] ||', "true ||"),
    (GATE, "cells may be planned without the cell job", '[ ! -s planned.txt ] || fail "cells are planned', 'true || fail "cells are planned'),
    (GATE, "a planned cell may not report", '[ -z "$missing" ] ||', "true ||"),
    (GATE, "an unplanned marker is accepted", '[ -z "$extra" ] ||', "true ||"),
    (GATE, "the cell job with zero cells passes", '[ -s planned.txt ] || fail "the $CELLS job is expected', 'true || fail "the $CELLS job is expected'),
    (GATE, "an input may be missing", '[ -n "$value" ] || fail "$input is not set"', 'true || fail "$input is not set"'),
    (GATE, "the gate forgets the documentation workflow", "documentation security web lint test build}\"", "security web lint test build}\""),
    (GATE, "the gate forgets the security workflow", "security web lint test build}\"", "web lint test build}\""),
    (GATE, "the gate forgets the web workflow", " web lint test build}\"", " lint test build}\""),
    (GATE, "the gate forgets the lint workflow", " lint test build}\"", " test build}\""),
    (GATE, "the gate forgets the test workflow", " test build}\"", " build}\""),
    (GATE, "the gate forgets the build workflow", " test build}\"", " test}\""),
    (GATE, "any JSON is an array", "jq -e 'type == \"array\" and all(.[]; type == \"string\")'", "jq -e 'true'"),
    (GATE, "an array of anything is an array of jobs", "jq -e 'type == \"array\" and all(.[]; type == \"string\")'", "jq -e 'type == \"array\"'"),
    # --- assert-gate.sh: the verdict of a called workflow, and the other levels (ADR-0102)
    (GATE, "a callee that says nothing passes", '[ "$verdict" = pass ] || fail "$job succeeded but', 'true || fail "$job succeeded but'),
    (GATE, "any verdict that is not empty passes", '[ "$verdict" = pass ] ||', '[ -n "$verdict" ] ||'),
    (GATE, "VERDICT_JOBS is ignored", 'VERDICTS="${VERDICT_JOBS:-}"', 'VERDICTS=""'),
    (GATE, "a name in VERDICT_JOBS that is no job is accepted",
     "*) fail \"VERDICT_JOBS names '$job', which is not a job the gate knows\" ;;", "*) ;;"),
    (GATE, "a verdict job that sets no planner job still needs one", 'CHANGES="${CHANGES_JOB-changes}"', 'CHANGES="${CHANGES_JOB:-changes}"'),
    (GATE, "a level that names no cell job is asked for cells (the default cell job is contract)", 'CELLS="${CELL_JOB:-}"', 'CELLS="${CELL_JOB:-contract}"'),
    (GATE, "KNOWN_JOBS is ignored", 'KNOWN="${KNOWN_JOBS:-documentation', 'KNOWN="${XKNOWN_JOBS:-documentation'),
    (GATE, "PLANNED is needed even where there is no cell job", 'if [ -n "$CELLS" ]; then', "if true; then"),
    (GATE, "cells given to a level that names no cell job are judged by nobody",
     '[ "$(printf \'%s\' "${PLANNED:-[]}" | jq -c . 2> /dev/null)" = "[]" ] || fail "cells are planned but no cell job',
     'true || fail "cells are planned but no cell job'),
    (GATE, "an empty array of cells is told by its text, not its value", "| jq -c . 2> /dev/null)\" = \"[]\" ]", "| cat)\" = \"[]\" ]"),
    (GATE, "a level with no cell job is given cells only when PLANNED is unset", '"${PLANNED:-[]}" | jq -c .', '"[]" | jq -c .'),
    # --- the wiring of the called workflows, and what a stage of the migration changes (ADR-0102)
    (PLAN, "the planner forgets the workflows",
     'WORKFLOWS = {"documentation": ("docs",), "security": ("deny",), "web": ("web",), "lint": ("deps", "lint", "bash"), '
     '"test": ("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants", "workers"), "build": ("linux", "wasm")}', "WORKFLOWS = {}"),
    (PLAN, "the planner forgets the documentation workflow", '{"documentation": ("docs",), "security"', '{"security"'),
    (PLAN, "the planner forgets the security workflow", ', "security": ("deny",)', ""),
    (PLAN, "the planner forgets the web workflow", ', "web": ("web",)', ""),
    (PLAN, "the planner forgets the lint workflow", ', "lint": ("deps", "lint", "bash")', ""),
    (PLAN, "the planner forgets the test workflow", ', "test": ("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants", "workers")', ""),
    (PLAN, "the planner forgets the build workflow", ', "build": ("linux", "wasm")', ""),
    (PLAN, "the planner moves linux and wasm to a workflow of another name", '"build": ("linux", "wasm")', '"compile": ("linux", "wasm")'),
    (PLAN, "the planner leaves wasm in ci.yml", '"build": ("linux", "wasm")', '"build": ("linux",)'),
    (PLAN, "the planner leaves linux in ci.yml", '"build": ("linux", "wasm")', '"build": ("wasm",)'),
    (PLAN, "the planner moves test's jobs to a workflow of another name", '"test": ("ci-scripts"', '"suite": ("ci-scripts"'),
    (PLAN, "the planner leaves ci-scripts in ci.yml", '("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants", "workers")',
     '("hermetic", "pg-adapter", "contract", "mutants", "workers")'),
    (PLAN, "the planner leaves contract in ci.yml", '("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants", "workers")',
     '("ci-scripts", "hermetic", "pg-adapter", "mutants", "workers")'),
    (PLAN, "the planner leaves mutants in ci.yml", '("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants", "workers")',
     '("ci-scripts", "hermetic", "pg-adapter", "contract", "workers")'),
    (PLAN, "the planner leaves workers in ci.yml", '("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants", "workers")',
     '("ci-scripts", "hermetic", "pg-adapter", "contract", "mutants")'),
    (PLAN, "the planner moves lint's jobs to a workflow of another name", '"lint": ("deps", "lint", "bash")', '"style": ("deps", "lint", "bash")'),
    (PLAN, "the planner leaves deps in ci.yml", '("deps", "lint", "bash")', '("lint", "bash")'),
    (PLAN, "the planner leaves lint in ci.yml", '("deps", "lint", "bash")', '("deps", "bash")'),
    (PLAN, "the planner leaves bash in ci.yml", '("deps", "lint", "bash")', '("deps", "lint")'),
    (PLAN, "the planner moves docs to a workflow of another name", '{"documentation": ("docs",)', '{"docs-book": ("docs",)'),
    (PLAN, "the planner moves deny to a workflow of another name", '"security": ("deny",)', '"supply-chain": ("deny",)'),
    (PLAN, "the planner moves web to a workflow of another name", '"web": ("web",)', '"site": ("web",)'),
    (CI, "the gate judges every planned job and not `top`", "EXPECTED_JOBS: ${{ needs.changes.outputs.top }}", "EXPECTED_JOBS: ${{ needs.changes.outputs.jobs }}"),
    (CI, "the gate has no VERDICT_JOBS", "          VERDICT_JOBS: documentation security web lint test build\n", ""),
    (CI, "the gate's VERDICT_JOBS names a workflow that is not called", "VERDICT_JOBS: documentation security web lint test build",
     "VERDICT_JOBS: documentation security web lint test build contract"),
    (CI, "the gate's VERDICT_JOBS forgets a called workflow", "VERDICT_JOBS: documentation security web lint test build", "VERDICT_JOBS: documentation security web lint build"),
    (CI, "the gate's VERDICT_JOBS forgets the build workflow", "VERDICT_JOBS: documentation security web lint test build", "VERDICT_JOBS: documentation security web lint test"),
    (CI, "`changes` does not output the plans", "      plans: ${{ steps.classify.outputs.plans }}\n", ""),
    (CI, "`changes` does not output top", "      top: ${{ steps.classify.outputs.top }}\n", ""),
    # --- the verdict action, which every called workflow's `verdict` job calls (written once, from the second workflow on)
    (ACT, "the action judges no plan", "        EXPECTED_JOBS: ${{ inputs.plan }}\n", "        EXPECTED_JOBS: '[]'\n"),
    (ACT, "the action knows no job of the workflow", "KNOWN_JOBS: ${{ inputs.jobs }}", "KNOWN_JOBS: verdict"),
    (ACT, "the action judges the needs of nobody", "NEEDS_JSON: ${{ inputs.needs }}", "NEEDS_JSON: '{}'"),
    (ACT, "the action asks for a planner job, which a called workflow has not", '        CHANGES_JOB: ""\n', ""),
    # the cells: the caller names the cell job and the planned cells, and the action collects the markers for the judge to compare
    (ACT, "the action ignores the cell job it was given", "CELL_JOB: ${{ inputs.cell-job }}", 'CELL_JOB: ""'),
    (ACT, "the action asks for the cell job whatever it was given", "CELL_JOB: ${{ inputs.cell-job }}", "CELL_JOB: contract"),
    (ACT, "the action ignores the planned cells it was given", "PLANNED: ${{ inputs.cells }}", "PLANNED: '[]'"),
    (ACT, "the judge reads the markers from another directory than they were put in", "REPORTED_DIR: reported", "REPORTED_DIR: elsewhere"),
    (ACT, "the markers are put in another directory than the judge reads", "        path: reported\n", "        path: elsewhere\n"),
    (ACT, "the markers are collected even when there is no cell job", "      if: inputs.cell-job != ''\n", ""),
    (ACT, "the markers are never collected", "      if: inputs.cell-job != ''\n", "      if: inputs.cell-job == 'never'\n"),
    (ACT, "a missing marker stops the action before the judge can say so", "      continue-on-error: true\n", ""),
    (ACT, "the markers are collected from an action that is not pinned", "actions/download-artifact@9000827ccba6bdab643e8b6fd33ac0654aef8333 # v8.0.2",
     "actions/download-artifact@v8"),
    # the judge first, then the collection: the two blocks swapped
    (ACT, "the markers are collected after the judge",
     re.compile(r"(    - name: Collect the per-cell pass markers\n.*?        merge-multiple: true\n)(    # A planned job must be exactly.*?      run: \.github/scripts/assert-gate\.sh\n)", re.S),
     r"\2\1"),
    (ACT, "every marker is not collected", "        pattern: cell-ok-*\n", "        pattern: cell-ok-native-*\n"),
    (ACT, "the cell job is a required input", re.compile(r"(  cell-job:\n    description: [^\n]*\n)    required: false"), r"\1    required: true"),
    (ACT, "the cells default to nothing that is an array", '    default: "[]"', '    default: ""'),
    (ACT, "the action judges with another script", "run: .github/scripts/assert-gate.sh", "run: true"),
    (ACT, "the output is set by the judge step too", "run: .github/scripts/assert-gate.sh", 'run: echo "verdict=pass" >> "$GITHUB_OUTPUT"; .github/scripts/assert-gate.sh'),
    (ACT, "the output is set even when the judge failed", "    - id: mark\n", "    - id: mark\n      if: always()\n"),
    (ACT, "the output is another word than pass", 'echo "verdict=pass"', 'echo "verdict=ok"'),
    (ACT, "the action does not expose the output", "value: ${{ steps.mark.outputs.verdict }}", "value: pass"),
    (ACT, "the plan is not a required input", re.compile(r"(  plan:\n    description: [^\n]*\n)    required: true"), r"\1    required: false"),
    # the ci-scripts step runs its commands a core at a time (Test phase, stage 1): a failure of any of them must still fail the job
    (TST, "ci-scripts forgets the status of its commands", "<<'EOF' || status=$?", "<<'EOF' || true"),
    (TST, "ci-scripts exits 0 whatever failed", '          exit "$status"\n', "          exit 0\n"),
    (TST, "ci-scripts stops at the first failing command", "parallel --will-cite --group --halt never", "parallel --will-cite --group --halt now,fail=1"),
    (TST, "ci-scripts no longer keeps a command's lines together", "parallel --will-cite --group --halt never", "parallel --will-cite --halt never"),
    (TST, "ci-scripts drops a test script from its list", "          sh .github/scripts/tests/test-ports.sh\n", ""),
    # the mutants job is a matrix of shards (Test phase, stage 2): the script is told how many and which, and a wrong pair loses rows or runs them twice
    (TST, "mutants is not told how many shards there are", "          MUTANTS_SHARDS: ${{ strategy.job-total }}\n", ""),
    (TST, "mutants is told a number of shards that is not the matrix's", "MUTANTS_SHARDS: ${{ strategy.job-total }}", "MUTANTS_SHARDS: 4"),
    (TST, "mutants is not told which shard it is", "          MUTANTS_SHARD: ${{ strategy.job-index }}\n", ""),
    (TST, "mutants is told every shard is the first", "MUTANTS_SHARD: ${{ strategy.job-index }}", "MUTANTS_SHARD: 0"),
    (TST, "mutants is one runner again", "        shard: [0, 1, 2, 3]\n", "        shard: [0]\n"),
    (TST, "mutants stops its other shards at the first red one", "      fail-fast: false\n      matrix:\n        shard:", "      fail-fast: true\n      matrix:\n        shard:"),
    (TST, "mutants is not a matrix", "    strategy:\n      fail-fast: false\n      matrix:\n        shard: [0, 1, 2, 3]\n", ""),
    (TST, "every mutants shard saves the cache", "save-cache: ${{ github.ref == 'refs/heads/main' && strategy.job-index == 0 }}",
     "save-cache: ${{ github.ref == 'refs/heads/main' }}"),
    # the hermetic job builds without debug info (Test phase, stage 3): nothing fails without it, so the check is the only thing that notices
    (TST, "hermetic builds with debug info again", "      CARGO_PROFILE_DEV_DEBUG: none\n", "      CARGO_PROFILE_DEV_DEBUG: full\n"),
    (TST, "hermetic has no debug setting", "    env:\n      CARGO_PROFILE_DEV_DEBUG: none\n    steps:\n", "    steps:\n"),
    # deep.yml, the nightly randomized tests (Test phase, stage 3, status G-48): not called, not gated, and each of these would still "work"
    (DEEP, "deep runs the oracle_ tests, which a runner has no oracle for", " -E 'not test(/\\boracle_/)'", ""),
    (DEEP, "deep runs at the default number of cases", "${{ inputs.cases || 4096 }}", "${{ inputs.cases }}"),
    (DEEP, "deep saves the cache and may evict what the pull requests use", "save-cache: 'false'", "save-cache: 'true'"),
    (DEEP, "deep is called by another workflow", "on:\n  workflow_dispatch:", "on:\n  workflow_call:\n  workflow_dispatch:"),
    (DEEP, "deep never runs by itself", "  schedule:\n    - cron: \"37 5 * * *\"\n", ""),
    # contract-pass-tools (Test phase, stage 6): the e2e cells get the browser and not the fonts apt would add (138 s to 57 s of the tools step, the cell's tail gone); nothing fails without it
    (TOOLS, "the e2e tools step apt-installs the browser's system packages again", "        pnpm --dir e2e exec playwright install chromium\n", "        pnpm --dir e2e exec playwright install --with-deps chromium\n"),
    (TOOLS, "the e2e tools step no longer installs the browser", "        pnpm --dir e2e exec playwright install chromium\n", ""),
    # the workers job (Test phase, stage 8, status G-6): the Worker's wasm32 unit tests. Each of these leaves a job that still looks healthy, until a bump of wasm-bindgen or a cache eviction
    (TST, "workers installs a runner of a version written in the workflow", "          tool: wasm-bindgen@${{ steps.wasm-bindgen.outputs.version }}\n", "          tool: wasm-bindgen@0.2.130\n"),
    (TST, "workers reads the runner's version from something else", '          id="$(scripts/rs cargo pkgid --locked wasm-bindgen)"\n', '          id="registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen@0.2.130"\n'),
    (TST, "workers reads the runner's version without holding Cargo to the lock", '          id="$(scripts/rs cargo pkgid --locked wasm-bindgen)"\n', '          id="$(scripts/rs cargo pkgid wasm-bindgen)"\n'),
    (TST, "workers installs a runner named by the whole package id", '          echo "version=${id##*[#@]}" >> "$GITHUB_OUTPUT"\n', '          echo "version=$id" >> "$GITHUB_OUTPUT"\n'),
    (TST, "workers runs nextest and not the script with the count guard", "        run: scripts/wasm-tests.sh\n",
     "        run: scripts/rs cargo nextest run --profile workers --target wasm32-unknown-unknown -p thrust-worker --locked\n"),
    (TST, "workers has the web jobs' Node, which asserts on the Worker's cloudflare: imports", "          node-version: '26.13.1'\n", "          node-version-file: web/.node-version\n"),
    (TST, "workers has a Node 22 pinned by version", "          node-version: '26.13.1'\n", "          node-version: '22.23.5'\n"),
    (TST, "workers has a Node 24 from before 24.12", "          node-version: '26.13.1'\n", "          node-version: '24.11.1'\n"),
    (TST, "workers builds for no wasm32 target", "          nextest: 'true'\n          targets: wasm32-unknown-unknown\n", "          nextest: 'true'\n"),
    (TST, "workers shares a cache key with the hermetic job", "          cache-key: wasm-workers\n", "          cache-key: native-hermetic\n"),
    (TST, "the verdict does not wait for workers", "    needs: [ci-scripts, hermetic, pg-adapter, contract, mutants, workers]\n", "    needs: [ci-scripts, hermetic, pg-adapter, contract, mutants]\n"),
    (TST, "the verdict does not judge workers", "          jobs: ci-scripts hermetic pg-adapter contract mutants workers\n", "          jobs: ci-scripts hermetic pg-adapter contract mutants\n"),
    (PLAN, "workers is not planned when the Worker changes", '    if WORKER in hit:\n        jobs.add("workers")\n', ""),
    (PLAN, "workers is planned for every crate change", "    if WORKER in hit:\n", "    if changed:\n"),
    (PLAN, "workers is planned when the native server changes", "    if WORKER in hit:\n", "    if WORKER in hit or MUTANTS_BINARY in hit:\n"),
    (PLAN, "a change to the wasm32 tests' scripts plans no workers", '{"jobs": ["workers", "ci-scripts"]}', '{"jobs": ["ci-scripts"]}'),
    (PLAN, "a change to the wasm32 tests' scripts plans no ci-scripts", '{"jobs": ["workers", "ci-scripts"]}', '{"jobs": ["workers"]}'),
    (PLAN, "the planner does not know the workers job", '"mutants", "workers", "linux", "wasm", "web", "docs")', '"mutants", "linux", "wasm", "web", "docs")'),
    # the bash job (Lint phase, stage 1, the shell lint). The planner adds it to what the first matching rule says (ALSO); the workflow installs
    # shellcheck at the version the helper is held to, and runs the helper, which has the control. Each of these leaves a job that looks healthy
    (PLAN, "the planner does not know the bash job", '"lint", "bash", "deny"', '"lint", "deny"'),
    (PLAN, "a shell script plans no shell lint", '(r"^(?!web/).*\\.sh$", "bash",', '(r"^(?!web/).*\\.shx$", "bash",'),
    (PLAN, "the stock app's scripts plan the shell lint", '(r"^(?!web/).*\\.sh$", "bash",', '(r"^.*\\.sh$", "bash",'),
    (PLAN, "shellcheck's configuration plans no shell lint", '    (r"^\\.shellcheckrc$", "bash", "shellcheck\'s configuration"),\n', ""),
    (PLAN, "a composite action plans no shell lint", '    (r"^\\.github/actions/[^/]+/action\\.yml$", "bash", "a composite action (shellcheck reads its run blocks)"),\n', ""),
    (PLAN, "the additions to a path's jobs are dropped", "                jobs.add(job)\n                notes.append(", "                notes.append("),
    # (the install step is in the deps job too, so each anchor of the bash job reaches the line that follows it, which is the bash job's alone)
    (LNT, "bash installs a shellcheck of a version the helper is not held to", "          tool: shellcheck@0.11.0\n      - name: shellcheck (the control, then every maintained script)\n",
     "          tool: shellcheck@0.10.0\n      - name: shellcheck (the control, then every maintained script)\n"),
    (LNT, "bash does not install shellcheck (the image's 0.9.0 answers, and the helper refuses it)",
     "      - uses: taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29\n        with:\n          tool: shellcheck@0.11.0\n      - name: shellcheck (the control, then every maintained script)\n",
     "      - name: shellcheck (the control, then every maintained script)\n"),
    (LNT, "bash runs shellcheck by hand, and not the helper that has the control", "        run: sh .github/scripts/check-shell.sh\n",
     "        run: shellcheck $(git ls-files '*.sh')\n"),
    (LNT, "bash sets up Rust it does not need",
     "      - uses: taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29\n        with:\n          tool: shellcheck@0.11.0\n      - name: shellcheck (the control, then every maintained script)\n",
     "      - uses: ./.github/actions/setup-rust\n      - uses: taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29\n        with:\n          tool: shellcheck@0.11.0\n      - name: shellcheck (the control, then every maintained script)\n"),
    (LNT, "the verdict does not wait for bash", "    needs: [deps, lint, bash]\n", "    needs: [deps, lint]\n"),
    (LNT, "the verdict does not judge bash", "          jobs: deps lint bash\n", "          jobs: deps lint\n"),
    # the deps job's shellcheck (Lint phase, stage 1 follow-up): actionlint runs the one on PATH over every workflow's run: blocks, so the job installs the pinned one
    # before the actionlint step and runs actionlint through the helper that hands it that shellcheck and has the control
    (LNT, "deps installs a shellcheck of a version the scripts are not held to", "          tool: shellcheck@0.11.0\n      # actionlint checks every workflow",
     "          tool: shellcheck@0.10.0\n      # actionlint checks every workflow"),
    (LNT, "deps does not install shellcheck (actionlint reads the workflows' shell with the image's 0.9.0, which only the helper's control refuses)",
     "      - uses: taiki-e/install-action@8cca0918f8471c33515202b73f56ab27f2bed283 # v2.87.29\n        with:\n          tool: shellcheck@0.11.0\n      # actionlint checks every workflow",
     "      # actionlint checks every workflow"),
    (LNT, "deps installs shellcheck after the step that runs actionlint, when the PATH was already read",
     re.compile(r"(      - uses: taiki-e/install-action@[0-9a-f]{40} # v\S+\n        with:\n          tool: shellcheck@0\.11\.0\n)(      # actionlint checks every workflow.*?lint-actionlint\.sh\n)", re.S),
     r"\2\1"),
    (LNT, "deps runs actionlint by hand, and not the helper that hands it the shellcheck and has the control",
     '          ACTIONLINT="$RUNNER_TEMP/actionlint" sh .github/scripts/lint-actionlint.sh\n', '          "$RUNNER_TEMP/actionlint"\n'),
    # the linux job (Phase 3, stage 1): the native server and `migrate`, built in release, as a compile proof. Each of these leaves a job that still looks healthy
    (PLAN, "linux is not planned when the native server changes", '    if any(crate in hit for crate in BUILD_CRATES):\n        jobs.add("linux")\n', ""),
    (PLAN, "linux is planned for every crate change", "    if any(crate in hit for crate in BUILD_CRATES):\n", "    if changed:\n"),
    (PLAN, "linux is planned only when the server's own crate changed", "    if any(crate in hit for crate in BUILD_CRATES):\n", "    if any(crate in changed for crate in BUILD_CRATES):\n"),
    (PLAN, "linux is not planned when only the crate of migrate changed", 'BUILD_CRATES = ("thrust-server", "thrust-adapters-postgres")', 'BUILD_CRATES = ("thrust-server",)'),
    (PLAN, "linux is not planned when only the server changed", 'BUILD_CRATES = ("thrust-server", "thrust-adapters-postgres")', 'BUILD_CRATES = ("thrust-adapters-postgres",)'),
    (PLAN, "the planner does not know the linux job", '"workers", "linux", "wasm", "web", "docs")', '"workers", "wasm", "web", "docs")'),
    # the action build-native (Phase 3, stages 1 and 3): the steps of the native jobs. Each of these leaves a job that still looks healthy
    (NAT, "native builds the server in debug", "      run: scripts/rs cargo build --release --locked -p thrust-server\n", "      run: scripts/rs cargo build --locked -p thrust-server\n"),
    (NAT, "native builds the server without the lockfile", "      run: scripts/rs cargo build --release --locked -p thrust-server\n", "      run: scripts/rs cargo build --release -p thrust-server\n"),
    (NAT, "native builds migrate with another command than the deploy bundle", " -p thrust-adapters-postgres --features native --bin migrate\n", " -p thrust-adapters-postgres --bin migrate\n"),
    (NAT, "native builds migrate in debug", "cargo build --release --locked -p thrust-adapters-postgres", "cargo build --locked -p thrust-adapters-postgres"),
    (NAT, "native checks for one binary only", "        for bin in thrust-server migrate; do\n", "        for bin in thrust-server; do\n"),
    (NAT, "native does not fail when a binary is missing", '"target/release/$bin$EXE" || { echo "::error::cargo built nothing at target/release/$bin$EXE"; exit 1; }', '"target/release/$bin$EXE" || true'),
    (NAT, "native compares the host with itself, which always agrees", 'test "$host" = "$TARGET" || {', 'test "$host" = "$host" || {'),
    (NAT, "native is not told its target by the job", "        TARGET: ${{ inputs.target }}\n", "        TARGET: x86_64-unknown-linux-gnu\n"),
    (NAT, "native checks the host after it has built", re.compile(r'(    # The control against a proof of the wrong target:.*?        echo "building for \$host"\n\n)(    - name: Build thrust-server \(release\)\n      shell: bash\n      run: scripts/rs cargo build --release --locked -p thrust-server\n\n)', re.S), r"\2\1"),
    (NAT, "native sets Rust up itself, two composites deep, where the cache is never saved", "    # The control against a proof of the wrong target:",
     "    - uses: ./.github/actions/setup-rust\n      with:\n        cache-key: build-native\n        save-cache: ${{ github.ref == 'refs/heads/main' }}\n\n    # The control against a proof of the wrong target:"),
    (NAT, "native uploads nothing and passes", "        if-no-files-found: error\n", "        if-no-files-found: ignore\n"),
    (NAT, "native keeps its binaries for ninety days", "        retention-days: 3\n", "        retention-days: 90\n"),
    (NAT, "native uploads under a name that is not its job's", "        name: build-${{ inputs.name }}-${{ inputs.target }}\n", "        name: build-${{ inputs.target }}\n"),
    (NAT, "native uploads one binary only", "          target/release/thrust-server${{ inputs.exe }}\n          target/release/migrate${{ inputs.exe }}\n", "          target/release/thrust-server${{ inputs.exe }}\n"),
    (NAT, "native checks for the binary without its suffix", 'test -x "target/release/$bin$EXE"', 'test -x "target/release/$bin"'),
    (NAT, "native is not given the suffix", "        EXE: ${{ inputs.exe }}\n", '        EXE: ""\n'),
    (NAT, "native uploads the binaries without their suffix", "          target/release/thrust-server${{ inputs.exe }}\n          target/release/migrate${{ inputs.exe }}\n", "          target/release/thrust-server\n          target/release/migrate\n"),
    (NAT, "native requires a suffix of every job", '    required: false\n    default: ""\n', '    required: true\n    default: ""\n'),
    (NAT, "native hands a secret to the upload", "    - name: Upload the binaries\n", "    - name: Upload the binaries\n      env:\n        TOKEN: ${{ secrets.GITHUB_TOKEN }}\n"),
    (BLD, "linux runs on an image that moves", "    runs-on: ubuntu-24.04\n    timeout-minutes: 30\n    permissions:\n      contents: read\n    steps:\n      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          persist-credentials: false\n      # Rust is set up HERE",
     "    runs-on: ubuntu-latest\n    timeout-minutes: 30\n    permissions:\n      contents: read\n    steps:\n      - uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1\n        with:\n          persist-credentials: false\n      # Rust is set up HERE"),
    (BLD, "linux shares a cache key with another job", "          cache-key: build-linux\n", "          cache-key: native-hermetic\n"),
    (BLD, "linux saves its cache on every ref", "          cache-key: build-linux\n          save-cache: ${{ github.ref == 'refs/heads/main' }}\n", "          cache-key: build-linux\n          save-cache: 'true'\n"),
    (BLD, "linux does not set Rust up (the action no longer does)", "      - uses: ./.github/actions/setup-rust\n        with:\n          cache-key: build-linux\n          save-cache: ${{ github.ref == 'refs/heads/main' }}\n", ""),
    (BLD, "linux claims the macOS target", "          target: x86_64-unknown-linux-gnu\n", "          target: aarch64-apple-darwin\n"),
    (BLD, "linux has a step of its own", "          name: linux\n          target: x86_64-unknown-linux-gnu\n", "          name: linux\n          target: x86_64-unknown-linux-gnu\n      - run: echo hi\n"),
    # build-extended.yml (stage 3; the owner's decision of 2026-11-23): the macOS build is not a required check, and runs nightly and before a release
    (BEXT, "macos shares a cache key with windows", "          cache-key: build-macos\n", "          cache-key: build-windows\n"),
    (BEXT, "windows shares a cache key with macos", "          cache-key: build-windows\n", "          cache-key: build-macos\n"),
    (BEXT, "macos saves its cache on every ref", "          cache-key: build-macos\n          save-cache: ${{ github.ref == 'refs/heads/main' }}\n", "          cache-key: build-macos\n          save-cache: 'true'\n"),
    (BEXT, "windows saves its cache on every ref", "          cache-key: build-windows\n          save-cache: ${{ github.ref == 'refs/heads/main' }}\n", "          cache-key: build-windows\n          save-cache: 'true'\n"),
    (BEXT, "macos does not set Rust up", "      # Rust is set up here, one composite deep, so that its post step can save the cache (see the header of build-native).\n      - uses: ./.github/actions/setup-rust\n        with:\n          cache-key: build-macos\n          save-cache: ${{ github.ref == 'refs/heads/main' }}\n", ""),
    (BEXT, "windows does not set Rust up", "      - uses: ./.github/actions/setup-rust\n        with:\n          cache-key: build-windows\n          save-cache: ${{ github.ref == 'refs/heads/main' }}\n", ""),
    (BEXT, "macos runs on the Linux runner", "    runs-on: macos-15\n", "    runs-on: ubuntu-24.04\n"),
    (BEXT, "macos runs on an image that moves", "    runs-on: macos-15\n", "    runs-on: macos-latest\n"),
    (BEXT, "macos names itself linux", "          name: macos\n", "          name: linux\n"),
    (BEXT, "macos claims the Linux target", "          target: aarch64-apple-darwin\n", "          target: x86_64-unknown-linux-gnu\n"),
    (BEXT, "macos has a step of its own", "          name: macos\n          target: aarch64-apple-darwin\n", "          name: macos\n          target: aarch64-apple-darwin\n      - run: echo hi\n"),
    (BEXT, "macos may ask for an id-token", "    runs-on: macos-15\n    timeout-minutes: 30\n    permissions:\n      contents: read\n", "    runs-on: macos-15\n    timeout-minutes: 30\n    permissions:\n      contents: read\n      id-token: write\n"),
    (BEXT, "windows runs on the Linux runner", "    runs-on: windows-2025\n", "    runs-on: ubuntu-24.04\n"),
    (BEXT, "windows runs on an image that moves", "    runs-on: windows-2025\n", "    runs-on: windows-latest\n"),
    (BEXT, "windows names itself macos", "          name: windows\n", "          name: macos\n"),
    (BEXT, "windows claims the Linux target", "          target: x86_64-pc-windows-msvc\n", "          target: x86_64-unknown-linux-gnu\n"),
    (BEXT, "windows forgets its suffix", "          exe: .exe\n", ""),
    (BEXT, "macos is given a suffix", "          name: macos\n          target: aarch64-apple-darwin\n", "          name: macos\n          target: aarch64-apple-darwin\n          exe: .exe\n"),
    (BEXT, "windows has a step of its own", "          target: x86_64-pc-windows-msvc\n          exe: .exe\n", "          target: x86_64-pc-windows-msvc\n          exe: .exe\n      - run: echo hi\n"),
    (BEXT, "windows may ask for an id-token", "    runs-on: windows-2025\n    timeout-minutes: 30\n    permissions:\n      contents: read\n", "    runs-on: windows-2025\n    timeout-minutes: 30\n    permissions:\n      contents: read\n      id-token: write\n"),
    (BEXT, "build-extended is a called workflow", "on:\n  workflow_dispatch:\n", "on:\n  workflow_call:\n  workflow_dispatch:\n"),
    (BEXT, "build-extended never runs by itself", '  schedule:\n    - cron: "11 6 * * *"\n', ""),
    (BEXT, "build-extended does not run when a release candidate is pushed", "  push:\n    branches:\n      - rc/*\n", ""),
    (BEXT, "build-extended is started by every pull request", "  pull_request:\n    paths:\n      - .github/workflows/build-extended.yml\n      - .github/actions/build-native/**\n", "  pull_request:\n"),
    (BEXT, "build-extended is not started by an edit of the action it uses", "      - .github/actions/build-native/**\n", ""),
    (BEXT, "build-extended makes its job wait for another", "  macos:\n    runs-on: macos-15\n", "  macos:\n    needs: [other]\n    runs-on: macos-15\n"),
    (BEXT, "build-extended grants more at the top", "\npermissions: {}\n", "\npermissions:\n  contents: write\n"),
    (BEXT, "build-extended lets two runs of one ref go on at once", "  group: build-extended-${{ github.ref }}\n", "  group: build-extended\n"),
    (BEXT, "build-extended lets a push not replace the run in flight", "  cancel-in-progress: ${{ github.event_name == 'push' || github.event_name == 'pull_request' }}\n", "  cancel-in-progress: false\n"),
    (BLD, "linux may ask for an id-token", "    if: contains(fromJSON(inputs.plan), 'linux')\n    runs-on: ubuntu-24.04\n    timeout-minutes: 30\n    permissions:\n      contents: read\n",
     "    if: contains(fromJSON(inputs.plan), 'linux')\n    runs-on: ubuntu-24.04\n    timeout-minutes: 30\n    permissions:\n      contents: read\n      id-token: write\n"),
    # the wasm job (Phase 3, stage 2): the bundle's two Workers, built with the bundle's commands, under the bundle's cache key. Each of these leaves a job that still looks healthy
    (PLAN, "wasm is not planned when the Worker changes", '    if any(crate in hit for crate in WASM_CRATES):\n        jobs.add("wasm")\n', ""),
    (PLAN, "wasm is planned for every crate change", "    if any(crate in hit for crate in WASM_CRATES):\n", "    if changed:\n"),
    (PLAN, "wasm is planned only when the Worker's own crate changed", "    if any(crate in hit for crate in WASM_CRATES):\n", "    if any(crate in changed for crate in WASM_CRATES):\n"),
    (PLAN, "wasm is not planned when only the edge Worker changed", 'WASM_CRATES = ("thrust-worker", "thrust-edge")', 'WASM_CRATES = ("thrust-worker",)'),
    (PLAN, "wasm is not planned when only the Worker changed", 'WASM_CRATES = ("thrust-worker", "thrust-edge")', 'WASM_CRATES = ("thrust-edge",)'),
    (PLAN, "the planner does not know the wasm job", '"linux", "wasm", "web", "docs")', '"linux", "web", "docs")'),
    (BLD, "wasm builds the Worker in debug", "        run: ../../scripts/rs worker-build --release . --no-default-features\n", "        run: ../../scripts/rs worker-build . --no-default-features\n"),
    (BLD, "wasm builds the Worker with its default features", "        run: ../../scripts/rs worker-build --release . --no-default-features\n", "        run: ../../scripts/rs worker-build --release .\n"),
    (BLD, "wasm builds the Worker in the edge's directory", "        working-directory: crates/worker\n", "        working-directory: crates/edge\n"),
    (BLD, "wasm builds the edge Worker in debug", "        run: ../../scripts/rs worker-build --release .\n", "        run: ../../scripts/rs worker-build .\n"),
    (BLD, "wasm checks one Worker only", "          for dir in crates/worker/build crates/edge/build; do\n", "          for dir in crates/worker/build; do\n"),
    (BLD, "wasm checks one file of the three only", "            for file in worker/shim.mjs index.js index_bg.wasm; do\n", "            for file in worker/shim.mjs; do\n"),
    (BLD, "wasm does not fail when a file is missing", '"$dir/$file" || { echo "::error::worker-build wrote no $dir/$file"; exit 1; }', '"$dir/$file" || true'),
    (BLD, "wasm has a cache key of its own, which build-bundle does not restore", "          cache-key: deploy-release\n", "          cache-key: build-wasm\n"),
    (BLD, "wasm saves its cache on every ref", "          cache-key: deploy-release\n          save-cache: ${{ github.ref == 'refs/heads/main' }}\n", "          cache-key: deploy-release\n          save-cache: 'true'\n"),
    (BLD, "wasm builds for no wasm32 target", "          targets: wasm32-unknown-unknown\n", ""),
    (BLD, "wasm installs no worker-build", "          worker-tools: \"true\"\n", ""),
    (BLD, "wasm uploads nothing and passes", "            crates/edge/build\n          if-no-files-found: error\n", "            crates/edge/build\n          if-no-files-found: ignore\n"),
    (BLD, "wasm keeps its Workers for ninety days", "            crates/edge/build\n          if-no-files-found: error\n          retention-days: 3\n",
     "            crates/edge/build\n          if-no-files-found: error\n          retention-days: 90\n"),
    (BLD, "wasm uploads the Worker only", "            crates/worker/build\n            crates/edge/build\n", "            crates/worker/build\n"),
    (BLD, "wasm may ask for an id-token", "    if: contains(fromJSON(inputs.plan), 'wasm')\n    runs-on: ubuntu-24.04\n    timeout-minutes: 30\n    permissions:\n      contents: read\n",
     "    if: contains(fromJSON(inputs.plan), 'wasm')\n    runs-on: ubuntu-24.04\n    timeout-minutes: 30\n    permissions:\n      contents: read\n      id-token: write\n"),
]
MUTANTS += call_mutants()

# Changes no behaviour, so the tests must still pass: if one is killed, the driver or the tests are broken, not strong.
EQUIVALENT = [
    (PLAN, "(equivalent) a comment is added to the planner", "FULL = \"full\"\n", "FULL = \"full\"  # a comment\n"),
    (GATE, "(equivalent) a comment is added to the gate", "set -eu\n", "set -eu\n# a comment\n"),
    (CI, "(equivalent) a comment is added to ci.yml", "on:\n  pull_request:\n", "on:\n  # a comment\n  pull_request:\n"),
    (DOC, "(equivalent) a comment is added to documentation.yml", "permissions: {}\n\njobs:", "permissions: {}\n\n# a comment\njobs:"),
    (SEC, "(equivalent) a comment is added to security.yml", "permissions: {}\n\njobs:", "permissions: {}\n\n# a comment\njobs:"),
    (WEB, "(equivalent) a comment is added to web.yml", "permissions: {}\n\njobs:", "permissions: {}\n\n# a comment\njobs:"),
    (LNT, "(equivalent) a comment is added to lint.yml", "permissions: {}\n\njobs:", "permissions: {}\n\n# a comment\njobs:"),
    (TST, "(equivalent) a comment is added to test.yml", "permissions: {}\n\njobs:", "permissions: {}\n\n# a comment\njobs:"),
    (BLD, "(equivalent) a comment is added to build.yml", "permissions: {}\n\njobs:", "permissions: {}\n\n# a comment\njobs:"),
    (ACT, "(equivalent) a comment is added to the verdict action", "runs:\n  using: composite\n", "runs:\n  # a comment\n  using: composite\n"),
    (DEEP, "(equivalent) a comment is added to deep.yml", "permissions: {}\n\njobs:", "permissions: {}\n\n# a comment\njobs:"),
    (TOOLS, "(equivalent) a comment is added to contract-pass-tools", "runs:\n  using: composite\n", "runs:\n  # a comment\n  using: composite\n"),
    (NAT, "(equivalent) a comment is added to build-native", "runs:\n  using: composite\n", "runs:\n  # a comment\n  using: composite\n"),
    (BEXT, "(equivalent) a comment is added to build-extended.yml", "permissions: {}\n\n# A newer push", "permissions: {}\n\n# a comment\n# A newer push"),
]


def run(cmd, env=None):
    done = subprocess.run(cmd, capture_output=True, text=True, timeout=TIMEOUT, env=env)
    return done.returncode, done.stdout + done.stderr


def plan_tests(planner):
    return run([sys.executable, "-I", os.path.join(SCRIPTS, "tests", "test_plan_ci.py"), planner])


def gate_tests(base, gate_text):
    """test-ci-scripts.sh, in a copy of the tree it reads, with this text as assert-gate.sh."""
    shutil.copytree(SCRIPTS, os.path.join(base, ".github", "scripts"), ignore=shutil.ignore_patterns("__pycache__"))
    os.makedirs(os.path.join(base, "scripts"))
    shutil.copy(os.path.join(ROOT, "scripts", "contract-all.sh"), os.path.join(base, "scripts"))
    with open(os.path.join(base, ".github", "scripts", GATE), "w", encoding="utf-8") as f:
        f.write(gate_text)
    return run([SHELL, os.path.join(base, ".github", "scripts", "tests", "test-ci-scripts.sh")],
               dict(os.environ, STOP_AT_FIRST_BAD="1"))


def wiring_tests(base, overrides):
    """test_workflow_wiring.py, on a copy of .github/ under `base`, with these files (relative to the root) replaced by these texts."""
    shutil.copytree(os.path.join(ROOT, ".github"), os.path.join(base, ".github"), ignore=shutil.ignore_patterns("__pycache__", "node_modules"))
    for rel, text in overrides.items():
        with open(os.path.join(base, rel), "w", encoding="utf-8") as f:
            f.write(text)
    return run([sys.executable, "-I", os.path.join(base, ".github", "scripts", "tests", "test_workflow_wiring.py"), base])


def read(target):
    with open(os.path.join(ROOT, TARGET_PATH[target]), encoding="utf-8") as f:
        return f.read()


def judge(target, text, workdir):
    """(exit status, output) of the tests of `target` run against `text`, after the check that it still compiles."""
    if target in WIRED:
        return wiring_tests(os.path.join(workdir, "tree"), {TARGET_PATH[target]: text})
    if target == PLAN:
        try:
            compile(text, PLAN, "exec")
        except SyntaxError as e:
            return None, f"does not compile: {e}"
        path = os.path.join(workdir, PLAN)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        status, out = plan_tests(path)
        if status != 0:
            return status, out
        return wiring_tests(os.path.join(workdir, "tree"), {TARGET_PATH[PLAN]: text})
    syntax = os.path.join(workdir, "syntax.sh")
    with open(syntax, "w", encoding="utf-8") as f:
        f.write(text)
    status, out = run([SHELL, "-n", syntax])
    if status != 0:
        return None, f"does not parse: {out.strip()}"
    return gate_tests(os.path.join(workdir, "tree"), text)


def apply(text, old, new):
    """(how many times `old` matches, the text with the first match replaced). `old` is a string or a compiled regex."""
    if isinstance(old, re.Pattern):
        return len(old.findall(text)), old.sub(new, text, count=1)
    return text.count(old), text.replace(old, new, 1)


def one(entry, sources):
    target, label, old, new = entry
    n, mutated = apply(sources[target], old, new)
    if n != 1:
        return "BROKEN", f"the text to replace is found {n} times in {target}, not once"
    with tempfile.TemporaryDirectory() as workdir:
        status, out = judge(target, mutated, workdir)
    if status is None:
        return "BROKEN", out
    return ("SURVIVED" if status == 0 else "killed"), out


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--jobs", type=int, default=min(8, os.cpu_count() or 2), help="mutants run at once")
    ap.add_argument("--only", help="only the mutants whose label contains this text")
    ap.add_argument("--list", action="store_true", help="list the mutants and stop")
    args = ap.parse_args()
    entries = [e for e in MUTANTS + EQUIVALENT if not args.only or args.only in e[1]]
    if args.list:
        for i, (target, label, _, _) in enumerate(MUTANTS + EQUIVALENT, 1):
            print(f"{i:3} {target:15} {label}")
        return 0

    sources = {target: read(target) for target in TARGET_PATH}
    for target in TARGET_PATH:
        with tempfile.TemporaryDirectory() as workdir:
            status, out = judge(target, sources[target], workdir)
        if status != 0:
            print(f"mutate_gate: the tests of {target} fail on the unmutated file, so no mutant could be judged:\n{out[-1500:]}", file=sys.stderr)
            return 2

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.jobs) as pool:
        results = list(pool.map(lambda e: one(e, sources), entries))
    killed = survived = broken = wrong = 0
    for i, (entry, (verdict, out)) in enumerate(zip(entries, results), 1):
        equivalent = entry[1].startswith("(equivalent)")
        if equivalent:
            ok = verdict == "SURVIVED"
            wrong += not ok
            print(f"{'ok      ' if ok else 'WRONG   '} {i:3} {entry[1]}" + ("" if ok else f": it was {verdict}, and it must survive\n{out[-600:]}"))
        elif verdict == "killed":
            killed += 1
            print(f"killed   {i:3} {entry[1]}")
        elif verdict == "SURVIVED":
            survived += 1
            print(f"SURVIVED {i:3} {entry[1]}")
        else:
            broken += 1
            print(f"BROKEN   {i:3} {entry[1]}: {out}")
    print(f"mutants={sum(1 for e in entries if not e[1].startswith('(equivalent)'))} killed={killed} survived={survived} broken={broken} "
          f"equivalent-wrong={wrong} (tests run under {SHELL})")
    return 1 if survived or broken or wrong else 0


if __name__ == "__main__":
    sys.exit(main())
