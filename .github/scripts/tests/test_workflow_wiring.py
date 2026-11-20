"""The wiring of the called workflows, read from the real files (ADR-0102). Run by ci-scripts.

    test_workflow_wiring.py [repository root]

For every workflow in the planner's WORKFLOWS (the jobs a migration stage has moved out of ci.yml), the planner, `ci.yml` and the
workflow's own file must agree:
  * ci.yml has a calling job of that name: `needs: changes`, it runs when the planner's `top` names it, it calls that workflow's file
    and passes it its own plan from `plans`; and the job is no longer a job of ci.yml itself (a workflow that bears its job's name, as
    `web` does, has its call under that id, which is the one job of that name);
  * ci.yml's `gate` needs every job of ci.yml, judges the planner's `top` and not every planned job, and lists the called workflows as
    VERDICT_JOBS, a static list held equal here to the keys of WORKFLOWS (the planner's own output would let a planner bug switch the
    check off for the workflow it dropped);
  * the workflow's file has exactly the jobs the planner moved into it, then a `verdict` job: `if: always()`, needing those jobs,
    which checks the repository out and calls the action `verdict` with its needs, the plan it was given and those jobs, and
    exposes the output of that call;
  * the action (`.github/actions/verdict/`, once for all the workflows) judges what it is given with assert-gate.sh, with no planner
    job, and sets the output only after the judge passed; if it is given a cell job it first collects the cells' pass markers, into the
    directory the judge reads them from, and gives the judge the cell job and the planned cells;
  * the workflow that holds the job `contract` (the one that runs the contract cells) is given the planner's matrix and cells as inputs,
    calls contract-cell.yml once per cell, and passes the cell job and the cells to the action, which no other workflow does; ci.yml
    passes it the planner's matrix and cells, and its gate judges no cells and downloads no marker;
  * each of its jobs runs when the plan names it, and it can be called and run alone, with no workflow-level `concurrency` and no
    `github.event_name` (the rules R8 and R9 of lint-workflow.sh);
  * ci.yml holds only `changes`, the calls and `gate` (the migration's acceptance for it).
A root argument points it at a copy of the tree, which is how mutate_gate.py shows that this test can fail.
"""

import importlib.util
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(sys.argv[1]) if len(sys.argv) > 1 else os.path.abspath(os.path.join(HERE, "..", "..", ".."))

spec = importlib.util.spec_from_file_location("plan_ci", os.path.join(ROOT, ".github", "scripts", "plan-ci.py"))
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


def read_text(rel):
    path = os.path.join(ROOT, rel)
    if not os.path.isfile(path):
        return None
    with open(path, encoding="utf-8") as f:
        return f.read()


def job_ids(text):
    """The ids of a workflow file's jobs: the keys at two spaces of indent under `jobs:`."""
    ids, in_jobs = [], False
    for line in text.splitlines():
        if re.match(r"^jobs:\s*$", line):
            in_jobs = True
            continue
        if in_jobs and re.match(r"^[A-Za-z]", line):
            in_jobs = False
        m = re.match(r"^  ([A-Za-z_][A-Za-z0-9_-]*):\s*$", line)
        if in_jobs and m:
            ids.append(m.group(1))
    return ids


def block(text, job):
    """The text of one job of a workflow: from its key to the next job's key or the end."""
    m = re.search(rf"^  {re.escape(job)}:\n((?:(?:    .*|\s*#.*|)\n)*)", text, re.M)
    return m.group(0) if m else ""


CELL_JOB = "contract"  # the job that runs the contract cells: its workflow's verdict job proves that each planned cell reported

ci_text = read_text(".github/workflows/ci.yml") or ""
ci_jobs = job_ids(ci_text)
moved_jobs = [j for jobs in pc.WORKFLOWS.values() for j in jobs]

check("ci.yml is there and has jobs", len(ci_jobs) > 2, True)
verdicts = re.findall(r"^\s+VERDICT_JOBS: (.+)$", ci_text, re.M)
check("ci.yml's gate sets VERDICT_JOBS once", len(verdicts), 1)
check("ci.yml's VERDICT_JOBS is the keys of the planner's WORKFLOWS: a static list, held equal to it here",
      sorted(verdicts[0].split()) if verdicts else None, sorted(pc.WORKFLOWS))
gate_needs = re.search(r"^  gate:.*?^    needs: \[(.*?)\]", ci_text, re.M | re.S)
check("ci.yml's gate needs every other job of ci.yml, and nothing else",
      sorted(x.strip() for x in gate_needs.group(1).split(",")) if gate_needs else None, sorted(j for j in ci_jobs if j != "gate"))
# A moved job's name may still be a job of ci.yml in one case: the call of the workflow that bears it (`web` calls web.yml, which holds
# the job `web`). That call is checked below (it must call its workflow's file), and an id used twice is refused here.
check("a job that has moved is no longer a job of ci.yml, but for the call that bears its workflow's name",
      [j for j in moved_jobs if j in ci_jobs and j not in pc.WORKFLOWS], [])
check("no job id of ci.yml is used twice (a call that bears its job's name is the one job of that name)", len(ci_jobs), len(set(ci_jobs)))
check("ci.yml's gate judges the planner's `top`, not every planned job",
      re.search(r"EXPECTED_JOBS: \$\{\{ needs\.changes\.outputs\.top \}\}", ci_text) is not None, True)
check("ci.yml's `changes` job outputs `top` and `plans`",
      all(re.search(rf"^      {o}: \$\{{\{{ steps\.classify\.outputs\.{o} \}}\}}$", ci_text, re.M) for o in ("top", "plans")), True)

for workflow, jobs in pc.WORKFLOWS.items():
    body = block(ci_text, workflow)
    check(f"ci.yml calls {workflow}.yml in a job {workflow}", body != "", True)
    check(f"the {workflow} call runs when `top` names it", f"if: contains(fromJSON(needs.changes.outputs.top || '[]'), '{workflow}')" in body, True)
    check(f"the {workflow} call is of its own file", f"uses: ./.github/workflows/{workflow}.yml" in body, True)
    check(f"the {workflow} call passes its plan from `plans`",
          f"plan: ${{{{ toJSON(fromJSON(needs.changes.outputs.plans || '{{}}').{workflow}) }}}}" in body, True)
    check(f"the {workflow} call needs `changes`", "needs: changes" in body, True)
    path = f".github/workflows/{workflow}.yml"
    text = read_text(path)
    check(f"{path} exists", text is not None, True)
    text = text or ""
    check(f"{path} has the jobs the planner moved into it, then a `verdict` job", job_ids(text), list(jobs) + ["verdict"])
    check(f"{path} can be called and run alone", ("workflow_call:" in text, "workflow_dispatch:" in text), (True, True))
    check(f"{path} exposes its verdict job's output", "value: ${{ jobs.verdict.outputs.verdict }}" in text, True)
    check(f"{path} sets no workflow-level concurrency and reads no event name (R8, R9)",
          (re.search(r"^concurrency:", text, re.M), re.search(r"github\.event_name", text)), (None, None))
    for job in jobs:
        check(f"{path}: {job} runs when the plan names it", f"if: contains(fromJSON(inputs.plan), '{job}')" in block(text, job), True)
    verdict = block(text, "verdict")
    check(f"{path}: the verdict job runs `always()`", "    if: always()\n" in verdict, True)
    needs = re.search(r"^    needs: (\[.*\]|\S+)$", verdict, re.M)
    listed = sorted(x.strip() for x in needs.group(1).strip("[]").split(",")) if needs else None
    check(f"{path}: the verdict job needs every one of the workflow's jobs", listed, sorted(jobs))
    # the judge and the output are the action's (checked once, below); this job checks the repository out, calls it, and exposes what it says
    check(f"{path}: the verdict job exposes the call's output", "    outputs:\n      verdict: ${{ steps.judge.outputs.verdict }}\n" in verdict, True)
    check(f"{path}: the verdict job has two steps, the checkout and the call", len(re.findall(r"^      - ", verdict, re.M)), 2)
    check(f"{path}: the verdict job checks out first, then calls the action",
          0 < verdict.find("uses: actions/checkout@") < verdict.find("uses: ./.github/actions/verdict"), True)
    check(f"{path}: the call has the id the output is read from", "      - id: judge\n        uses: ./.github/actions/verdict\n" in verdict, True)
    # a workflow that runs the contract cells says so to the action, which then proves that each cell reported; one that does not says nothing
    cells_extra = "          cell-job: contract\n          cells: ${{ inputs.cells }}\n" if CELL_JOB in jobs else ""
    check(f"{path}: the call passes the job's needs, the plan it was given and exactly the workflow's jobs (and the cell job and the cells, if it has them)",
          f"        with:\n          needs: ${{{{ toJSON(needs) }}}}\n          plan: ${{{{ inputs.plan }}}}\n          jobs: {' '.join(jobs)}\n{cells_extra}" in verdict, True)
    check(f"{path}: a workflow with no cell job passes none to the verdict action",
          CELL_JOB in jobs or ("cell-job" not in verdict and "cells:" not in verdict), True)
    check(f"{path}: the verdict job runs no command of its own (it could not set the output but through the action)",
          re.search(r"^\s+run:", verdict, re.M), None)

# The action every `verdict` job calls: the judge and the output, written once (it was a copy in each workflow until the second one).
action_path = ".github/actions/verdict/action.yml"
action = read_text(action_path)
check(f"{action_path} exists, as soon as one workflow is called", action is not None or not pc.WORKFLOWS, True)
action = action or ""
for name in ("needs", "plan", "jobs"):
    check(f"{action_path}: the input {name} is declared and required", f"  {name}:\n    description:" in action and
          re.search(rf"^  {name}:\n    description: .*\n    required: true$", action, re.M) is not None, True)
check(f"{action_path}: its output is the mark step's", "outputs:\n  verdict:\n    description: pass, set only after the judge passed\n"
      "    value: ${{ steps.mark.outputs.verdict }}\n" in action, True)
check(f"{action_path}: it judges the needs it was given against the plan and the jobs it was given",
      all(f"        {k}: ${{{{ inputs.{v} }}}}\n" in action for k, v in (("NEEDS_JSON", "needs"), ("EXPECTED_JOBS", "plan"), ("KNOWN_JOBS", "jobs"))), True)
check(f"{action_path}: it has no planner job to judge (that is ci.yml's)", "        CHANGES_JOB: \"\"\n" in action, True)
# The cells: the caller names the cell job and the planned cells, and the action collects the markers and has the judge compare them.
for name, default in (("cell-job", '""'), ("cells", '"[]"')):
    check(f"{action_path}: the input {name} is optional, with the default {default} (no cells)",
          re.search(rf"^  {name}:\n    description: .*\n    required: false\n    default: {re.escape(default)}$", action, re.M) is not None, True)
check(f"{action_path}: the judge gets the cell job and the planned cells from the inputs, and reads the markers from the directory they are put in",
      "        CELL_JOB: ${{ inputs.cell-job }}\n        PLANNED: ${{ inputs.cells }}\n        REPORTED_DIR: reported\n" in action, True)
collect_at = action.find("- name: Collect the per-cell pass markers")
collect = action[collect_at:action.find("    # A planned job must be exactly")] if collect_at >= 0 else ""
check(f"{action_path}: it collects the markers before the judge, and only for a workflow that has a cell job",
      0 <= collect_at < action.find("run: .github/scripts/assert-gate.sh") and "      if: inputs.cell-job != ''\n" in collect, True)
check(f"{action_path}: the collection tolerates that there is nothing to download (the judge then fails on `no cell markers`, so it cannot pass)",
      "      continue-on-error: true\n" in collect, True)
check(f"{action_path}: the collection is the pinned download of every cell marker into the directory the judge reads",
      re.search(r"      uses: actions/download-artifact@[0-9a-f]{40} # v\S+\n      with:\n        pattern: cell-ok-\*\n        path: reported\n"
                r"        merge-multiple: true\n", collect) is not None, True)
check(f"{action_path}: it judges with the one script", action.count("run: .github/scripts/assert-gate.sh") == 1, True)
judge_at, mark_at = action.find("run: .github/scripts/assert-gate.sh"), action.find("- id: mark")
check(f"{action_path}: the output is set after the judge, and only there", 0 < judge_at < mark_at and action.count("verdict=pass") == 1, True)
check(f"{action_path}: the mark step writes the output and nothing else, and runs only if the judge passed",
      action[mark_at:].split("\n", 1)[1:] == ['      shell: bash\n      run: echo "verdict=pass" >> "$GITHUB_OUTPUT"\n'], True)

# The workflow that holds the job `contract` runs the contract cells, and its verdict job proves that each planned cell reported (M5b): it
# is given the planner's matrix and cells, calls contract-cell.yml once per cell, and passes the cell job and the cells to the action.
cell_wf = next((w for w, jobs in pc.WORKFLOWS.items() if CELL_JOB in jobs), None)
check(f"the job {CELL_JOB} is a job of a called workflow, and no longer of ci.yml", cell_wf is not None and CELL_JOB not in ci_jobs, True)
if cell_wf:
    cell_path = f".github/workflows/{cell_wf}.yml"
    cell_text = read_text(cell_path) or ""
    for name, default in (("matrix", "'{\"include\":[]}'"), ("cells", "'[]'")):
        check(f"{cell_path}: the input {name} is required when called, and has a default when run alone",
              re.search(rf"^      {name}:\n        description: .*\n        type: string\n        required: true$", cell_text, re.M) is not None and
              re.search(rf"^      {name}:\n        description: .*\n        type: string\n        default: {re.escape(default)}$", cell_text, re.M) is not None, True)
    cell_block = block(cell_text, CELL_JOB)
    check(f"{cell_path}: {CELL_JOB} is one call of contract-cell.yml per cell of the matrix it was given, and no cell stops the others",
          all(s in cell_block for s in ("    strategy:\n      fail-fast: false\n      matrix: ${{ fromJSON(inputs.matrix) }}\n",
                                        "    uses: ./.github/workflows/contract-cell.yml\n",
                                        "    with:\n      target: ${{ matrix.target }}\n      pass: ${{ matrix.pass }}\n",
                                        "    permissions:\n      contents: read\n")), True)
    ci_call = block(ci_text, cell_wf)
    check(f"ci.yml's {cell_wf} call passes the planner's matrix and the planner's cells",
          "      matrix: ${{ needs.changes.outputs.matrix || '{\"include\":[]}' }}\n      cells: ${{ needs.changes.outputs.cells || '[]' }}\n" in ci_call, True)
check("ci.yml's gate judges no cells: it downloads no marker and is given no planned cells and no directory of markers",
      all(s not in ci_text for s in ("download-artifact", "cell-ok", "PLANNED:", "REPORTED_DIR:")), True)
# The migration's acceptance for ci.yml (phase 1): it holds the planner, the calls and the gate, and nothing else, so no job of it needs
# to read the planner's `jobs` any more.
check("ci.yml holds only `changes`, the calls of the called workflows and `gate`", sorted(ci_jobs), sorted(["changes", "gate"] + list(pc.WORKFLOWS)))

# The ci-scripts job runs its commands a core at a time (Test phase, stage 1), so a failure of any of them must still fail the job. A step that ran them
# all and then exited 0 would pass the gate with every helper test broken, so the lines that carry the status are checked as text, and the mutant driver
# removes each of them. And since the commands are a list in the workflow, a test script that is not in it would never run: every one is.
SCRIPTS_JOB = "ci-scripts"
scripts_wf = next((w for w, jobs in pc.WORKFLOWS.items() if SCRIPTS_JOB in jobs), None)
check(f"the job {SCRIPTS_JOB} is a job of a called workflow", scripts_wf is not None, True)
if scripts_wf:
    scripts_path = f".github/workflows/{scripts_wf}.yml"
    scripts_block = block(read_text(scripts_path) or "", SCRIPTS_JOB)
    heredoc = re.search(r"^          parallel (.*) <<'EOF' \|\| status=\$\?\n((?:          .*\n)*?)          EOF\n", scripts_block, re.M)
    check(f"{scripts_path}: {SCRIPTS_JOB} runs its commands under parallel, from a heredoc, and keeps the status when parallel fails", heredoc is not None, True)
    flags = heredoc.group(1) if heredoc else ""
    check(f"{scripts_path}: {SCRIPTS_JOB}: parallel runs every command and fails at the end (--halt never), and keeps a command's lines together (--group)",
          "--halt never" in flags and "--group" in flags, True)
    step_lines = [l.strip() for l in scripts_block.splitlines() if l.startswith("          ") and l.strip() and not l.strip().startswith("#")]
    check(f"{scripts_path}: {SCRIPTS_JOB}: the step ends with `exit \"$status\"`, the number of commands that failed", step_lines[-1] if step_lines else None, 'exit "$status"')
    commands = [l.strip() for l in (heredoc.group(2) if heredoc else "").splitlines() if l.strip()]
    check(f"{scripts_path}: {SCRIPTS_JOB}: no command is listed twice", len(commands), len(set(commands)))
    tests_dir = os.path.join(ROOT, ".github", "scripts", "tests")
    on_disk = sorted(f for f in os.listdir(tests_dir) if re.match(r"(test[-_].*|mutate_gate\.py)$", f))
    listed = sorted({name for c in commands for name in re.findall(r"\.github/scripts/tests/(\S+)", c)})
    check(f"{scripts_path}: {SCRIPTS_JOB} runs every test script of .github/scripts/tests/ (a new one is added to the list)", on_disk, listed)

# The mutants job is a matrix of runners (Test phase, stage 2), and scripts/mutants.sh gives each its share of the rows from two numbers. If either were
# wrong the rows would not be shared out (a leg that is told nothing runs every row, one told a count that is not the matrix's drops some), and no row's
# verdict would say so, so the lines that carry them are checked as text and the mutant driver changes each. The script's own tests prove the sharing
# (test-mutants-shards.sh); what only the workflow can get wrong is here.
MUTANTS_JOB = "mutants"
mutants_wf = next((w for w, jobs in pc.WORKFLOWS.items() if MUTANTS_JOB in jobs), None)
check(f"the job {MUTANTS_JOB} is a job of a called workflow", mutants_wf is not None, True)
if mutants_wf:
    mutants_path = f".github/workflows/{mutants_wf}.yml"
    mutants_block = block(read_text(mutants_path) or "", MUTANTS_JOB)
    legs = re.search(r"^    strategy:\n      fail-fast: false\n      matrix:\n        shard: \[([0-9, ]+)\]\n", mutants_block, re.M)
    check(f"{mutants_path}: {MUTANTS_JOB} is a matrix of shards, `fail-fast: false` so each reports its own rows", legs is not None, True)
    check(f"{mutants_path}: {MUTANTS_JOB} has more than one shard (one would be the whole run on one runner again)",
          len(legs.group(1).split(",")) > 1 if legs else None, True)
    check(f"{mutants_path}: {MUTANTS_JOB}'s script is told the number of shards by the matrix itself (strategy.job-total), not by a number written beside it",
          "        env:\n          MUTANTS_SHARDS: ${{ strategy.job-total }}\n" in mutants_block, True)
    check(f"{mutants_path}: {MUTANTS_JOB}'s script is told which shard it is (strategy.job-index)", "          MUTANTS_SHARD: ${{ strategy.job-index }}\n        run: scripts/mutants.sh\n" in mutants_block, True)
    check(f"{mutants_path}: {MUTANTS_JOB}: one shard saves the cache (the others would each build the archive and fail to reserve its key)",
          "          save-cache: ${{ github.ref == 'refs/heads/main' && strategy.job-index == 0 }}\n" in mutants_block, True)

# The hermetic job builds without debug info (Test phase, stage 3): its nextest step 99 s to 72 s, measured (the phase document). Nothing breaks when the line is
# missing, so the saving would go unnoticed: the line is checked as text, at the job level (an `env` on one step would leave the other cargo steps on another
# profile, and cargo rebuilds when the profile differs), and the mutant driver changes it.
HERMETIC_JOB = "hermetic"
hermetic_wf = next((w for w, jobs in pc.WORKFLOWS.items() if HERMETIC_JOB in jobs), None)
check(f"the job {HERMETIC_JOB} is a job of a called workflow", hermetic_wf is not None, True)
if hermetic_wf:
    hermetic_path = f".github/workflows/{hermetic_wf}.yml"
    hermetic_block = block(read_text(hermetic_path) or "", HERMETIC_JOB)
    check(f"{hermetic_path}: {HERMETIC_JOB} builds without debug info, at the job level so that every cargo step shares one profile",
          "    env:\n      CARGO_PROFILE_DEV_DEBUG: none\n    steps:\n" in hermetic_block, True)

# deep.yml (Test phase, stage 3, status G-48): the randomized tests at 4096 cases every night, not gated. It is its own workflow, like flake-hunt.yml: ci.yml does not call
# it and `gate` does not read it, so a red night blocks nothing. A run that hid its failures, or one that ran the `oracle_` tests (a runner has no oracle: they would fail
# every night for want of one), or one that lost its schedule, would all still "work", so the lines that carry those are checked as text and the mutant driver changes each.
deep_text = read_text(".github/workflows/deep.yml")
check("deep.yml is there", deep_text is not None, True)
if deep_text is not None:
    check("deep.yml is not a called workflow: no `workflow_call`, and ci.yml has no job for it", "workflow_call" not in deep_text and "deep" not in ci_jobs, True)
    check("deep.yml runs by hand and every night", "  workflow_dispatch:\n" in deep_text and re.search(r"^  schedule:\n    - cron: ", deep_text, re.M) is not None, True)
    check("deep.yml runs the deep profile over the workspace, less the oracle_ tests, which need an oracle a runner does not have",
          "        run: scripts/rs cargo nextest run --profile deep --workspace --locked -E 'not test(/\\boracle_/)'\n" in deep_text, True)
    check("deep.yml's cases default to 4096", "          PROPTEST_CASES: ${{ inputs.cases || 4096 }}\n" in deep_text, True)
    check("deep.yml only restores the cache: a nightly must not evict what the pull requests use", "          save-cache: 'false'\n" in deep_text, True)

# contract-pass-tools (Test phase, stage 6): the e2e passes get the browser and not the fonts `--with-deps` would apt-install (about 32 MB, 8 minutes on a slow mirror, most of the e2e cell's
# run-to-run spread, and that cell is the longest job of every full run). Nothing fails without the saving, so the line that carries it is checked as text and the mutant driver changes it.
tools_text = read_text(".github/actions/contract-pass-tools/action.yml")
check("contract-pass-tools is there", tools_text is not None, True)
if tools_text is not None:
    check("contract-pass-tools installs the e2e browser by itself, in its own line",
          "        pnpm --dir e2e exec playwright install chromium\n" in tools_text, True)
    check("contract-pass-tools does not apt-install the browser's system packages (no command line, comments aside, has `playwright install --with-deps`)",
          [l for l in tools_text.splitlines() if "playwright install --with-deps" in l and not l.strip().startswith("#")], [])

# The workers job (Test phase, stage 8, status G-6) runs the Worker's wasm32 unit tests under wasm-bindgen-test-runner in Node. Five things about it can be wrong and leave
# the job looking healthy until a Cargo.lock bump or a cache eviction: a runner pinned to a version written in the workflow (it must be the lock's, or the job goes red on
# the next bump of wasm-bindgen with a message about the runner), a Node that asserts on the Worker's `cloudflare:` imports (every 22, which is what the web jobs pin, and 23, 24 before 24.12, 25 before 25.2: the first run of this job, ERR_INTERNAL_ASSERTION, on 22.22.0), no wasm32 target
# (the build would fail, but only after the cache and the tools), a cache key shared with a native job (a wasm32 test profile saved under it evicts that job's entries), and a
# run of nextest that is not scripts/wasm-tests.sh (which carries the count guard). Each is a line, checked as text, and the mutant driver changes each.
WORKERS_JOB = "workers"
workers_wf = next((w for w, jobs in pc.WORKFLOWS.items() if WORKERS_JOB in jobs), None)
check(f"the job {WORKERS_JOB} is a job of a called workflow", workers_wf is not None, True)
if workers_wf:
    workers_path = f".github/workflows/{workers_wf}.yml"
    workers_block = block(read_text(workers_path) or "", WORKERS_JOB)
    workers_code = [l for l in workers_block.splitlines() if not l.strip().startswith("#")]
    check(f"{workers_path}: {WORKERS_JOB} runs scripts/wasm-tests.sh, which has the count guard, and not nextest by itself", "        run: scripts/wasm-tests.sh\n" in workers_block, True)
    check(f"{workers_path}: {WORKERS_JOB} installs the runner at the wasm-bindgen of Cargo.lock, which Cargo (`cargo pkgid --locked`) reads from it",
          "          tool: wasm-bindgen@${{ steps.wasm-bindgen.outputs.version }}\n" in workers_block
          and '          id="$(scripts/rs cargo pkgid --locked wasm-bindgen)"\n' in workers_block
          and '          echo "version=${id##*[#@]}" >> "$GITHUB_OUTPUT"\n' in workers_block, True)
    check(f"{workers_path}: {WORKERS_JOB} writes no version of wasm-bindgen itself (no `wasm-bindgen@<digit>` outside a comment)",
          [l for l in workers_code if re.search(r"wasm-bindgen@[0-9]", l)], [])
    node_pin = re.search(r"^          node-version: '([0-9]+)\.([0-9]+)\.[0-9]+'$", workers_block, re.M)
    node_ok = bool(node_pin) and (int(node_pin.group(1)), int(node_pin.group(2))) >= (26, 0)
    check(f"{workers_path}: {WORKERS_JOB} pins Node 26 or later by an exact version, and not the web jobs' 22 (which asserts on the Worker's cloudflare: imports)", node_ok, True)
    check(f"{workers_path}: {WORKERS_JOB} does not take its Node from web/.node-version", [l for l in workers_code if "web/.node-version" in l], [])
    check(f"{workers_path}: {WORKERS_JOB} sets up the wasm32 target and nextest", "          nextest: 'true'\n          targets: wasm32-unknown-unknown\n" in workers_block, True)
    check(f"{workers_path}: {WORKERS_JOB} has a cache key of its own, which no other job uses",
          ("          cache-key: wasm-workers\n" in workers_block, (read_text(workers_path) or "").count("cache-key: wasm-workers\n")), (True, 1))

print(f"passed={passed} bad={failed}")
sys.exit(1 if failed else 0)
