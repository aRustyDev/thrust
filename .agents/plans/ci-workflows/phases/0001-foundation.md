# Phase 0001: Foundation

Part of [ci-workflows](../PLAN.md). Planned 2026-11-16; nothing built. Facts are from `main` at `b8702d3`, read on that
day: recount them when a stage starts. Phase 0 makes the split safe before a single job moves: it fixes how a called
workflow is judged by `gate`, teaches the planner about workflows, and records what CI costs today so that later passes
have something to be measured against.

Phase 0 has **no job**, so the eight passes do not apply to it; each stage's evidence is its acceptance criteria. The
passes start in Phase 2, on the migrated layout.

## Purpose and use cases

The one required `gate` must keep meaning "everything the change needs ran, and passed", with the work spread over many
files. Four cases the foundation has to get right:

1. A maintainer adds a job to `test.yml`. The planner and the gate pick it up, or the gate fails; it is never silently
   not run.
2. A change touches only `deny.toml`. Only the Security workflow runs, and `gate` passes.
3. A planned job inside a called workflow is skipped by a bug in its `if:`. `gate` is red. A calling job reports
   `success` when at least one job inside it succeeded, so with two planned jobs and one skipped it is green, and the
   existing design cannot see it ([research 0018](../research/0018-gate-spike.md), F5).
4. `test.yml` is run alone (dispatch or schedule). Its result is judged too, not only when `ci` calls it.

## What exists today (read 2026-11-16)

| Fact | Where | Consequence |
|---|---|---|
| The gate's job list is hard-coded: `KNOWN="ci-scripts deps lint deny hermetic pg-adapter contract mutants web docs"`, equal to `plan-ci.py`'s `JOBS`, checked by `same_jobs` in `test-ci-scripts.sh` | `assert-gate.sh:34`, `plan-ci.py:43` | The gate needs the list as an input, or one copy per level |
| The gate reads cell markers (`cell-ok-*`) for `contract` itself | `assert-gate.sh:67-86` | The marker rule stays where the contract matrix lives |
| `plan-ci.py` runs everything for `ci.yml` and `contract-cell.yml` only. Any other file under `.github/workflows/` falls into the last rule and plans just `ci-scripts` and `deps` | `plan-ci.py:64`, `:100` | **A change to the new `test.yml` would not run the tests.** The rules must name the new files before M1, with a test that fails if a workflow is left to the generic rule |
| `lint-workflow.sh` is a text scanner (R1 to R7). It accepts local `uses: ./...`, wants `permissions: {}` and a `permissions:` on every job, and `lint-all-workflows.sh` gives any workflow it does not name the strictest credentials policy | `lint-workflow.sh`, `lint-all-workflows.sh:17-24` | Called workflows that hold no credentials pass **as they are**. No per-callee registry is needed; the new work is rules for the conventions |
| `contract-cell.yml` is already a called workflow (`workflow_call`, `permissions: {}`, no `concurrency`, inputs only through `env`) | `contract-cell.yml` | The pattern exists at depth 2; `ci` → `test` → `contract-cell` is depth 3 |
| `assert-sha-checks.sh` matches a check by exact name (`gate`), and a re-run needs every report of it to be SUCCESS | `assert-sha-checks.sh:47` | A callee job named `gate` would show as `<caller> / gate`, which does not collide, but two jobs called "gate" in a check list is a trap for a reader |
| GitHub reports **0 billable minutes** here: `"billable":{"UBUNTU":{"total_ms":0,"jobs":11}}` beside `run_duration_ms: 51000` (run 43942132459, a public repository) | `gh api .../actions/runs/<id>/timing` | Passes 5 to 7 and stage 0.5 measure **runner-minutes** (the sum of job durations) and wall clock, not billable minutes |
| `cicd.md` says 34 hand mutants of the planner and the gate are caught, "verified locally, under dash; not yet in CI". The driver was a session scratchpad file and is not in the repository | `cicd.md:52` | Invariant 6 cannot be checked by anyone else. Stage 0.3 commits a driver |
| `flagship.yml` (#229, merged after this phase was planned) is a deploy-adjacent workflow: its own environment and secrets, push-to-main and nightly triggers, nothing `gate` waits for | `.github/workflows/flagship.yml` | It goes on the list of files that run no check (0.3) and belongs to the Deploy phase |

## Acceptance criteria for the phase

Done when:
- ADR-0102 is merged, `accepted`, and records the design the spike chose;
- the spike's controls have each been shown (a table of scenario, expected, observed, run id);
- the planner and the gate handle callees, with the cases listed in 0.3, and the mutant driver is in the repository;
- the lint conventions are enforced, `actionlint` runs in CI, and the two indexes exist;
- the baseline is published;
- and the migration (Phase 1) can start with **no further change to the planner, the gate or the lint** other than the
  moves themselves.

## Order

The stage numbers are the plan's; the order is **0.5, 0.2, 0.1, 0.3, 0.4**. The baseline is first because it measures the
CI that nothing has yet changed. The spike decides the design that the ADR records, so it comes before the ADR. The
planner and the gate follow the ADR. The lint rules need the ADR's conventions, so they come last and can run beside 0.3.

| PR | Stage | Lands as | Needs |
|---|---|---|---|
| 1 | 0.5 baseline | `research/0017-ci-baseline.md` + its script | nothing |
| 2 | 0.2 spike | a **draft PR, closed unmerged** (#232, like probes #196 to #199); its results are `research/0018-gate-spike.md`, which the ADR cites | nothing |
| 3 | 0.1 ADR-0102 | `adrs/ci/0102-*.md` | `research/0018` |
| 4 | 0.3 planner, gate, mutant driver | scripts and tests; no workflow moves | ADR-0102 |
| 5 | 0.4 lint and indexes | `lint-workflow.sh` rules, tests, `actionlint` in CI, two READMEs | ADR-0102 |

## Stage 0.5: Baseline

**Purpose.** Later passes (5 caching, 6 parallelization, 7 CPU time) claim an improvement; this is what the claim is
measured against. It is measured on the unchanged `ci`, and again after the migration to prove the migration cost nothing.

**Acceptance** (refined when the stage started, from what the API returned).
- The last 30 runs of each class among the completed `ci` runs created since the job set last changed (#206,
  2026-11-16T15:51:24Z; an older run has other jobs). The classes are decided by **what ran**, not what triggered the run:
  **full** (every job group ran), **affected-only**, and **nothing** (counted, not sampled). Per job: runs, median and p90
  duration, failures among its executions. Per run: the wall clock of the final attempt, split into the wait before the
  first job and the execution span; the runner-seconds of every execution; and the part of those spent in jobs that failed.
- An execution is a job name and its start time, because GitHub lists a job that passed in an earlier attempt again under
  each later one, with a new id and its original times. A re-run's failed attempts count; a carried-over job counts once.
- The method is in the document, and the script re-runs it and prints the same shape.
- The controls: the script refuses an empty list of runs, a run with no jobs, a job with no conclusion, a negative
  duration, an unknown job group and a malformed `--since`, and retries only a server error. Its self-test (38 checks) and
  11 bugs planted one at a time, each caught, show that those refusals can fail.
- The wall clock agrees with GitHub's `run_duration_ms` on three runs (within a second).

**Done 2026-11-16**, in [research/0017-ci-baseline.md](../research/0017-ci-baseline.md) and
[scripts/ci-baseline.py](../scripts/ci-baseline.py).

**Steps.** `gh api repos/aRustyDev/thrust/actions/workflows/ci.yml/runs` for the run list; `.../runs/<id>/jobs` for each job's
times; aggregate. The script is Python, run with `uv run`, kept in `.agents/plans/ci-workflows/scripts/`.

**Public solutions.** `gh run list --json` has run-level times only, not a job's, so it cannot give the per-job table; the
jobs endpoint can. No tool was found that aggregates this for a repository without a service account; the aggregation is a
small script with a self-test. *(Hand-written: the aggregation. Probe: the timing endpoint above, which returned zero
billable.)*

## Stage 0.2: Nested-gate spike

**Purpose.** Choose, by measurement, how `gate` judges a job that lives inside a called workflow. Two candidates:

- **A, a verdict per callee.** Every called workflow ends with a `verdict` job (`if: always()`, `needs` every inner job)
  that runs the **same** `assert-gate.sh`, parameterized by its own job list and the `plan` it was given. The top `gate`
  then needs only the calling jobs. Strength: a callee run alone is judged too (use case 4). Cost: the script runs at
  every level, and each callee needs its inputs wired.
- **B, the callee reports, the gate judges.** The last job of a callee exports `toJSON(needs)` as a workflow output, and the
  one `gate` flattens every callee's results into its own check. Strength: one place judges, and `assert-gate.sh`
  changes least. Cost: a callee run alone is not judged by anything.

The expectation was **A**, because of use case 4. The spike found A one case short (its own `verdict` job can be skipped) and
chose **A+**: A, with the callee's `verdict` output required by `gate`.

**Mechanism.** `workflow_dispatch` needs the file on the default branch (GitHub's docs; re-check when this stage starts),
so a spike that exists only on a branch is triggered by `pull_request` with a `paths:` filter on its own files, as `e2e-schedule.yml` does. It is a draft pull
request, closed unmerged. A `spike-callee.yml` takes `plan` and `scenario`; a `spike-caller.yml` calls it once per
scenario with `continue-on-error: true`, and its last job asserts each scenario's result **equals what it should be**. That
last job is the control that cannot pass vacuously: if every scenario came back green, it fails.

| Scenario | The bug it stands for | Gate must be |
|---|---|---|
| green | none | success |
| planned job skipped | an `if:` that disagrees with the plan (use case 3) | failure |
| planned job failed | an ordinary failure | failure |
| unplanned job ran | the planner and an `if:` disagree the other way | failure |
| callee skipped though planned | the calling job's `if:` is wrong | failure |
| callee never reached its verdict | cancelled, timed out | failure |
| plan input malformed | a planner bug | failure |

Run **both** A and B on all seven. Run `re-actors/alls-green` (pinned to a commit) on the same seven as a third column,
and record which it fails: it is a public aggregate gate, and the claim that it cannot express a per-run planned set rests on its
README (inputs `jobs`, `allowed-failures`, `allowed-skips`, all static lists) until this shows it.

**Facts to measure** (each is something the plan assumes; each gets a line in the table with the run it came from):
- F1 `github.event_name` inside a callee is the caller's event.
- F2 `github.workflow` inside a callee is the caller's name.
- F3 a workflow-level `concurrency` in a callee cancels or waits on its caller.
- F4 a callee's job appears as `<caller job> / <callee job>` in `gh api .../check-runs` (what `assert-sha-checks.sh` reads).
- F5 a calling job's result is `success` when every inner job was skipped. *(Measured: no. It is `success` when one inner job
  succeeded and a planned one was skipped, and `skipped` when none ran.)*
- F6 a callee's workflow outputs reach `needs.<caller>.outputs`, and the size they may have.
- F7 an artifact uploaded in a callee can be downloaded by a later job of the caller (the contract cells' markers).
- F8 a callee cannot hold a permission its calling job did not grant.

**Acceptance.** The table is complete: seven scenarios times three candidates, and F1 to F8, each with a run id. One
candidate is chosen, with the controls it passes and the ones it does not.

**Done 2026-11-16**, in [research/0018-gate-spike.md](../research/0018-gate-spike.md): five designs (the three candidates,
A+, and a `naive` control) against seven scenarios, F1 to F8 and three facts found on the way, in two identical samples.
Chosen: **A+**, right on all seven. `naive` was wrong on three (the control can fail), A on one, `alls-green` on two.

## Stage 0.1: ADR-0102

**Purpose.** Record the decision in the repository's own home for decisions
([ADR-0100](../../contrail/adrs/ci/0100-build-matrix-per-adapter-set.md) and
[ADR-0101](../../contrail/adrs/ci/0101-ci-supply-chain.md) are its neighbours; 0102 is free). It does not restate how CI
works today.

**Content.** The decision: a thin orchestrator, called workflows, the judging design the spike chose. The conventions:
- a callee routes by an input, never by `github.event_name`, which is the caller's (F1);
- no workflow-level `concurrency` in a file that has `workflow_call`: its group is built from the caller's name, equals the
  caller's, and the run starts no job (F3);
- only the permissions its caller grants: asking for more stops the run from starting (F8);
- the callee's final job is named `verdict` (decided by the owner, 2026-11-16), runs `if: always()`, and sets the output
  `verdict=pass` only after the judge passed; `gate` requires that output as well as the calling job's result (design A+);
- every file keeps `permissions: {}` and a `permissions:` on each job.

Alternatives not taken: a gate per workflow (N required checks, against ADR-0100's one aggregate context); `pr.yml` as the
orchestrator; `paths:` filters (a skipped workflow reports nothing); the `triggerd` planner now (its cut-over is a stage of the
Actions track, after phase 8).

**Acceptance.** Merged and `accepted`; every convention is enforced by a check (0.4) or says why it cannot be; it cites the
spike's table.

**Done 2026-11-16**, in [ADR-0102](../../contrail/adrs/ci/0102-one-workflow-per-concern-and-the-verdict-job.md): `accepted`, with
the six conventions, each tied to its fact, and a Confirmation that names the check for each (the ones of 0.3 and 0.4 are in
place when those stages merge; two are enforced by GitHub itself). ADR-0100 has a dated note and the ADR index a row.

## Stage 0.3: Planner, gate and mutant driver

**Purpose.** Give the planner and the gate the mechanism the migration needs, as a **behaviour-preserving** change: when it
merges, CI plans and judges exactly what it did before.

**Acceptance** (refined while it was built; **done 2026-11-16**, in this stage's pull request).
- `plan-ci.py` has a `WORKFLOWS` map, `{called workflow: (its jobs)}`, empty today: a job in no workflow is still a job of
  `ci.yml` itself. (The first plan made `ci` a one-entry map holding every job, which makes `ci` mean two things.)
  `check_workflows` refuses a map that is not a partition (a name that is no job, a job in two workflows, a workflow with no
  job) before the planner prints a line, and the test refuses the same. Beside `jobs`, `matrix` and `cells`, which do not
  change, it prints `top` (what `ci.yml`'s own gate expects: the planned jobs still in `ci.yml`, then the called workflows
  that have one), `workflows` (the called workflows with a planned job) and `plans` (each one's planned jobs, which is its
  `plan` input). While the map is empty `top` equals `jobs`, `workflows` is `[]` and `plans` is `{}`, so CI plans what it
  did; once a stage fills the map, Phase 1 needs no further change to the planner.
- Its RULES name every workflow file that runs checks (`test`, `lint`, `documentation`, `security`, `web`) as **run
  everything**, as `ci` and `contract-cell` are, and `test_plan_ci.py` lists the real `.github/workflows/` directory and
  fails if a file there matches only the generic rule and is not on a short list of files that run no check (`deploy`,
  `release`, `release-plz`, `e2e-schedule`, `flake-hunt`, `flagship`). The control is a mutant of the driver below: a name
  deleted from the rule must fail the test.
  *(Narrowing a rule so that a change to `test.yml` runs only the test jobs is a Phase 2 optimization, not this.)*
- `assert-gate.sh` is the **one** script at every level: the top-level `gate` and each callee's `verdict` job run it, with no
  copy. It takes four optional inputs, each defaulting to what `ci.yml`'s gate needs, so a level that forgets one fails and
  does not pass: `KNOWN_JOBS`, `CHANGES_JOB` (set and empty: none, as in a called workflow), `CELL_JOB` (the same) and
  `VERDICT_JOBS`, the expected jobs that call a workflow, each of which must also carry the output `verdict` equal to `pass`
  (design A+; both are in `toJSON(needs)`, which the script already reads). `same_jobs` reads the default list from its new
  place in the script. No workflow calls another yet, so `ci.yml`'s gate is run as before, and its 109 cases are unchanged.
- `test-ci-scripts.sh` has 173 cases: the callee-level scenarios of the spike (green, skip-planned, fail-planned,
  unplanned-ran, malformed-plan) and every job of a callee with every result; a gate that calls workflows, with every result
  of the callee against every kind of `verdict` (`pass`, `fail`, empty, `{}` and no outputs at all), so that only a success
  that says `pass` passes; and the controls that a verdict job that forgets one of its three inputs fails.
- `.github/scripts/tests/mutate_gate.py` (Python, not shell: the mutant table is clearer there, and the mutants run in
  parallel) is committed: the 34 hand mutants of the first driver, rebased on today's source, and 19 for this stage, 53 in all,
  each of which must fail the tests of its file (`test-ci-scripts.sh` under dash). It is the last line of the `ci-scripts`
  job (the owner's decision, 2026-11-16), so it runs whenever that job does and a surviving mutant fails the gate. The first
  run found a real gap (a CLI test that could not tell a working `plans` line from a broken one while the map is empty);
  it is closed. Controls: the tests must pass unmutated first; the text a mutant replaces must be found exactly once; a
  mutant must compile; and two mutants that change nothing must survive. The mutants of a rule's effect find the rule by its
  pattern, not by the text of its pass list: an hour after the first run, `#238` added `e2e-mail` to four rules, two literal
  anchors went BROKEN in CI (the driver failing closed, as it should) and became regexes. It takes 77 s at four workers on a
  laptop; on a hosted runner `ci-scripts` took 119 s against a median of 71 s, so it added under a minute, and pass 7 may
  shorten it. A change that plans no `ci-scripts` (a docs-only one) skips it, as it skips
  every other helper test.

**Steps.** Planner and gate changes with their tests first (red, then green); the rules and the directory test; the mutant
driver last, because it has to mutate the final text.

**Public solutions.** `re-actors/alls-green` was measured in 0.2: wrong on two of seven scenarios (an unplanned job that ran; a judge that was
skipped), and it needs a hand-written step to say what may be skipped, so it replaces nothing. Mutation
tools for Python exist (`mutmut`, `cosmic-ray`); none mutates POSIX shell, and the gate is shell. *(Hand-written: the mutant
driver, rewritten from the one that survived in a session scratchpad.)*

## Stage 0.4: Lint conventions and indexes

**Purpose.** Make the ADR's conventions mechanical, so a callee that breaks one fails `deps` rather than a review.

**Acceptance.**
- `lint-workflow.sh` gains **R8** (a file whose `on:` has `workflow_call` has no workflow-level `concurrency:`) and **R9**
  (it does not read `github.event_name`), each with a case in `test-lint-workflow.sh` that must fail and a case that must
  pass. The current files, `contract-cell.yml` included, stay clean.
- `.github/workflows/README.md` and `.github/actions/README.md` exist: one row per file (what it is, its triggers, whether
  `gate` waits for it), linking `cicd.md` for the how. `plan-ci.py` reads these two as no job's input, with a test.
- `lint-all-workflows.sh` needs **no** change for credentials (above); a test shows a new callee with no credentials passes
  with no entry.
- `actionlint` runs in CI at a pinned version, as a step of `deps` (the owner's decision, 2026-11-16: add it). Every
  workflow in the repository passes it, or each finding it keeps is named with the reason. The control: a copy of the
  `contract` call with a `with:` key the callee does not declare must be refused. It runs inside the checkout: outside a
  git repository `actionlint` checked nothing and printed nothing (research 0018, F11), and the control is what shows it is looking.

**Public solutions.** `actionlint` checks a caller's `with:` against the callee's declared inputs and outputs, which matters
once there are a dozen such calls. It is **not wired into CI today** (the cicd-review plan only suggests running it by
hand), and a search result mentioned an open false-positive report about reading a callee's outputs through `needs` (actionlint
#407). It did not appear on the spike's reads of a declared output, and the same tool refused a read of an undeclared one
(research 0018, F11). **How it is installed is decided here, at the start of the stage, by survey**: the tools
`taiki-e/install-action` already installs in `ci.yml` (cargo-deny, cargo-nextest, mdbook) against actionlint's own release and
checksum. *(Hand-written: R8 and R9, which are this repository's conventions; actionlint has no rule for them.)*

## Action candidates

None in Phase 0. The first exact duplicate will be the `verdict` job's steps (checkout, judge, mark the output), the same in
every callee under the chosen design: created as a composite action in M1 without asking (asked about only if the copies
are not identical).

## Decisions on the questions (owner, 2026-11-16, after #225)

| Question | Answer |
|---|---|
| The name of a callee's final job | `verdict`, so the only job named `gate` in a check list is the required one |
| The mutant driver (0.3) | Commit it and run it in `ci-scripts`. A claim checked only by hand had already lapsed once (the driver was lost with a session) |
| `actionlint` (0.4) | Add it, as a pinned check in `deps`. The public tool for the `with:` contracts the split creates |
| Prof's group | Low |

No question is open.
