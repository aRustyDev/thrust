---
status: accepted
date: 2026-11-16
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0102: One workflow per concern, called by a thin `ci`, each judged by a `verdict` job

## Context and Problem Statement

All the checks run from one workflow, `ci.yml`, beside seven others. The owner wants one workflow per concern (Test, Lint, Docs,
Security, Web, E2E, Build, Release, Deploy, PR and the rest), so that each can be understood, run on its own and improved on its
own ([the plan](../../../ci-workflows/PLAN.md)). The split must keep what ADR-0100 and ADR-0110 rely on: `main` requires one
check, `gate`; `deploy.yml` starts from `workflow_run` of `ci` and asks for `gate` by name on the commit; and `gate` proves that
the planned jobs ran, never that "nothing failed".

The split puts a job inside a workflow that another workflow calls, and there the existing proof has a hole. A calling job's
result is `success` when one job inside it succeeded even if a planned sibling was skipped, so `gate` cannot see the skip. The
rest of this ADR is how to split the workflow without opening that hole, and the rules a called workflow must follow.

## Decision Drivers

* `gate` stays the one required context, with its name, and `deploy.yml`'s trigger and `assert-sha-checks.sh` keep working
  (ADR-0100, ADR-0110).
* A planned job that never ran must turn `gate` red, at every level of nesting. A skipped job is not a pass.
* A concern's workflow can also run alone (dispatch, schedule), and is judged there too.
* Least privilege: no secret, environment or `id-token` outside the jobs that need them, and no `pull_request_target`
  (`lint-all-workflows.sh`).
* Built-ins first (the owner's standing rule): a public tool is used where it does the job, and the hand-written part is named.
* GitHub's limits: 10 levels of nesting and 50 called workflows from one top-level file (the changelog of November 2025), and a
  called workflow sits directly in `.github/workflows/`.

## Considered Options

* Option 1: a thin `ci.yml` that calls one workflow per concern and keeps `gate`; each called workflow ends with a `verdict` job
* Option 2: a gate in every workflow, each a required check
* Option 3: `pr.yml` as the orchestrator
* Option 4: independent workflows with `paths:` filters and no orchestrator
* Option 5: keep one `ci.yml`

## Decision Outcome

**Chosen option: Option 1, judged by design A+** (the owner, 2026-11-16, for the thin `ci.yml`, the name `verdict` and the
order of the work; the spike of 2026-11-16, [research 0018](../../../ci-workflows/research/0018-gate-spike.md), for how a
called workflow is judged).

### The shape

* **`ci.yml`** (`name: ci`) holds `changes` (the planner), one calling job per concern workflow, and `gate`. The ruleset on
  `main`, the `workflow_run` trigger of `deploy.yml` and the name `gate` do not change.
* **A concern workflow** (`test.yml`, `lint.yml`, ...) has `workflow_call`, and also `workflow_dispatch` (and `schedule` where it
  has one), so it runs alone. A workflow that `ci` calls is inside the gate. A workflow with triggers of its own that `gate` does
  not wait for (`release`, `deploy`, `flagship`, the scheduled ones, `pr`, `issues`) stays outside it.
* **The planner** says which workflows have a planned job and passes each its slice as the input `plan`, a JSON array of job
  names. Which job belongs to which workflow is stated in the planner, as a partition of its job list.

### The `verdict` job

Every called workflow ends with a job named `verdict`:

* it runs `if: always()` and needs every job of its workflow;
* it runs the one judging script, `assert-gate.sh`, parameterized by the workflow's own job list and its `plan`: a planned job
  must be exactly `success`, an unplanned job must be `skipped`, a job it cannot see is a failure, and a plan that is not a JSON
  array of strings is a failure;
* only when the script succeeds does its last step set the output `verdict=pass`, which the workflow exposes.

The top-level `gate` requires, for each calling job the planner expects, **both** `needs.<job>.result == success` **and**
`needs.<job>.outputs.verdict == pass`. Both are in `toJSON(needs)`, which `gate` already reads.

The output is the point of the `+`. A `verdict` job that is itself skipped (a bug in its own `if`) leaves the callee `success`,
and without the output `gate` would pass it: the spike's design `a`, wrong on that case. A callee run alone is judged by its
own `verdict` job, which fails the workflow when its jobs were not as planned.

### The conventions, each with its evidence

1. **A callee routes by an input**, never by `github.event_name` or `github.workflow`: both are the caller's (F1, F2). `mode` and
   `plan` are inputs.
2. **No workflow-level `concurrency` in a called workflow.** The group is built from the caller's name, so it equals the
   caller's, GitHub reports a deadlock, and the run starts no job (F3).
3. **A callee asks for no permission its calling job does not grant.** If it does, the run does not start (`startup_failure`),
   and in `ci` no `gate` would report (F8).
4. **Every file keeps `permissions: {}` and a `permissions:` on each job**, pins its actions to a commit, and passes an input to
   a script through `env`, never into the script's text: the rules of `lint-workflow.sh`, which called workflows already satisfy
   with no change (F10).
5. **An `if` that cannot be evaluated skips its job without failing it** (F9), which is why a planned job is judged by a
   result that must be exactly `success`, and why the `verdict` job validates its own `plan`.
6. **A called workflow is a direct child of `.github/workflows/`**, and a contract cell is three levels deep (`ci`, `test`,
   `contract-cell`), well inside the limits.

### What this does not decide

Which jobs move to which file, in what order, and with what passes (the plan's Phase 1 and after); the replacement of the
planner by `triggerd`; and the Release, Deploy and PR workflows' own designs.

### Consequences

* Good, because `gate` stays one check with one name, so `deploy.yml`, the ruleset and `assert-sha-checks.sh` need no change.
* Good, because a planned job that never ran is red at every level: the spike's design was right on all seven scenarios, where
  the calling job's result alone was wrong on three.
* Good, because one script judges at every level, so there is no second copy of the rules to drift, and each concern's workflow
  is judged when it runs alone.
* Bad, because each called workflow carries one more job and its inputs wired. It is expected to cost about what `gate` does,
  a median of 10 runner-seconds ([research 0017](../../../ci-workflows/research/0017-ci-baseline.md)); the exact figure is
  measured when the first one lands.
* Bad, because nothing judges the judge of a callee run alone: if its `verdict` job is skipped it still reports `success`. There
  is no top-level `gate` there to require the output.
* Bad, because a called workflow's jobs, skipped ones included, appear in the check list as `<calling job> / <job>`, so the list
  grows by every job of every callee (F4).
* Neutral: `assert-gate.sh` gains a second kind of input (a job list and a plan, not the constant it holds now), and
  `plan-ci.py` a `workflows` output.

### Confirmation

Planned with this decision (Phase 0, stages 0.3 and 0.4 of the plan); each is in place when its stage merges:

* `test-ci-scripts.sh` generates a case for each of the spike's seven scenarios, including the control that a callee whose
  `verdict` output is missing fails the gate although its result is `success`; the hand mutants of the planner and the gate run
  as a step of `ci-scripts`.
* `test_plan_ci.py` lists the real `.github/workflows/` directory and fails for a workflow that falls into the planner's generic
  rule and is not on the short list of files that run no check.
* `lint-workflow.sh` gains rules for conventions 1 and 2 (a file with `workflow_call` sets no workflow-level `concurrency` and
  does not read `github.event_name`), each with a case that must fail.
* `actionlint` runs in CI at a pinned version and, inside the checkout, refuses a call that passes an undeclared input, omits a
  required one, or reads an undeclared output (F11); its control is a copy of a caller that breaks the contract.
* Conventions 3 and 6 need no check of ours: GitHub refuses a run that breaks them (it does not start), and the run's absence
  is what `gate` reports. Convention 4 is `lint-all-workflows.sh`, which runs today; convention 5 is the `verdict` job's own
  check of its `plan`.
* The spike itself is the evidence for the rest: five designs against seven scenarios and eleven facts, in two identical samples.

## Pros and Cons of the Options

### Option 1: a thin `ci.yml`, one workflow per concern, a `verdict` job in each

* Good, because the required check, its name and its consumers stay as they are.
* Good, because the proof that a planned job ran extends to every level, and was measured.
* Bad, because of the costs under Consequences.

### Option 2: a gate in every workflow, each a required check

* Good, because each workflow is self-contained.
* Bad, because `main`'s ruleset would require N checks, against ADR-0100's one aggregate context; every new workflow means a
  ruleset change; and `deploy.yml` and `assert-sha-checks.sh` would have to track N names.
* Bad, because a workflow that never starts reports nothing, so N required checks are N ways to block a pull request forever
  (the failure ADR-0100 names).

### Option 3: `pr.yml` as the orchestrator

* Good, because the owner's layout already names a PR workflow.
* Bad, because the orchestrator must also run on `merge_group`, push to `main` and the nightly schedule, so runs on `main` would
  be called PR; and `deploy.yml` would start from `workflow_run` of `PR`, changing a contract that works. PR keeps its own job:
  the pull-request chores that `gate` does not wait for.

### Option 4: independent workflows with `paths:` filters

* Good, because it is the least code.
* Bad, because a workflow skipped by a path filter reports no check, and a required check that never reports blocks a pull
  request forever, which is why the planner is a job and not a filter (`cicd.md`).

### Option 5: keep one `ci.yml`

* Good, because nothing changes.
* Bad, because it is the state the owner asked to leave: one 26 KB file for every concern.

### The judging designs (research 0018)

| Design | Wrong of 7 | |
|---|---|---|
| The calling job's result only (what `gate` reads today) | 3 | the control: it passes a skipped planned job, an unplanned job that ran, and a skipped judge |
| A `verdict` job, its output not required (`a`) | 1 | a skipped `verdict` leaves the callee green |
| **A `verdict` job and its output required (A+)** | **0** | chosen |
| The callee reports, `gate` judges (`b`) | 0 | a callee run alone is not judged: its own result stayed `success` |
| `re-actors/alls-green` v1.3.0 as the verdict | 2 | it cannot say an unplanned job must not run, nor judge a skipped judge, and it needs a hand-written step to say what may be skipped |

## Open Questions

None. The orchestrator, the name `verdict` and the judging design are decided.

## More Information

[The plan](../../../ci-workflows/PLAN.md) and [Phase 0](../../../ci-workflows/phases/0001-foundation.md);
[research 0017](../../../ci-workflows/research/0017-ci-baseline.md) (what `ci` costs today) and
[research 0018](../../../ci-workflows/research/0018-gate-spike.md) (the spike: the matrix, F1 to F11 with their runs, and the
runs themselves). How CI works today is in [CI/CD](../../../../../docs/src/architecture/cicd.md). Related:
[[ADR-0100]] (one aggregate required context), [[ADR-0101]] (supply chain), [[ADR-0110]] (the deploy starts from `ci`).
