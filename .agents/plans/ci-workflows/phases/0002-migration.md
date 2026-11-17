# Phase 0002: Migration

Part of [ci-workflows](../PLAN.md). Planned 2026-11-17; nothing built. Facts are from `main` at `f263ab4`, read on that day:
recount them when a stage starts. Phase 1 of the plan, after [Phase 0](0001-foundation.md) (the foundation: merged). It moves
today's jobs out of `ci.yml` into one workflow per concern as a **pure refactor**: the same commands, the same job boundaries,
the same result of `gate`. No pass applies to it; each stage's evidence is its acceptance criteria. The passes start in Phase 2,
on the migrated layout.

The plan's decisions are in [PLAN.md](../PLAN.md); the design that makes a called workflow judgeable is
[ADR-0102](../../contrail/adrs/ci/0102-one-workflow-per-concern-and-the-verdict-job.md), and what it rests on is
[research 0018](../research/0018-gate-spike.md). This document does not restate them.

## What "the same" means, and how each pull request proves it

A refactor is only a refactor if something shows it. Each stage's pull request carries:

1. **A parity table**: every step of every moved job, old place to new place, with its command byte for byte.
2. **A mechanical check of that table** (below), so that it is not only my reading.
3. **A full run and an affected-only run**, both green, running the same commands as before (a dispatch of `ci` is a full
   run: it has no base to diff).
4. **Two probes that make the real wiring fail**, as draft pull requests closed unmerged (like #196 to #199 and the spike):
   a planned job in the new workflow skipped by a bug in its `if`, and the new workflow's `verdict` job skipped. Each must turn
   the real `gate` red. This is invariant 3 of the plan, shown on the real files and not on a fixture.
5. The other invariants of the plan: `gate` stays the only required context, an `rc/*` pull request still plans a staging
   dispatch, the lint and the planner's tests pass, and every script of `deps` is run before the push.

**The mechanical check** is a script, `.agents/plans/ci-workflows/scripts/ci-commands.py`: it lists every step of every job of
every workflow (the job's id, the step's `uses:` and `with:` or its `run:` text, its `if:` and `env:`), and compares the list on
`main` with the list on the branch, treating a job as the same job wherever it lives. A step that disappeared, appeared or changed
is reported; a step that only moved is not. Its control is the reason to trust it: a copy of the branch with one step dropped,
and one with one command changed, must each be reported. *(Hand-written: nothing public compares the steps of two sets of
workflows; `actionlint` and `diff` on the YAML do not know that a job moved. The survey is repeated when the stage starts.)*

## What exists today (read 2026-11-17)

| Fact | Where | Consequence |
|---|---|---|
| `ci.yml` has `changes`, ten jobs and `gate`; each job has `if: contains(fromJSON(needs.changes.outputs.jobs ...), '<id>')`. `changes` outputs `jobs`, `matrix`, `cells` only | `ci.yml` | A stage adds the outputs `top` and `plans` (the planner already prints them, Phase 0) and replaces the jobs it moves with one calling job |
| `gate` downloads the `cell-ok-*` pass markers itself and runs `assert-gate.sh` with `EXPECTED_JOBS=jobs`, `PLANNED=cells`, over its default job list | `ci.yml` `gate` | A moved job leaves `gate`'s list and its calling workflow joins it. The marker rule stays at the top until `contract` moves |
| The gate's default `KNOWN_JOBS` is a constant in `assert-gate.sh`, and `same_jobs` in `test-ci-scripts.sh` holds it equal to the planner's `JOBS` | `assert-gate.sh`, `test-ci-scripts.sh` | After a stage the constant is the planner's top-level names (the jobs still in `ci.yml`, then the workflows): `same_jobs` must compare that, so it is part of every stage |
| `VERDICT_JOBS` names the called workflows, and a name in it must be a job the gate knows | `assert-gate.sh` | `ci.yml` sets it as a **static list**, not from the planner's `workflows` output: a planner bug that dropped a workflow from that output would otherwise switch off the check for it. A test holds the list equal to the keys of `WORKFLOWS` |
| `flake-hunt.yml` sets a workflow-level `concurrency:` and reads an empty `inputs` as "a scheduled run" | `flake-hunt.yml` | Folding it into `test.yml` breaks R8 (no workflow-level `concurrency:` in a called workflow) and sits on the edge of R9 (a callee routes by an input): see the questions |
| `e2e-schedule.yml` calls `contract-cell.yml` twice, has a workflow-level `concurrency:`, and runs on a pull request that edits it | `e2e-schedule.yml` | A rename to `e2e.yml`, not a callee: it has its own triggers and `gate` does not wait for it |
| A called workflow's jobs are `<calling job> / <job>` in the check list, and a matrix call is `<calling job> (<cell>) / <job>` | research 0018, F4 | Nothing requires a name but `gate`. `ci-baseline.py` classifies a job by what precedes ` / `, so its `--groups` changes with the layout, as research 0017 says |
| Renaming a job id (`docs`, `ci-scripts`) touches about 20 and 40 lines of the planner, the gate, the tests and the mutants, and about 20 lines of documentation, each | counted 2026-11-17 | A rename is mechanical, but it is not free: see the questions |

## The stages, and the order

The plan listed M1 to M6 in the order of the owner's priority groups, with `test.yml` first. This phase takes **the simplest
callee first**, because the first real called workflow is where whatever the spike could not show will show, and that is
cheapest where there is least to move.

| Stage | Workflow | Moves | Why here |
|---|---|---|---|
| M1 | `documentation.yml` | `docs` | One job. No matrix, no cell markers, no secrets, no Docker. It exercises every part of the wiring: `changes`' outputs, the calling job, the `verdict` job, `gate`'s `VERDICT_JOBS`, the planner's map, the index, the probes |
| M2 | `security.yml` | `deny` | The second callee, so the `verdict` job's steps are now **the same in two places**: the composite action is made here, by the plan's rule (see below) |
| M3 | `web.yml` | `web` | One job again, with its own environment (Node, pnpm, and two environment variables at the job level) |
| M4 | `lint.yml` | `lint`, `deps` | Two jobs, and `deps` has sixteen steps, the workflow guard and the two added in Phase 0 (the index guard, `actionlint`) among them: the first callee that holds more than one job, and the one that runs the guards of the files it is moving into |
| M5 | `test.yml` | `ci-scripts`, `hermetic`, `pg-adapter`, `contract`, `mutants` | The largest: the matrix call, the cell markers, and the `verdict` job that must judge them. Its own design, written when it starts |
| M6 | `e2e.yml` | `e2e-schedule.yml` renamed | Not a callee; the index and the planner's lists change |

Every stage edits `ci.yml`, so they run one after another, each starting from the `main` that holds the last.

### M1 in detail: `documentation.yml`

**Purpose.** Move the book's check out of `ci.yml`, and prove the pattern of ADR-0102 on the smallest real job.

**Acceptance.**
- `.github/workflows/documentation.yml` (`name: documentation`, the lowercase of the repository's other workflows): triggers
  `workflow_call` (input `plan`, a JSON array of the jobs to run, required; output `verdict`) and `workflow_dispatch` (input `plan`,
  default `["docs"]`, so that a run by hand runs the workflow). No workflow-level `concurrency:`, `permissions: {}`, each job its
  own. It holds the job `docs`, **verbatim** (the same steps, the same pins, the same comment), with
  `if: contains(fromJSON(inputs.plan), 'docs')`, and the `verdict` job of ADR-0102, written out, not yet an action.
- `ci.yml`: `docs` is gone; a calling job `documentation` (`needs: changes`, `if` on `top`, `with: plan:` from `plans`);
  `changes` outputs `top` and `plans`; `gate` needs `documentation` and sets `EXPECTED_JOBS` to `top`, `VERDICT_JOBS` to
  `documentation` and the default list to the new names.
- `plan-ci.py`: `WORKFLOWS = {"documentation": ("docs",)}`. `assert-gate.sh`'s default list, `same_jobs`, the index row, the
  `cicd.md` text, `ci-baseline.py`'s `--groups` and the mutant driver follow. The mutant driver is run: every mutant is still
  killed and none is broken, and a mutant of the new wiring (the gate without `VERDICT_JOBS`) is added.
- The book is built by the same commands: `scripts/check-book.sh` and `scripts/check-book.sh --self-test`, in that order.
- Proof, as above: the parity table and its check, a full run and an affected-only run (a change under `docs/` plans the
  `documentation` workflow and nothing else; a change to `deny.toml` plans no `documentation`), and the two probes.

**State machine of the job.** `docs` is planned, then runs, then succeeds (the workflow's `verdict` job judges it) or fails (the
`verdict` fails, `documentation` fails, `gate` fails); `docs` not planned is skipped, and its `verdict` expects that. The
`verdict` job is `if: always()`, so it reports when `docs` is skipped, failed or cancelled.

**Steps.** The workflow file and the `ci.yml` edit first; then the planner's map and the gate's list with their tests (red, then
green); then the index, the docs and the parity script; then the probes. Details of the job's steps are not repeated: they are
moved, not written.

**Public solutions.** None applies to a move. The `verdict` job's judge is `assert-gate.sh`, which research 0018 measured
against `re-actors/alls-green` and kept. The composite action for the `verdict` job is **not made in M1**, by the plan's rule:
with one callee its steps are not repeated anywhere, and an action drawn from one example guesses its inputs.

## The `verdict` action, by the plan's rule

M2 is where the same steps stand in two files, so M2 makes the composite action (checkout, judge, mark the output) and says so in
its pull request; M1's `verdict` job is then replaced by a call of it, in M2, so that the two callees are one change of the
same kind. The `verdict` job of `test.yml` (M5) is **not** an exact copy: it also downloads the cell markers and passes the
planned cells, so it is asked about when M5 starts, with the two side by side.

## Decisions on the questions (owner, 2026-11-17, after #251)

All four were answered as recommended.

| Question | Answer |
|---|---|
| The order | The simplest callee first: M1 `documentation.yml`, `test.yml` fifth. Your priority groups govern the passes of Phases 2 to 14; nothing in the migration depends on them, and `test.yml` is the stage most likely to find something the spike did not |
| The job ids | Kept through the migration (`docs`, `ci-scripts`, `lint`, `deny`, ...), and renamed in the phase that owns the workflow. A rename is about 60 lines of the planner and its tests and 20 to 40 of documentation per job, and `ci-scripts` is not one-to-one with `bash` (it also runs the planner's Python tests and the mutants) |
| `flake-hunt.yml` | Kept exactly as it is through the migration, and a stage of the Test phase (Phase 2), where `test.yml` is being reworked anyway. Folding it in needs a job-level `concurrency:` (R8 forbids the workflow-level one it has) and reads "no `mode` given" as "the weekly hunt" (R9), a refactor that bends two of its own rules for nothing the Test phase does not buy at less risk |
| `name:` of a workflow | Lowercase, the repository's convention (`documentation`, `security`, `test`): the name shows in every check and in `workflow_run`'s `workflows:` list |

No question is open.
