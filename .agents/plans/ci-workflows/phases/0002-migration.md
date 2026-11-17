# Phase 0002: Migration

Part of [ci-workflows](../PLAN.md). Planned 2026-11-17; M1 is built and merged (#254), the other stages are not. Facts are from
`main` at `f263ab4`, read on that day: recount them when a stage starts. Phase 1 of the plan, after [Phase 0](0001-foundation.md) (the foundation: merged). It moves
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

### M1: done (#254, merged 2026-11-17 as `8905e4c`)

What it showed, as the stage's own acceptance asked:
- **Parity**, by `scripts/ci-commands.py`: 34 jobs before, 36 after; 30 unchanged, `docs` moved with its 4 steps identical, 3
  changed as intended (`changes`, `gate`, `ci-scripts`), 2 appeared (`ci.yml:documentation`, `documentation.yml:verdict`), 0 gone.
- **The wiring**, by `test_workflow_wiring.py` (35 checks) and `mutate_gate.py` (76 mutants killed, the 4 that change nothing
  survived); a full run of 40 checks, `gate` certified, `documentation / docs` built the book.
- **Two probes** that made the real `gate` red, each for its own reason: #255 (`docs` planned and skipped) failed
  `documentation / verdict` and `gate`; #256 (the step that sets `verdict=pass` skipped) failed `gate` alone, with `documentation
  succeeded but its verdict output is 'missing', not 'pass'`.

What it found: the gate walks `needs` in the order `ci.yml` lists them, so a probe that also reddens `ci-scripts` hides its own
message (plant a fault the text tests cannot see); `actionlint` refuses a constant `if: false`; a skip of the `verdict` job itself
cannot be planted alone, because the wiring test checks `if: always()`, so a probe skips the step that sets the output instead.

**A correction to M1's pull request**, which said the book and `deny.toml` cases would be "the next PR's (M2) first real
affected-only run". They will not: every stage edits `ci.yml`, which the planner runs in full. The affected-only run is taken once,
after M6, as the migration's acceptance; until then those cases are `test_plan_ci.py`'s.

### M2 in detail: `security.yml`, and the `verdict` action

**Purpose.** Move the supply-chain check out of `ci.yml`, and, because the `verdict` job's steps now stand in two files, make them
a composite action, by the plan's rule: an exact duplicate in two places is made into an action without asking, and the pull
request says so.

**The duplicate, side by side.** The `verdict` job of `documentation.yml` (M1) and the one `security.yml` needs differ in one
value, the jobs the workflow knows (`docs`, `deny`); every other line is the same: the runner, the timeout, the permission, the
checkout, the judge step with its five variables, and the step that sets the output.

| | In the action | Stays in the job |
|---|---|---|
| Judge step: `assert-gate.sh` with `NEEDS_JSON`, `EXPECTED_JOBS`, `KNOWN_JOBS`, and `CHANGES_JOB`, `CELL_JOB` both set empty | yes | |
| The step that sets the output `verdict=pass`, after the judge | yes | |
| `actions/checkout` | | **yes**: a local action is read from the checkout, so the checkout comes first and an action cannot make it |
| `runs-on`, `timeout-minutes`, `permissions`, `needs`, `if: always()`, the job's `outputs` | | yes: they belong to the job (`.github/actions/README.md` says so) |

Inputs of the action: `needs` (`toJSON(needs)` of the calling job), `plan` (the workflow's `plan` input) and `jobs` (the jobs the
workflow knows); output: `verdict`. A variable the two copies share and the action fixes: `CHANGES_JOB` and `CELL_JOB` empty, since a
called workflow has neither a planner job nor cells. `test.yml`'s `verdict` job (M5) does have cells, so it is **not** an exact copy and is
asked about when M5 starts, with the two side by side; nothing here guesses its inputs.

**Acceptance.**
- `.github/actions/verdict/action.yml` (a row in `.github/actions/README.md`), and `documentation.yml`'s `verdict` job calls it: the
  same job, with the judge and mark steps replaced by one step.
- `.github/workflows/security.yml` (`name: security`): `workflow_call` (input `plan`, required; output `verdict`) and
  `workflow_dispatch` (`plan` defaults to `["deny"]`); no workflow-level `concurrency:`; `permissions: {}`; the job `deny` moved
  **verbatim** (steps, pins, comment) with `if: contains(fromJSON(inputs.plan), 'deny')` and without `needs: changes`; a `verdict`
  job that calls the action.
- `ci.yml`: `deny` is gone; a calling job `security`; `gate` needs it, and `VERDICT_JOBS` is `documentation security`.
- The planner: `WORKFLOWS` holds `security: (deny)`; a rule makes a change to the action run everything (the gate trusts it, like
  `assert-gate.sh`); the gate's default list, `same_jobs`, the indexes, `cicd.md`, `running.md`, `ci-baseline.py`'s groups and the mutant
  driver follow. A change to `deny.toml` plans the workflow `security` and nothing else; a change to a crate's `Cargo.toml` plans it
  too (cases of `test_plan_ci.py`).
- The wiring test checks the action once (its judge, its two fixed variables, its output set after the judge and only there) and
  each called workflow's `verdict` job (the call and its three inputs). The mutant driver gains mutants of the action and of
  `security.yml`; every mutant is killed, none is broken, the equivalent ones survive.
- Proof as for M1: the parity table and its check (`deny` moved, its steps identical; the `verdict` job of `documentation.yml` the one
  change to an existing file's job), a full run, and two probes planted in `security.yml`: `deny` planned and skipped, and the call
  of the action skipped. The second also shows that `documentation`, which shares the action, still passes.

**State machine of the job.** As `docs`: planned, then runs, then succeeds or fails; not planned, then skipped. The `verdict` job
is `if: always()`.

**Public solutions.** None applies to the move. The judge is `assert-gate.sh` (research 0018 measured `re-actors/alls-green` against
it). The job runs `cargo deny` as `scripts/rs cargo deny --locked check`, from `taiki-e/install-action`. Whether a public
cargo-deny action fits better is a question for the Security phase's survey (pass 1, not made here); a refactor does not swap it,
since that changes the command.

## Decisions on the questions (owner, 2026-11-17, after #251)

All four were answered as recommended.

| Question | Answer |
|---|---|
| The order | The simplest callee first: M1 `documentation.yml`, `test.yml` fifth. Your priority groups govern the passes of Phases 2 to 14; nothing in the migration depends on them, and `test.yml` is the stage most likely to find something the spike did not |
| The job ids | Kept through the migration (`docs`, `ci-scripts`, `lint`, `deny`, ...), and renamed in the phase that owns the workflow. A rename is about 60 lines of the planner and its tests and 20 to 40 of documentation per job, and `ci-scripts` is not one-to-one with `bash` (it also runs the planner's Python tests and the mutants) |
| `flake-hunt.yml` | Kept exactly as it is through the migration, and a stage of the Test phase (Phase 2), where `test.yml` is being reworked anyway. Folding it in needs a job-level `concurrency:` (R8 forbids the workflow-level one it has) and reads "no `mode` given" as "the weekly hunt" (R9), a refactor that bends two of its own rules for nothing the Test phase does not buy at less risk |
| `name:` of a workflow | Lowercase, the repository's convention (`documentation`, `security`, `test`): the name shows in every check and in `workflow_run`'s `workflows:` list |

No question is open.
