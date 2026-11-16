# Research 0018: how `gate` judges a called workflow (the spike)

Stage 0.2 of [phase 0001](../phases/0001-foundation.md), part of [ci-workflows](../PLAN.md). The spike's workflows were pull
request #232, a draft closed unmerged, at commits `6577e0b` (first sample) and `ba54b53` (second); they stay readable at
`refs/pull/232/head`. It ran on 2026-11-16 and was read through the API. Two samples of the whole matrix were identical.

## The result

**Choose design A+: every called workflow ends with a `verdict` job, and `gate` requires both the calling job's result and
the callee's `verdict` output.**

- The `verdict` job is `if: always()` and needs every inner job. It runs the one judging script over its own jobs and the plan
  it was given, and only if that succeeds does its last step set the output `verdict=pass`, which the workflow exposes.
- For each callee the planner expects, the top-level `gate` requires `needs.<callee>.result == success` **and**
  `needs.<callee>.outputs.verdict == pass`. Both are in `toJSON(needs)`, which `assert-gate.sh` already reads.
- It was right on all seven scenarios. Design `a` (the same without the output) was wrong on one, and the reason is the whole
  case for the `+`: if the `verdict` job is itself skipped, the callee reports `success` and nothing notices. Requiring the
  output closes that.
- Design `b` (the callee reports, `gate` flattens) was also right on all seven. It loses to A+ on one point: a callee run
  alone, by dispatch or schedule, is not judged. In `b`'s skipped-planned leg the callee's own result was `success`.
- `re-actors/alls-green` was wrong on two scenarios (below), so the hand-written judge stays. This replaces the plan's
  assumption that no public gate covers it with a measurement.

One limit stays with A+, and it is not a flaw of the design: a callee run alone whose `verdict` job is skipped still reports
`success`, because there is no top-level `gate` to require the output. Nothing can judge the judge there.

## The designs

| Design | What `gate` reads of a callee |
|---|---|
| `naive` | the calling job's result only: what it reads today. The **control**: it must be wrong where the hole is |
| `a` | the same; the callee has a `verdict` job that fails it when its jobs were not as planned |
| `aplus` | the calling job's result and the callee's `verdict` output being `pass` |
| `b` | the callee exports `toJSON(needs)`; `gate` judges it with the same rules |
| `allsgreen` | design `a` with `re-actors/alls-green` (v1.3.0, pinned) as the verdict job; `allowed-skips` is the jobs the plan leaves out |

## What each design decided

Each cell is what the top-level gate decided; **✗** marks a wrong decision (it should pass `green` and fail the rest).

| scenario | the bug it stands for | `naive` | `a` | `aplus` | `b` | `allsgreen` |
|---|---|---|---|---|---|---|
| `green` | none | pass | pass | pass | pass | pass |
| `skip-planned` | an `if` disagrees with the plan, so a planned job is skipped | pass ✗ | fail | fail | fail | fail |
| `fail-planned` | an ordinary failure | fail | fail | fail | fail | fail |
| `unplanned-ran` | an `if` disagrees the other way, so an unplanned job runs | pass ✗ | fail | fail | fail | pass ✗ |
| `callee-skipped` | the calling job's own `if` is wrong, so a planned workflow never runs | fail | fail | fail | fail | fail |
| `no-verdict` | the judging job is skipped by a bug in its own `if` | pass ✗ | pass ✗ | fail | fail | pass ✗ |
| `malformed-plan` | a planner bug: the plan is not JSON | fail | fail | fail | fail | fail |
| **wrong decisions** | | **3** | **1** | **0** | **0** | **2** |

- **`naive` is wrong three times**, so the control can fail and the hole is real. In its `skip-planned` leg the calling job
  was `success`: job `a` ran, job `b` was planned and skipped. That is the case the plan could not see.
- **`alls-green` cannot say that an unplanned job must not run**: a job that succeeded is fine to it whatever the plan says
  (`allowed-skips` only lets a job be skipped). It also has to be fed the list of skippable jobs, which a hand-written step
  computes (and validates), so it replaces no hand-written logic.
- The `callee-skipped` and `malformed-plan` rows are right for every design, `naive` included, because the existing rule that an
  expected job must be exactly `success` already catches a skipped calling job.

## The facts the plan assumed

| | Fact | Result | Run |
|---|---|---|---|
| F1 | `github.event_name` inside a callee is the caller's event | **yes**: `pull_request`, in a callee with no trigger but `workflow_call` | 43963043055 |
| F2 | `github.workflow` inside a callee is the caller's name | **yes**: `spike-facts`, not `spike-facts-callee` | 43963043055 |
| F3 | a workflow-level `concurrency` in a callee cancels or waits on its caller | **worse: the run does not start.** Zero jobs, conclusion `failure`: `deadlock was detected for concurrency group: 'spike-f3-refs/pull/232/merge' between a top level workflow and 'call'`. The group was built from the caller's name (F2) | 43963043155, 43964175324 |
| F4 | a callee's jobs are named `<calling job> / <job>` in the check list | **yes**, read from the check-runs API: `leg (aplus, green) / callee / verdict`. A top-level job keeps its bare name (`gate`), so a callee's `verdict` cannot collide with it. Skipped jobs of a callee are listed too | 43964175542 |
| F5 | a calling job's result is `success` when every job inside was skipped | **no.** It is `success` when at least one inner job ran and succeeded while a planned one was skipped (`naive`'s `skip-planned`); with a valid empty plan, so that every inner job is skipped, it is `skipped` | 43963043134, 43964175381 |
| F6 | a callee's workflow output reaches `needs.<caller>.outputs` | **yes** (`probe`, `verdict`, `results`). The size limit was not measured: A+'s output is the word `pass` | 43963043055 |
| F7 | an artifact uploaded in a callee can be downloaded by a later job of the caller | **yes** (the contract cells' markers keep working) | 43963043055 |
| F8 | a callee cannot hold a permission its calling job did not grant | **yes, and the whole run does not start**: `startup_failure`, `Invalid workflow file: nested job 'greedy' is requesting 'contents: write', but is only allowed 'contents: read'.` A run that does not start has no jobs, so in `ci` no `gate` would report (a consequence, not a run) | 43963043362, 43964175476 |
| F9 | (found) an `if` that cannot be evaluated | **skips the job, silently**: `fromJSON` of a non-JSON plan skipped every inner job and the calling job was `skipped`; the failure came from the verdict job's own check of the plan | 43963043292 |
| F10 | (found) the pattern at the depth `ci` → `test` → `contract-cell` | **works**: a matrix caller → a leg workflow → a callee. The repository's own lint and helper tests accepted every spike workflow with no change (`ci` run 43963043146: `deps` and `ci-scripts` passed) | 43963043134 |
| F11 | (found) `actionlint` on the call contracts | **refuses** an undeclared input, a missing required input and a read of an undeclared output, and knows the callee's output types. **Only inside a git repository**: in a plain directory it checked nothing and printed nothing. The spike's own two reads of a declared output raised no false positive (#407 did not appear) | local, `actionlint` in `/opt/homebrew/bin` |

The REST API does not expose the messages of F3 and F8; they are on the run page, which is where they were read.

## What this changes in the plan

Each is made in the pull request that adds this document.

- **The plan's F5 claim was wrong** and is corrected in `PLAN.md` and in the phase document (use case 3, and the table of what
  exists). The hole is real but narrower than written: it needs a callee with two or more planned jobs, one of them skipped.
- **Stage 0.3 is specified**: the gate requires `result == success` and `outputs.verdict == pass` of each expected callee; the
  judging script is the one `assert-gate.sh`, taking its job list and plan as inputs; the callee's verdict calls it.
- **Stage 0.1 (ADR-0102) records the conventions**, now with their evidence: route by input (F1); no workflow-level
  `concurrency` in a callee, because it deadlocks the run (F3); a callee asks for no permission its caller does not grant, or the
  run never starts (F8); and a callee's `verdict` job must itself be required (the `+`).
- **Stage 0.4**: `actionlint` runs inside the checkout (F11), and its control is a copy of a caller that breaks the contract.
- **An `if` that errors skips quietly (F9)**, so every planned job is judged by a result that must be exactly `success`.
- **The inventory gains `flagship.yml`** (#229): a deploy-adjacent workflow with its own environment and secrets that nothing
  in `gate` waits for. It belongs to the Deploy phase and goes on the list of files that run no check.
- **The test-suites plan** ([`../../test-suites/PLAN.md`](../../test-suites/PLAN.md), #234) hands its workflows to phases 2, 12,
  13 and 14 of this one, and its pull requests edit commands inside `ci.yml` jobs that Phase 1 moves: whichever lands second
  takes the other's change.

## Method

- One caller (`spike-gate.yml`) ran five designs against six scenarios through a leg workflow that called the callee and then
  asserted what the gate would decide, so a design that mis-judged showed as a red `top` job. `spike-malformed.yml` ran the
  seventh scenario apart, because a plan that is not JSON might have stopped the whole run (it did not, F9). Each fact had a
  run of its own, so that a deadlock (F3) or a startup failure (F8) could not void the others.
- The workflows ran from a pull request because `workflow_dispatch` needs the file on the default branch (GitHub's docs).
- The judge and the top-level decision are two shell scripts (`spike-judge.sh`, `spike-top.sh`), tried under `dash` on 14
  synthetic cases before the first push. The real script is rewritten in stage 0.3, not copied.
- Not measured: the size limit of an output (F6), and a `verdict` skipped in a callee run alone (no top-level gate there).

## The runs

| | First sample (`6577e0b`) | Second sample (`ba54b53`) |
|---|---|---|
| `spike-gate` (30 legs) | 43963043134 | 43964175542 |
| `spike-malformed` (5 legs) | 43963043292 | 43964175480 |
| `spike-facts` (F1, F2, F6, F7) | 43963043055 | 43964175410 |
| `spike-f3` (F3) | 43963043155 | 43964175324 |
| `spike-f8` (F8) | 43963043362 | 43964175476 |
| `spike-f5` (F5, the empty plan) | added in the second commit | 43964175381 |
| `ci` | 43963043146 | 43964175562 |

## Every leg, as the runs reported it (first sample; the second is identical)

`a b c` are the inner jobs' conclusions (`ok` success, `FAIL` failure, `-` skipped, `?` never created), `judge` is the verdict
or report job, `top` is the job that asserts the gate's decision, which succeeds exactly when that decision was the right one.

### `spike-gate`

| scenario | design | a b c | judge | top | gate decided | right? |
|---|---|---|---|---|---|---|
| green | naive | ok ok - | none | ok | pass | yes |
| green | a | ok ok - | ok | ok | pass | yes |
| green | aplus | ok ok - | ok | ok | pass | yes |
| green | b | ok ok - | ok | ok | pass | yes |
| green | allsgreen | ok ok - | ok | ok | pass | yes |
| skip-planned | naive | ok - - | none | FAIL | pass | **NO** |
| skip-planned | a | ok - - | FAIL | ok | fail | yes |
| skip-planned | aplus | ok - - | FAIL | ok | fail | yes |
| skip-planned | b | ok - - | ok | ok | fail | yes |
| skip-planned | allsgreen | ok - - | FAIL | ok | fail | yes |
| fail-planned | naive | ok FAIL - | none | ok | fail | yes |
| fail-planned | a | ok FAIL - | FAIL | ok | fail | yes |
| fail-planned | aplus | ok FAIL - | FAIL | ok | fail | yes |
| fail-planned | b | ok FAIL - | ok | ok | fail | yes |
| fail-planned | allsgreen | ok FAIL - | FAIL | ok | fail | yes |
| unplanned-ran | naive | ok ok ok | none | FAIL | pass | **NO** |
| unplanned-ran | a | ok ok ok | FAIL | ok | fail | yes |
| unplanned-ran | aplus | ok ok ok | FAIL | ok | fail | yes |
| unplanned-ran | b | ok ok ok | ok | ok | fail | yes |
| unplanned-ran | allsgreen | ok ok ok | ok | FAIL | pass | **NO** |
| callee-skipped | naive | ? ? ? | none | ok | fail | yes |
| callee-skipped | a | ? ? ? | ? | ok | fail | yes |
| callee-skipped | aplus | ? ? ? | ? | ok | fail | yes |
| callee-skipped | b | ? ? ? | ? | ok | fail | yes |
| callee-skipped | allsgreen | ? ? ? | ? | ok | fail | yes |
| no-verdict | naive | ok ok - | none | FAIL | pass | **NO** |
| no-verdict | a | ok ok - | - | FAIL | pass | **NO** |
| no-verdict | aplus | ok ok - | - | ok | fail | yes |
| no-verdict | b | ok ok - | - | ok | fail | yes |
| no-verdict | allsgreen | ok ok - | - | FAIL | pass | **NO** |

legs=30 wrong=6

### `spike-malformed`

| scenario | design | a b c | judge | top | gate decided | right? |
|---|---|---|---|---|---|---|
| malformed-plan | naive | - - - | none | ok | fail | yes |
| malformed-plan | a | - - - | FAIL | ok | fail | yes |
| malformed-plan | aplus | - - - | FAIL | ok | fail | yes |
| malformed-plan | b | - - - | ok | ok | fail | yes |
| malformed-plan | allsgreen | - - - | FAIL | ok | fail | yes |

legs=5 wrong=0
