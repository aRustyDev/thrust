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
3. A planned job inside a called workflow is skipped by a bug in its `if:`. `gate` is red. Today a calling job reports
   `success` when every job inside it was skipped, so this is the case the existing design cannot see.
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
| 2 | 0.2 spike | a **draft PR, closed unmerged** (like probes #196 to #199); its table goes into the ADR | nothing |
| 3 | 0.1 ADR-0102 | `adrs/ci/0102-*.md` | the spike's table |
| 4 | 0.3 planner, gate, mutant driver | scripts and tests; no workflow moves | ADR-0102 |
| 5 | 0.4 lint and indexes | `lint-workflow.sh` rules, tests, `actionlint` in CI, two READMEs | ADR-0102 |

## Stage 0.5: Baseline

**Purpose.** Later passes (5 caching, 6 parallelization, 7 CPU time) claim an improvement; this is what the claim is
measured against. It is measured on the unchanged `ci`, and again after the migration to prove the migration cost nothing.

**Acceptance.**
- For the last 30 completed `ci` runs that ran past `changes` (so a docs-only run is not a sample of the full pipeline),
  split into **full** (an `rc/*` pull request, a merge queue entry, the nightly run) and **affected-only**, the document
  gives per job: runs, median and p90 duration, share of failures; per run: wall clock (`run_duration_ms`) and
  runner-minutes (the sum of the jobs' `completed_at - started_at`).
- The method is in the document, and the script re-runs it and prints the same shape.
- A control: the script run on a run list it must reject (an empty list; a run with no jobs) exits non-zero.

**Steps.** `gh api repos/aRustyDev/thrust/actions/workflows/ci.yml/runs` for the run list; `.../runs/<id>/jobs` for each job's
times; aggregate. The script is Python, run with `uv run`, kept in `.agents/plans/ci-workflows/scripts/`.

**Public solutions.** `gh run list --json` has run-level times only, not a job's, so it cannot give the per-job table; the
jobs endpoint can. No tool was found that aggregates this for a repository without a service account; the aggregation is
about 60 lines. *(Hand-written: the aggregation. Probe: the timing endpoint above, which returned zero billable.)*

## Stage 0.2: Nested-gate spike

**Purpose.** Choose, by measurement, how `gate` judges a job that lives inside a called workflow. Two candidates:

- **A, a verdict per callee.** Every called workflow ends with a `verdict` job (`if: always()`, `needs` every inner job)
  that runs the **same** `assert-gate.sh`, parameterized by its own job list and the `plan` it was given. The top `gate`
  then needs only the calling jobs. Strength: a callee run alone is judged too (use case 4). Cost: the script runs at
  every level, and each callee needs its inputs wired.
- **B, the callee reports, the gate judges.** The last job of a callee exports `toJSON(needs)` as a workflow output, and the
  one `gate` flattens every callee's results into its own check. Strength: one place judges, and `assert-gate.sh`
  changes least. Cost: a callee run alone is not judged by anything.

The expectation is **A**, because of use case 4, but the spike decides.

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
- F5 a calling job's result is `success` when every inner job was skipped.
- F6 a callee's workflow outputs reach `needs.<caller>.outputs`, and the size they may have.
- F7 an artifact uploaded in a callee can be downloaded by a later job of the caller (the contract cells' markers).
- F8 a callee cannot hold a permission its calling job did not grant.

**Acceptance.** The table is complete: seven scenarios times three candidates, and F1 to F8, each with a run id. One
candidate is chosen, with the controls it passes and the ones it does not.

## Stage 0.1: ADR-0102

**Purpose.** Record the decision in the repository's own home for decisions
([ADR-0100](../../contrail/adrs/ci/0100-build-matrix-per-adapter-set.md) and
[ADR-0101](../../contrail/adrs/ci/0101-ci-supply-chain.md) are its neighbours; 0102 is free). It does not restate how CI
works today.

**Content.** The decision: a thin orchestrator, called workflows, the judging design the spike chose. The conventions:
- a callee routes by an input, never by `github.event_name` (F1);
- no workflow-level `concurrency` in a file that has `workflow_call` (F3);
- only the permissions its caller grants (F8);
- the callee's final job is named `verdict` (decided by the owner, 2026-11-16);
- every file keeps `permissions: {}` and a `permissions:` on each job.

Alternatives not taken: a gate per workflow (N required checks, against ADR-0100's one aggregate context); `pr.yml` as the
orchestrator; `paths:` filters (a skipped workflow reports nothing); the `triggerd` planner now (its cut-over is a stage of the
Actions track, after phase 8).

**Acceptance.** Merged and `accepted`; every convention is enforced by a check (0.4) or says why it cannot be; it cites the
spike's table.

## Stage 0.3: Planner, gate and mutant driver

**Purpose.** Give the planner and the gate the mechanism the migration needs, as a **behaviour-preserving** change: when it
merges, CI plans and judges exactly what it did before.

**Acceptance.**
- `plan-ci.py` has a `WORKFLOWS` map (workflow name to its jobs), a partition of `JOBS` (a test fails if a job is in none
  or in two), and prints `workflows=<JSON list of the workflows with at least one planned job>`. Today the map is one
  entry, `ci`, holding every job, so the output is equivalent to the current one. Each migration stage then moves names
  out of `ci`.
- Its RULES name every workflow file that runs checks (`test`, `lint`, `documentation`, `security`, `web`) as **run
  everything**, as `ci` and `contract-cell` are, and `test_plan_ci.py` lists the real `.github/workflows/` directory and
  fails if a file there matches only the generic rule and is not on a short list of files that run no check (`deploy`,
  `release`, `release-plz`, `e2e`). The control: delete one of the new names from the rule and the test must fail.
  *(Narrowing a rule so that a change to `test.yml` runs only the test jobs is a Phase 2 optimization, not this.)*
- `assert-gate.sh` takes its job list as an input and is the **one** script at every level (or, under B, flattens the
  callees' results), with no copy. `same_jobs` is extended to the new places the list lives.
- `test-ci-scripts.sh` gains the callee cases: each scenario of the spike's table as a generated case, the 80 existing
  cases unchanged and green, and the control that an absent callee result is a failure and not a skip.
- `.github/scripts/tests/mutate-gate.sh` (the 34 hand mutants of the planner and the gate, each of which the tests must
  catch) is committed, runs under `dash`, and reports a mutant that survives. It is a step of the `ci-scripts` job (the
  owner's decision, 2026-11-16), so it runs whenever that job does and a surviving mutant fails the gate; the control is a
  test weakened on purpose, which must make the step fail. A change that plans no `ci-scripts` (a docs-only one) skips it, as
  it skips every other helper test.

**Steps.** Planner and gate changes with their tests first (red, then green); the rules and the directory test; the mutant
driver last, because it has to mutate the final text.

**Public solutions.** `re-actors/alls-green`: see 0.2 (rejected only if the spike shows it fails a control). Mutation
tools for Python exist (`mutmut`, `cosmic-ray`); none mutates POSIX shell, and the gate is shell. *(Hand-written: the mutant
driver. It already exists in a session scratchpad and is being rewritten into the repository, not invented.)*

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
  `contract` call with a `with:` key the callee does not declare must be refused.

**Public solutions.** `actionlint` checks a caller's `with:` against the callee's declared inputs and outputs, which matters
once there are a dozen such calls. It is **not wired into CI today** (the cicd-review plan only suggests running it by
hand), and a search result mentions an open false-positive report about reading a callee's outputs through `needs` (actionlint
#407; status not checked: check it when this stage starts, since it matters only if the spike picks the design in which a
callee reports its results). **How it is installed is decided here, at the start of the stage, by survey**: the tools
`taiki-e/install-action` already installs in `ci.yml` (cargo-deny, cargo-nextest, mdbook) against actionlint's own release and
checksum. *(Hand-written: R8 and R9, which are this repository's conventions; actionlint has no rule for them.)*

## Action candidates

None in Phase 0. The first exact duplicate will be the verdict step, created in M1 only if A is chosen and only because it is
then the same steps in several callees (asked about only if the copies are not identical).

## Decisions on the questions (owner, 2026-11-16, after #225)

| Question | Answer |
|---|---|
| The name of a callee's final job | `verdict`, so the only job named `gate` in a check list is the required one |
| The mutant driver (0.3) | Commit it and run it in `ci-scripts`. A claim checked only by hand had already lapsed once (the driver was lost with a session) |
| `actionlint` (0.4) | Add it, as a pinned check in `deps`. The public tool for the `with:` contracts the split creates |
| Prof's group | Low |

No question is open.
