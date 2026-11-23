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

# The native jobs of build.yml (Phase 3: `linux`, stage 1, and `macos`, stage 3) are compile proofs of the native server and `migrate`, built in release and uploaded. Their steps are one
# action's, `build-native`; what is each job's is the runner, a name (which keys its cache and names its artifact) and the target it is a proof for. Things about them that can be wrong and
# leave a job green: a build that is not the release profile (the debug build of the cells already exists, and a release-only break would pass), an unlocked build (it would read a
# lockfile it is free to change), a `migrate` built by another command than the deploy bundle's (the proof would say nothing about the binary that ships), a check for the binaries that
# looks for one only (a build that made no `migrate` would pass), a proof that does not check what it builds for (a job named macos that landed on a Linux runner would upload Linux
# binaries under its name), a cache key shared with another job (a release profile saved under it evicts that job's entries), and a job on the wrong runner. Each is a line, checked as
# text, and the mutant driver changes each.
# job: (its workflow, its runner, its target). `linux` is a required check (build.yml, called by ci.yml); `macos` is not (build-extended.yml, never called, never in the gate).
NATIVE = {"linux": ("build", "ubuntu-24.04", "x86_64-unknown-linux-gnu"), "macos": ("build-extended", "macos-15", "aarch64-apple-darwin")}
native_action_path = ".github/actions/build-native/action.yml"
native_action = read_text(native_action_path) or ""
check(f"{native_action_path} exists", native_action != "", True)
migrate_cmd = "run: scripts/rs cargo build --release --locked -p thrust-adapters-postgres --features native --bin migrate\n"
check(f"{native_action_path}: it builds the server in release, locked", "      run: scripts/rs cargo build --release --locked -p thrust-server\n" in native_action, True)
check(f"{native_action_path}: it builds migrate with the command the deploy bundle builds it with",
      (migrate_cmd in native_action, migrate_cmd in (read_text(".github/actions/build-bundle/action.yml") or "")), (True, True))
check(f"{native_action_path}: it builds nothing but in release (no cargo build line without --release)",
      [l for l in native_action.splitlines() if re.search(r"run: .*cargo build", l) and "--release" not in l], [])
check(f"{native_action_path}: it checks for each of the two binaries it built, and fails when one is missing",
      '        for bin in thrust-server migrate; do\n          test -x "target/release/$bin" || { echo "::error::cargo built nothing at target/release/$bin"; exit 1; }\n        done\n' in native_action, True)
check(f"{native_action_path}: it checks that the runner's Rust host is the target the job claims, before it builds anything",
      "      env:\n        TARGET: ${{ inputs.target }}\n      run: |\n        set -eu\n        host=\"$(rustc -vV | sed -n 's/^host: //p')\"\n        test \"$host\" = \"$TARGET\" || { echo \""
      in native_action and 0 < native_action.find("The runner builds for the target") < native_action.find("Build thrust-server (release)"), True)
check(f"{native_action_path}: its cache key is derived from the job's name, saved on main only",
      "        cache-key: build-${{ inputs.name }}\n        save-cache: ${{ github.ref == 'refs/heads/main' }}\n" in native_action, True)
check(f"{native_action_path}: it uploads the binaries under the job's name and target, fails when there are none, and keeps them three days",
      "        name: build-${{ inputs.name }}-${{ inputs.target }}\n        path: |\n          target/release/thrust-server\n          target/release/migrate\n"
      "        if-no-files-found: error\n        retention-days: 3\n" in native_action, True)
check(f"{native_action_path}: it has no secret, no environment and no id-token: a compile proof publishes and attests nothing",
      [l for l in native_action.splitlines() if re.search(r"secrets\.|environment:|id-token|attestations|contents: write", l) and not l.strip().startswith("#")], [])
all_text = "".join(read_text(f".github/workflows/{f}") or "" for f in sorted(os.listdir(os.path.join(ROOT, ".github", "workflows"))) if f.endswith(".yml"))
all_text += "".join(read_text(f".github/actions/{d}/action.yml") or "" for d in sorted(os.listdir(os.path.join(ROOT, ".github", "actions"))) if os.path.isdir(os.path.join(ROOT, ".github", "actions", d)))
check("the only cache key that begins with build- is the native action's, derived from a job's name (two jobs share a cache only if they share a name)",
      re.findall(r"cache-key: (build-[^\n]+)", all_text), ["build-${{ inputs.name }}"])
check("no two native jobs share a target (the artifact of each is named after its name and target)", sorted(t for _, _, t in NATIVE.values()), sorted({t for _, _, t in NATIVE.values()}))
check("the native job that is a required check, linux, is a job of a called workflow, and macos is the job of none (it is not in the gate)",
      ([w for w, jobs in pc.WORKFLOWS.items() if "linux" in jobs], [w for w, jobs in pc.WORKFLOWS.items() if "macos" in jobs], "macos" in pc.JOBS), (["build"], [], False))
for job, (job_wf, runner, target) in NATIVE.items():
    job_path = f".github/workflows/{job_wf}.yml"
    job_text = read_text(job_path) or ""
    check(f"{job_path} exists", job_text != "", True)
    job_block = block(job_text, job)
    check(f"{job_path}: {job} runs on {runner}, a pinned image", f"    runs-on: {runner}\n" in job_block, True)
    check(f"{job_path}: {job} calls build-native with its own name and target ({job}, {target}), after the checkout",
          f"      - uses: ./.github/actions/build-native\n        with:\n          name: {job}\n          target: {target}\n" in job_block
          and 0 < job_block.find("uses: actions/checkout@") < job_block.find("uses: ./.github/actions/build-native"), True)
    check(f"{job_path}: {job} has no step of its own that runs a command (the steps are the action's)",
          [l for l in job_block.splitlines() if re.match(r"^\s+(- )?run:", l)], [])
    check(f"{job_path}: {job} has no secret, no environment and no id-token: a compile proof publishes and attests nothing",
          [l for l in job_block.splitlines() if re.search(r"secrets\.|environment:|id-token|attestations|contents: write", l) and not l.strip().startswith("#")], [])

# build-extended.yml (Phase 3, stage 3; the owner's decision of 2026-11-23: the macOS build is not a required check, and runs nightly and before a release). Like deep.yml it is not a
# called workflow: ci.yml has no job for it and `gate` does not read it, so a red run blocks nothing. Each of these can go wrong and leave a workflow that still "works": being called by
# ci.yml (a red macOS build would block pull requests), being started by every pull request (each would pay a macOS build, and `build-extended` would be a check on it), losing its night or
# its release-candidate trigger (it would run less often than asked), a job with a plan or a verdict (it would be waiting for a gate that does not wait for it), and the workflow's
# permissions. Each is a line, checked as text, and the mutant driver changes each.
EXT_PATH = ".github/workflows/build-extended.yml"
ext_text = read_text(EXT_PATH)
check(f"{EXT_PATH} is there", ext_text is not None, True)
if ext_text is not None:
    ext_code = [l for l in ext_text.splitlines() if not l.strip().startswith("#")]
    check(f"{EXT_PATH} is not a called workflow: no `workflow_call`, no job of ci.yml calls it, and it has no `plan` input",
          ("workflow_call" in ext_text, f"workflows/{os.path.basename(EXT_PATH)}" in ci_text, "inputs.plan" in ext_text), (False, False, False))
    check(f"{EXT_PATH} has no verdict job, and no job needs another (nothing in it is waited for)",
          ("verdict:" in ext_text, [l for l in ext_code if re.match(r"^\s+needs:", l)]), (False, []))
    check(f"{EXT_PATH} runs every night", '  schedule:\n    - cron: "11 6 * * *"\n' in ext_text, True)
    check(f"{EXT_PATH} runs when a release candidate is pushed, and by hand", "  push:\n    branches:\n      - rc/*\n" in ext_text and "  workflow_dispatch:\n" in ext_text, True)
    check(f"{EXT_PATH} is started by a pull request only if it edits this file or the action it uses, so no other pull request pays for it",
          "  pull_request:\n    paths:\n      - .github/workflows/build-extended.yml\n      - .github/actions/build-native/**\n\npermissions: {}\n" in ext_text, True)
    check(f"{EXT_PATH} grants nothing at the top, and its jobs read the repository only", "\npermissions: {}\n" in ext_text and
          all("    permissions:\n      contents: read\n" in block(ext_text, j) for j in job_ids(ext_text)), True)
    check(f"{EXT_PATH} has one run in flight per ref, and a push or a pull request replaces the one before it",
          "concurrency:\n  group: build-extended-${{ github.ref }}\n  cancel-in-progress: ${{ github.event_name == 'push' || github.event_name == 'pull_request' }}\n" in ext_text, True)
    check(f"{EXT_PATH} has the job macos and no other (the targets that follow are added to it on purpose)", job_ids(ext_text), ["macos"])

# The bash job (Lint phase, stage 1) runs shellcheck over the scripts the repository maintains, through .github/scripts/check-shell.sh. Four things about
# it can be wrong and leave the job looking healthy until the runner differs from a laptop: a shellcheck installed at a version the helper is not held to (the
# helper refuses it, but only on the runner), no install at all (the image's 0.9.0 answers, and the helper refuses that too), a run that is not the helper (which
# carries the control, and the list of what is not ours), and a Rust toolchain and cache the job does not use. Each is a line, checked as text.
BASH_JOB = "bash"
bash_wf = next((w for w, jobs in pc.WORKFLOWS.items() if BASH_JOB in jobs), None)
check(f"the job {BASH_JOB} is a job of a called workflow", bash_wf is not None, True)
if bash_wf:
    bash_path = f".github/workflows/{bash_wf}.yml"
    bash_block = block(read_text(bash_path) or "", BASH_JOB)
    bash_code = [l for l in bash_block.splitlines() if not l.strip().startswith("#")]
    helper = read_text(".github/scripts/check-shell.sh") or ""
    held_to = re.search(r'^SHELLCHECK_VERSION="([^"]+)"$', helper, re.M)
    check("check-shell.sh names the version it is held to, once", len(re.findall(r'^SHELLCHECK_VERSION="', helper, re.M)), 1)
    installs = re.findall(r"^          tool: shellcheck@(\S+)$", bash_block, re.M)
    check(f"{bash_path}: {BASH_JOB} installs shellcheck at the version check-shell.sh is held to ({held_to.group(1) if held_to else None})",
          installs == [held_to.group(1)] if held_to else None, True)
    check(f"{bash_path}: {BASH_JOB} installs it with install-action, pinned to a commit",
          bool(re.search(r"^      - uses: taiki-e/install-action@[0-9a-f]{40} # v\S+\n        with:\n          tool: shellcheck@", bash_block, re.M)), True)
    check(f"{bash_path}: {BASH_JOB} runs the helper, which carries the control, and not shellcheck by hand", "        run: sh .github/scripts/check-shell.sh\n" in bash_block, True)
    check(f"{bash_path}: {BASH_JOB} runs no other command", len([l for l in bash_code if re.match(r"^\s+run:", l)]), 1)
    check(f"{bash_path}: {BASH_JOB} sets up no Rust toolchain or cache (it reads shell)", [l for l in bash_code if "setup-rust" in l or "cache" in l], [])

# The deps job runs actionlint, and actionlint runs shellcheck over the shell of every workflow's `run:` blocks: the one on PATH, which on the image is 0.9.0 and does not know
# SC2329. So the workflows' shell was read by an older shellcheck than the scripts' (Lint phase, stage 1, probe C). Four things can be wrong and leave the job green: no install (the
# image's answers, and only lint-actionlint.sh's control refuses it, at run time), an install at a version that is not the one the scripts are held to, an install AFTER the actionlint
# step (PATH is read when it runs), and an actionlint run by hand and not through lint-actionlint.sh (which hands it the shellcheck and carries the control). Each is a line, as text.
DEPS_JOB = "deps"
deps_wf = next((w for w, jobs in pc.WORKFLOWS.items() if DEPS_JOB in jobs), None)
check(f"the job {DEPS_JOB} is a job of a called workflow", deps_wf is not None, True)
if deps_wf and bash_wf:
    deps_path = f".github/workflows/{deps_wf}.yml"
    deps_block = block(read_text(deps_path) or "", DEPS_JOB)
    deps_code = [l for l in deps_block.splitlines() if not l.strip().startswith("#")]
    deps_installs = re.findall(r"^          tool: shellcheck@(\S+)$", deps_block, re.M)
    check(f"{deps_path}: {DEPS_JOB} installs shellcheck once, at the version check-shell.sh is held to ({held_to.group(1) if held_to else None})",
          deps_installs == [held_to.group(1)] if held_to else None, True)
    check(f"{deps_path}: {DEPS_JOB} installs it with install-action, pinned to a commit, the same one {BASH_JOB} uses",
          re.findall(r"^      - uses: (taiki-e/install-action@[0-9a-f]{40}) # v\S+\n        with:\n          tool: shellcheck@", deps_block, re.M)
          == re.findall(r"^      - uses: (taiki-e/install-action@[0-9a-f]{40}) # v\S+\n        with:\n          tool: shellcheck@", bash_block, re.M)
          and len(re.findall(r"^      - uses: taiki-e/install-action@", deps_block, re.M)) == 1, True)
    install_at = next((i for i, l in enumerate(deps_code) if "tool: shellcheck@" in l), -1)
    actionlint_at = next((i for i, l in enumerate(deps_code) if "lint-actionlint.sh" in l), -1)
    check(f"{deps_path}: {DEPS_JOB} installs shellcheck before the step that runs actionlint", 0 <= install_at < actionlint_at, True)
    check(f"{deps_path}: {DEPS_JOB} runs actionlint through lint-actionlint.sh, once, and not by hand",
          (len([l for l in deps_code if "lint-actionlint.sh" in l]), [l for l in deps_code if re.match(r"^\s+\"?(\$RUNNER_TEMP/)?actionlint\"?(\s|$)", l)]), (1, []))

# The yaml job (Lint phase, stage 2) runs yamllint over the YAML the repository maintains, through .github/scripts/check-yaml.sh. yamllint is on no image and install-action has no
# manifest for it, so it is installed as Schemathesis is in contract-pass-tools: a pinned setup-uv and a pinned version. Five things about it can be wrong and leave the job looking
# healthy until the runner differs from a laptop: a yamllint at a version the helper is not held to (the helper refuses it, but only on the runner), no install (the helper refuses
# that too), an install after the helper (PATH is read when it runs), a uv that is not the one the repository pins (a second uv to keep), and a run that is not the helper (which carries
# the control, and the list of what is not ours). Each is a line, checked as text.
YAML_JOB = "yaml"
yaml_wf = next((w for w, jobs in pc.WORKFLOWS.items() if YAML_JOB in jobs), None)
check(f"the job {YAML_JOB} is a job of a called workflow", yaml_wf is not None, True)
if yaml_wf:
    yaml_path = f".github/workflows/{yaml_wf}.yml"
    yaml_block = block(read_text(yaml_path) or "", YAML_JOB)
    yaml_code = [l for l in yaml_block.splitlines() if not l.strip().startswith("#")]
    yaml_helper = read_text(".github/scripts/check-yaml.sh") or ""
    yaml_held = re.search(r'^YAMLLINT_VERSION="([^"]+)"$', yaml_helper, re.M)
    check("check-yaml.sh names the version it is held to, once", len(re.findall(r'^YAMLLINT_VERSION="', yaml_helper, re.M)), 1)
    yaml_installs = re.findall(r"^        run: uv tool install yamllint==(\S+)$", yaml_block, re.M)
    check(f"{yaml_path}: {YAML_JOB} installs yamllint once, at the version check-yaml.sh is held to ({yaml_held.group(1) if yaml_held else None})",
          yaml_installs == [yaml_held.group(1)] if yaml_held else None, True)
    uv_pin = r"astral-sh/setup-uv@[0-9a-f]{40} # v\S+"
    check(f"{yaml_path}: {YAML_JOB} installs uv with setup-uv, pinned to the commit and the uv version contract-pass-tools pins, with no cache",
          (re.findall(rf"^      - uses: ({uv_pin})\n        with:\n          version: \"(\S+)\"\n          enable-cache: false\n", yaml_block, re.M)
           == re.findall(rf"^      uses: ({uv_pin})\n      with:\n        version: \"(\S+)\"\n        enable-cache: false\n", read_text(".github/actions/contract-pass-tools/action.yml") or "", re.M)
           != []), True)
    yaml_install_at = next((i for i, l in enumerate(yaml_code) if "uv tool install yamllint" in l), -1)
    yaml_run_at = next((i for i, l in enumerate(yaml_code) if "check-yaml.sh" in l), -1)
    check(f"{yaml_path}: {YAML_JOB} installs yamllint before the step that runs the helper", 0 <= yaml_install_at < yaml_run_at, True)
    check(f"{yaml_path}: {YAML_JOB} runs the helper, which carries the control, and not yamllint by hand", "        run: sh .github/scripts/check-yaml.sh\n" in yaml_block, True)
    check(f"{yaml_path}: {YAML_JOB} runs no other command than the install and the helper", len([l for l in yaml_code if re.match(r"^\s+run:", l)]), 2)
    check(f"{yaml_path}: {YAML_JOB} sets up no Rust toolchain and no cache of its own (it reads YAML)", [l for l in yaml_code if "setup-rust" in l or "rust-cache" in l or "actions/cache" in l], [])

# The wasm job (Phase 3, stage 2) builds the two Workers the deploy bundle ships, in release, with the bundle's own commands, and shares the bundle's cache key so that the next
# release candidate or tag restores what `main` saved. Six things about it can be wrong and leave the job green: a Worker built by another command than the bundle's (the proof would be of
# something that does not ship), a Worker built in another directory, a check that looks at one of the Workers or one of the three files only (a build that wrote no wasm would pass), a cache key that
# is not the bundle's (the job would work and help nobody), a cache key saved on every ref (a pull request's entry would evict `main`'s), and no wasm32 target (the build would fail, but only after the
# cache and the tools). Each is a line, checked as text, and the mutant driver changes each.
WASM_JOB = "wasm"
wasm_wf = next((w for w, jobs in pc.WORKFLOWS.items() if WASM_JOB in jobs), None)
check(f"the job {WASM_JOB} is a job of a called workflow", wasm_wf is not None, True)
if wasm_wf:
    wasm_path = f".github/workflows/{wasm_wf}.yml"
    wasm_text = read_text(wasm_path) or ""
    wasm_block = block(wasm_text, WASM_JOB)
    bundle_text = read_text(".github/actions/build-bundle/action.yml") or ""

    def stripped(text):
        return [l.strip() for l in text.splitlines()]

    def has_sequence(text, sequence):
        """Whether these lines, stripped of their indentation, follow one another in the text."""
        lines = stripped(text)
        return any(lines[i:i + len(sequence)] == sequence for i in range(len(lines)))

    for label, directory, command in (("the Worker", "crates/worker", "../../scripts/rs worker-build --release . --no-default-features"),
                                      ("the edge Worker", "crates/edge", "../../scripts/rs worker-build --release .")):
        sequence = [f"working-directory: {directory}", f"run: {command}"]
        check(f"{wasm_path}: {WASM_JOB} builds {label} in {directory} with the command the deploy bundle builds it with",
              (has_sequence(wasm_block, sequence), has_sequence(bundle_text, sequence)), (True, True))
    check(f"{wasm_path}: {WASM_JOB} runs worker-build in release only (no worker-build line without --release)",
          [l for l in wasm_block.splitlines() if re.search(r"run: .*worker-build", l) and "--release" not in l], [])
    expected_files = ('          for dir in crates/worker/build crates/edge/build; do\n            for file in worker/shim.mjs index.js index_bg.wasm; do\n'
                      '              test -s "$dir/$file" || { echo "::error::worker-build wrote no $dir/$file"; exit 1; }\n')
    check(f"{wasm_path}: {WASM_JOB} checks the three files worker-build writes, for both Workers, and fails when one is missing", expected_files in wasm_block, True)
    check(f"{wasm_path}: {WASM_JOB} checks what the bundle checks (shim.mjs, index.js and index_bg.wasm of the Worker, and the same three of the edge)",
          all(f"test -s crates/{w}/build/{f}" in bundle_text for w in ("worker", "edge") for f in ("worker/shim.mjs", "index.js", "index_bg.wasm")), True)
    check(f"{wasm_path}: {WASM_JOB} builds for wasm32 with the Worker tools, a cache key saved on main only",
          "          targets: wasm32-unknown-unknown\n          worker-tools: \"true\"\n          cache-key: deploy-release\n"
          "          save-cache: ${{ github.ref == 'refs/heads/main' }}\n" in wasm_block, True)
    users = "".join(read_text(f".github/workflows/{f}") or "" for f in sorted(os.listdir(os.path.join(ROOT, ".github", "workflows"))) if f.endswith(".yml"))
    users += "".join(read_text(f".github/actions/{d}/action.yml") or "" for d in sorted(os.listdir(os.path.join(ROOT, ".github", "actions"))) if os.path.isdir(os.path.join(ROOT, ".github", "actions", d)))
    check(f"{wasm_path}: the cache key deploy-release has two users, the {WASM_JOB} job that saves it and build-bundle that restores it, and no other job",
          (users.count("cache-key: deploy-release\n"), "cache-key: deploy-release\n" in bundle_text), (2, True))
    check(f"{wasm_path}: build-bundle only restores that cache (it saves none: its `save-cache` is its caller's input, which defaults to false)",
          "save-cache: ${{ inputs.save-cache }}" in bundle_text and 'default: "false"' in bundle_text, True)
    check(f"{wasm_path}: {WASM_JOB} uploads both Workers, fails when there are none, and keeps them three days",
          "          name: build-wasm-wasm32-unknown-unknown\n          path: |\n            crates/worker/build\n            crates/edge/build\n"
          "          if-no-files-found: error\n          retention-days: 3\n" in wasm_block, True)
    check(f"{wasm_path}: {WASM_JOB} has no secret, no environment and no id-token: a compile proof publishes and attests nothing",
          [l for l in wasm_block.splitlines() if re.search(r"secrets\.|environment:|id-token|attestations|contents: write", l) and not l.strip().startswith("#")], [])

print(f"passed={passed} bad={failed}")
sys.exit(1 if failed else 0)
