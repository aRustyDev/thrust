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
    job and no cells, and sets the output only after the judge passed;
  * each of its jobs runs when the plan names it, and it can be called and run alone, with no workflow-level `concurrency` and no
    `github.event_name` (the rules R8 and R9 of lint-workflow.sh).
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
    check(f"{path}: the call passes the job's needs, the plan it was given and exactly the workflow's jobs",
          f"        with:\n          needs: ${{{{ toJSON(needs) }}}}\n          plan: ${{{{ inputs.plan }}}}\n          jobs: {' '.join(jobs)}\n" in verdict, True)
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
check(f"{action_path}: it has no planner job and no cells to judge", "        CHANGES_JOB: \"\"\n        CELL_JOB: \"\"\n" in action, True)
check(f"{action_path}: it judges with the one script", action.count("run: .github/scripts/assert-gate.sh") == 1, True)
judge_at, mark_at = action.find("run: .github/scripts/assert-gate.sh"), action.find("- id: mark")
check(f"{action_path}: the output is set after the judge, and only there", 0 < judge_at < mark_at and action.count("verdict=pass") == 1, True)
check(f"{action_path}: the mark step writes the output and nothing else, and runs only if the judge passed",
      action[mark_at:].split("\n", 1)[1:] == ['      shell: bash\n      run: echo "verdict=pass" >> "$GITHUB_OUTPUT"\n'], True)

# the jobs that stay in ci.yml run when `jobs` names them, as before
for job in (j for j in ci_jobs if j not in ("changes", "gate") and j not in pc.WORKFLOWS):
    check(f"ci.yml's {job} job runs when the planner's `jobs` names it",
          f"if: contains(fromJSON(needs.changes.outputs.jobs || '[]'), '{job}')" in block(ci_text, job), True)

print(f"passed={passed} bad={failed}")
sys.exit(1 if failed else 0)
