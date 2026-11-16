"""Hand mutants of plan-ci.py and assert-gate.sh: one bug planted in a copy at a time, and the tests of that file must fail for it.
Run by ci-scripts, because a gate whose tests cannot fail is not a gate (AGENTS.md rule 8).

    python3 -I .github/scripts/tests/mutate_gate.py [--jobs N] [--only TEXT] [--list]

  a plan-ci.py mutant must fail test_plan_ci.py, run with the copy as its argument;
  an assert-gate.sh mutant must fail test-ci-scripts.sh, run under dash (Ubuntu's sh) in a copy of the tree it reads, stopping
  at the first failing case (STOP_AT_FIRST_BAD), since one answer is all it wants.

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
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.abspath(os.path.join(HERE, ".."))
ROOT = os.path.abspath(os.path.join(SCRIPTS, "..", ".."))
PLAN, GATE = "plan-ci.py", "assert-gate.sh"
SHELL = shutil.which("dash") or "sh"
TIMEOUT = 900

# (file, label, the text to replace, its replacement). The text must occur once in the file.
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
    (PLAN, "web/ does not run the journeys", '{"jobs": ["web"], "passes": ["e2e"]}, "the stock app', '{"jobs": ["web"]}, "the stock app'),
    (PLAN, "the CA root does not run hermetic", '{"jobs": ["hermetic", "ci-scripts"]}, "the CA root', '{"jobs": ["ci-scripts"]}, "the CA root'),
    (PLAN, "a crate change does not run lint", 'jobs.update(("ci-scripts", "deps", "lint", "hermetic"))',
     'jobs.update(("ci-scripts", "deps", "hermetic"))'),
    (PLAN, "an empty change list runs nothing", 'full = args.full or (None if paths else "the change list is empty")', "full = args.full"),
    (PLAN, "cells but no contract job", '    if selected:\n        jobs.add("contract")', '    if False:\n        jobs.add("contract")'),
    (PLAN, "native-pg is mapped to the Worker", '"native-pg": "thrust-server"', '"native-pg": "thrust-worker"'),
    (PLAN, "e2e.sh runs no cell", '{"passes": ["e2e"]}, "the browser journeys\' runner"', '{}, "the browser journeys\' runner"'),
    (PLAN, "an unknown script runs nothing", '(r"^scripts/", FULL, "a script this planner does not know")',
     '(r"^scripts/", {}, "a script this planner does not know")'),
    (PLAN, "the oracle's files run no hermetic", '{"jobs": ["hermetic", "deps"], "passes": ["oracle"]}', '{"jobs": ["deps"], "passes": ["oracle"]}'),
    (PLAN, "--full runs only the matrix", "        return set(JOBS), list(cells),", '        return {"contract"}, list(cells),'),
    # --- plan-ci.py: the workflow files that run the checks, and the called workflows (ADR-0102)
    (PLAN, "a workflow that runs the checks, web.yml, is not run-everything", "|documentation|security|web)\\.yml$", "|documentation|security)\\.yml$"),
    (PLAN, "a workflow that runs the checks, documentation.yml, is not run-everything", "|lint|documentation|security|web)", "|lint|security|web)"),
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
    (GATE, "the gate forgets the docs job", "mutants web docs}\"", "mutants web}\""),
    (GATE, "any JSON is an array", "jq -e 'type == \"array\" and all(.[]; type == \"string\")'", "jq -e 'true'"),
    (GATE, "an array of anything is an array of jobs", "jq -e 'type == \"array\" and all(.[]; type == \"string\")'", "jq -e 'type == \"array\"'"),
    # --- assert-gate.sh: the verdict of a called workflow, and the other levels (ADR-0102)
    (GATE, "a callee that says nothing passes", '[ "$verdict" = pass ] || fail "$job succeeded but', 'true || fail "$job succeeded but'),
    (GATE, "any verdict that is not empty passes", '[ "$verdict" = pass ] ||', '[ -n "$verdict" ] ||'),
    (GATE, "VERDICT_JOBS is ignored", 'VERDICTS="${VERDICT_JOBS:-}"', 'VERDICTS=""'),
    (GATE, "a name in VERDICT_JOBS that is no job is accepted",
     "*) fail \"VERDICT_JOBS names '$job', which is not a job the gate knows\" ;;", "*) ;;"),
    (GATE, "a verdict job that sets no planner job still needs one", 'CHANGES="${CHANGES_JOB-changes}"', 'CHANGES="${CHANGES_JOB:-changes}"'),
    (GATE, "a verdict job that sets no cell job still needs cells", 'CELLS="${CELL_JOB-contract}"', 'CELLS="${CELL_JOB:-contract}"'),
    (GATE, "KNOWN_JOBS is ignored", 'KNOWN="${KNOWN_JOBS:-ci-scripts', 'KNOWN="${XKNOWN_JOBS:-ci-scripts'),
    (GATE, "PLANNED is needed even where there is no cell job", 'if [ -n "$CELLS" ]; then', "if true; then"),
]

# Changes no behaviour, so the tests must still pass: if one is killed, the driver or the tests are broken, not strong.
EQUIVALENT = [
    (PLAN, "(equivalent) a comment is added to the planner", "FULL = \"full\"\n", "FULL = \"full\"  # a comment\n"),
    (GATE, "(equivalent) a comment is added to the gate", "set -eu\n", "set -eu\n# a comment\n"),
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


def read(name):
    with open(os.path.join(SCRIPTS, name), encoding="utf-8") as f:
        return f.read()


def judge(target, text, workdir):
    """(exit status, output) of the tests of `target` run against `text`, after the check that it still compiles."""
    if target == PLAN:
        try:
            compile(text, PLAN, "exec")
        except SyntaxError as e:
            return None, f"does not compile: {e}"
        path = os.path.join(workdir, PLAN)
        with open(path, "w", encoding="utf-8") as f:
            f.write(text)
        return plan_tests(path)
    syntax = os.path.join(workdir, "syntax.sh")
    with open(syntax, "w", encoding="utf-8") as f:
        f.write(text)
    status, out = run([SHELL, "-n", syntax])
    if status != 0:
        return None, f"does not parse: {out.strip()}"
    return gate_tests(os.path.join(workdir, "tree"), text)


def one(entry, sources):
    target, label, old, new = entry
    n = sources[target].count(old)
    if n != 1:
        return "BROKEN", f"the text to replace is found {n} times in {target}, not once"
    with tempfile.TemporaryDirectory() as workdir:
        status, out = judge(target, sources[target].replace(old, new, 1), workdir)
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

    sources = {PLAN: read(PLAN), GATE: read(GATE)}
    with tempfile.TemporaryDirectory() as workdir:
        base_plan, plan_out = judge(PLAN, sources[PLAN], workdir)
    with tempfile.TemporaryDirectory() as workdir:
        base_gate, gate_out = judge(GATE, sources[GATE], workdir)
    for name, status, out in ((PLAN, base_plan, plan_out), (GATE, base_gate, gate_out)):
        if status != 0:
            print(f"mutate_gate: the tests of {name} fail on the unmutated file, so no mutant could be judged:\n{out[-1500:]}", file=sys.stderr)
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
